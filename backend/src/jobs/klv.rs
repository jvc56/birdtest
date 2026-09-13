//! Builds a MAGPIE KLV2 file directly, in Rust.
//!
//! This is birdtest's own implementation of what `magpie convert csv2klv`
//! does, so leave-generation aggregation never has to shell out to a MAGPIE
//! binary. It's not a guess at the format: every algorithm here is a direct
//! translation of MAGPIE's own (`src/ent/klv.h`, `src/ent/klv_csv.c`) --
//! variable names and comments below point at their C originals.
//!
//! ## Why this is safe to reimplement rather than merely convenient
//!
//! A KLV2 file is a KWG (a DAWG -- a trie with shared suffixes) of every
//! leave MAGPIE considers, immediately followed by one `f32` equity value per
//! leave, addressed by a **word index**: a leave's rank among all leaves,
//! counted via per-node subtree sizes (`word_counts` in MAGPIE, `counts`
//! here) computed fresh every time a KLV is loaded -- never stored in the
//! file. That index is a pure function of the graph's topology, not of *how*
//! the graph was built or in what order its siblings happen to be laid out.
//! So this module does not need to reproduce MAGPIE's own DAWG-minimizing
//! builder (`kwg_maker.c`'s suffix-sharing algorithm) or its specific sibling
//! ordering -- it needs only to emit a **topologically correct** trie in the
//! right node byte-format, encoding exactly the same leaves. A real MAGPIE
//! loading the result computes the same indices for the same leaves no
//! matter which of the two built the file, because both run the identical
//! counting algorithm over whatever graph is actually on disk.
//!
//! This module builds a plain trie (no suffix sharing -- MAGPIE's own
//! `KWG_MAKER_MERGE_NONE`, a supported, simpler variant of the same format)
//! rather than a minimized DAWG. The leave domain is small enough (at most a
//! few tens of thousands of nodes even for six-tile English leaves) that the
//! size difference from not sharing suffixes doesn't matter, and it removes
//! an entire nontrivial algorithm (DAWG minimization) from what has to be
//! reimplemented correctly.

use super::racks::LetterDistribution;
use crate::error::{AppError, AppResult};
use std::collections::{HashMap, VecDeque};

/// MAGPIE's `RACK_SIZE`, fixed at 7 across this whole system (both MAGPIE's
/// own build and birdtest's schema assume it). Leave generation tracks full
/// racks of exactly this many tiles, as MAGPIE's `RackList` does.
pub const RACK_SIZE: usize = 7;

/// MAGPIE's `RACK_SIZE - 1`. A KLV's leave domain is always every leave of
/// 1..=6 tiles; `klv_write_to_csv`/`klv_create_empty` in MAGPIE hardcode this
/// same bound, and matching it is what keeps a birdtest-built KLV
/// interchangeable with one MAGPIE would have built for the same data.
pub const MAX_LEAVE_SIZE: usize = RACK_SIZE - 1;

// --- KWG node bit layout (src/def/kwg_defs.h) -------------------------------
// tile:8 (bits 24-31) | accepts:1 (bit 23) | is_end:1 (bit 22) | arc_index:22 (bits 0-21)
const NODE_ACCEPTS_FLAG: u32 = 0x800000;
const NODE_IS_END_FLAG: u32 = 0x400000;
const NODE_ARC_INDEX_MASK: u32 = 0x3FFFFF;
const NODE_TILE_BIT_OFFSET: u32 = 24;

// --- Equity (src/ent/equity.h): a fixed-point i32, scaled by 1000, stored in
// the file as the equivalent f32 (not the raw scaled integer). Mirrored here
// so a birdtest-built value round-trips through the exact same precision a
// MAGPIE-built one would.
const EQUITY_RESOLUTION: f64 = 1000.0;
const EQUITY_MIN_VALUE: i32 = i32::MIN + 3;
const EQUITY_MAX_VALUE: i32 = -EQUITY_MIN_VALUE;

fn mean_to_equity_f32(mean: f64) -> f32 {
    let scaled = (mean * EQUITY_RESOLUTION).round();
    let clamped = scaled.clamp(EQUITY_MIN_VALUE as f64, EQUITY_MAX_VALUE as f64) as i32;
    (clamped as f64 / EQUITY_RESOLUTION) as f32
}

/// One node of the trie being built, before it's laid out into the flat
/// array a KWG actually is. `children` holds arena indices; order among
/// siblings is arbitrary (see the module doc) and is simply insertion order
/// here.
struct TrieNode {
    tile: u8,
    accepts: bool,
    children: Vec<usize>,
}

const ARENA_ROOT: usize = 0;

fn insert_leave(arena: &mut Vec<TrieNode>, letters: &[u8]) {
    let mut cur = ARENA_ROOT;
    for &ml in letters {
        let existing = arena[cur].children.iter().copied().find(|&c| arena[c].tile == ml);
        cur = match existing {
            Some(c) => c,
            None => {
                let idx = arena.len();
                arena.push(TrieNode { tile: ml, accepts: false, children: Vec::new() });
                arena[cur].children.push(idx);
                idx
            }
        };
    }
    arena[cur].accepts = true;
}

/// Lays the arena out as a flat KWG node array: every sibling group occupies
/// a contiguous run, `is_end` marks the last node in each run, and each
/// node's `arc_index` points at its own children's run (0 if it has none).
/// Mirrors `serialize_states_to_kwg` (`kwg_maker.c`) enough to produce a
/// valid KWG, without that function's suffix-sharing.
///
/// Returns the node array (including the two header slots at 0 and 1 --
/// `kwg_nodes[0] = dawg_root | IS_END`, `kwg_nodes[1] = 0 | IS_END` for "no
/// GADDAG", exactly as MAGPIE writes them) and the DAWG root node index
/// (`kwg_get_dawg_root_node_index`'s result -- the array index a leave
/// lookup actually starts scanning from, i.e. node 0's arc_index).
fn flatten(arena: &[TrieNode]) -> AppResult<(Vec<u32>, u32)> {
    let root_children = arena[ARENA_ROOT].children.clone();
    if root_children.is_empty() {
        return Err(AppError::internal(
            "letter distribution enumerates no leaves at all",
        ));
    }

    let mut nodes: Vec<u32> = vec![0, 0]; // patched below
    // Keyed by the arena node that OWNS this group of children (None = the
    // virtual root, whose "children" are the top-level first-letters).
    let mut group_start: HashMap<Option<usize>, usize> = HashMap::new();
    let mut output_index: HashMap<usize, usize> = HashMap::new();
    let mut queue: VecDeque<(Option<usize>, Vec<usize>)> = VecDeque::new();
    queue.push_back((None, root_children));

    while let Some((owner, group)) = queue.pop_front() {
        let start = nodes.len();
        group_start.insert(owner, start);
        for (i, &idx) in group.iter().enumerate() {
            let node = &arena[idx];
            let is_end = i == group.len() - 1;
            let packed = ((node.tile as u32) << NODE_TILE_BIT_OFFSET)
                | if node.accepts { NODE_ACCEPTS_FLAG } else { 0 }
                | if is_end { NODE_IS_END_FLAG } else { 0 };
            nodes.push(packed);
            output_index.insert(idx, start + i);
        }
        for &idx in &group {
            if !arena[idx].children.is_empty() {
                queue.push_back((Some(idx), arena[idx].children.clone()));
            }
        }
    }

    // Patch each node's arc_index now that its children's block (if any) has
    // a known start. Low 22 bits were left zero above, so OR-ing in is safe.
    for (owner, start) in &group_start {
        if let Some(arena_idx) = owner {
            let out_idx = output_index[arena_idx];
            nodes[out_idx] |= *start as u32;
        }
    }

    let dawg_root = *group_start.get(&None).expect("root group was just inserted above") as u32;
    nodes[0] = dawg_root | NODE_IS_END_FLAG;
    nodes[1] = NODE_IS_END_FLAG; // arc_index 0: no GADDAG in this KWG
    Ok((nodes, dawg_root))
}

/// Per-node subtree word counts (`klv_count_words_at`/`klv_count_words`),
/// computed bottom-up. Correct in one pass because `flatten` above always
/// places a node's children and later siblings at higher array indices than
/// the node itself, so both are already computed by the time this reaches
/// index `i`.
fn compute_counts(nodes: &[u32]) -> Vec<u32> {
    let mut counts = vec![0u32; nodes.len()];
    for i in (0..nodes.len()).rev() {
        let node = nodes[i];
        let mut c = if node & NODE_ACCEPTS_FLAG != 0 { 1 } else { 0 };
        let arc = node & NODE_ARC_INDEX_MASK;
        if arc != 0 {
            c += counts[arc as usize];
        }
        if node & NODE_IS_END_FLAG == 0 {
            c += counts[i + 1];
        }
        counts[i] = c;
    }
    counts
}

/// A leave's word index: the number of other leaves that sort before it in
/// this graph. Mirrors `klv_get_word_index_internal`/`increment_node_to_ml`/
/// `follow_arc` exactly (see the module doc for why an exact algorithmic
/// match, rather than an exact topology match, is what correctness actually
/// requires here). `letters` must be a leave this trie was actually built
/// from -- looking up anything else is a construction bug, not a normal
/// "not found" case, so this panics rather than returning an `Option`.
fn word_index_for(nodes: &[u32], counts: &[u32], root: u32, letters: &[u8]) -> u32 {
    let mut idx: u32 = 0;
    let mut node_index = root;
    for (i, &ml) in letters.iter().enumerate() {
        loop {
            let node = nodes[node_index as usize];
            let tile = (node >> NODE_TILE_BIT_OFFSET) as u8;
            if tile == ml {
                break;
            }
            assert!(
                node & NODE_IS_END_FLAG == 0,
                "leave {letters:?} not found in its own trie -- construction bug"
            );
            idx += counts[node_index as usize] - counts[node_index as usize + 1];
            node_index += 1;
        }
        if i == letters.len() - 1 {
            return idx;
        }
        // follow_arc: descending past a matched node always adds one, since
        // every prefix of a leave is independently enumerated as its own
        // (shorter) leave and so always accepts in this particular trie.
        idx += 1;
        node_index = nodes[node_index as usize] & NODE_ARC_INDEX_MASK;
    }
    unreachable!("letters is never empty")
}

fn serialize(nodes: &[u32], leave_values: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + nodes.len() * 4 + 4 + leave_values.len() * 4);
    out.extend_from_slice(&(nodes.len() as u32).to_le_bytes());
    for n in nodes {
        out.extend_from_slice(&n.to_le_bytes());
    }
    out.extend_from_slice(&(leave_values.len() as u32).to_le_bytes());
    for v in leave_values {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// The leave domain laid out as a KWG: the node array, and each enumerated
/// leave's word index (`word_indices[i]` is the index of `leaves[i]`).
struct Layout {
    nodes: Vec<u32>,
    leaves: Vec<String>,
    word_indices: Vec<u32>,
}

fn layout(distribution: &LetterDistribution) -> AppResult<Layout> {
    let leaves = distribution.enumerate_leaves(MAX_LEAVE_SIZE);

    let mut arena: Vec<TrieNode> = vec![TrieNode { tile: 0, accepts: false, children: Vec::new() }];
    let mut leaf_letters: Vec<Vec<u8>> = Vec::with_capacity(leaves.len());
    for rack in &leaves {
        let letters: Vec<u8> = rack
            .chars()
            .map(|c| {
                distribution.machine_letter(c).ok_or_else(|| {
                    AppError::internal(format!(
                        "letter {c:?} in enumerated leave {rack:?} is not in the distribution"
                    ))
                })
            })
            .collect::<AppResult<_>>()?;
        insert_leave(&mut arena, &letters);
        leaf_letters.push(letters);
    }

    let (nodes, root) = flatten(&arena)?;
    let counts = compute_counts(&nodes);
    let word_indices = leaf_letters
        .iter()
        .map(|letters| word_index_for(&nodes, &counts, root, letters))
        .collect();
    Ok(Layout { nodes, leaves, word_indices })
}

/// Builds a complete KLV2 file: every leave of 1..=[`MAX_LEAVE_SIZE`] tiles
/// drawable from `distribution`, valued from `value_by_leave` where present
/// and zero everywhere else -- exactly `magpie convert csv2klv`'s behavior for
/// a leaves CSV that doesn't mention every leave. Leave generation itself goes
/// through [`FullRackLeaves`]; this is the zeroed generation-0 KLV (an empty
/// map) and the format tests.
pub fn build(
    distribution: &LetterDistribution,
    value_by_leave: &HashMap<String, f64>,
) -> AppResult<Vec<u8>> {
    let layout = layout(distribution)?;
    let mut leave_values = vec![0.0f32; layout.leaves.len()];
    for (leave, &index) in layout.leaves.iter().zip(layout.word_indices.iter()) {
        let value = value_by_leave.get(leave).copied().unwrap_or(0.0);
        leave_values[index as usize] = mean_to_equity_f32(value);
    }
    Ok(serialize(&layout.nodes, &leave_values))
}

/// A leave packed into an integer: the tile index (in `distribution.tiles`
/// order, plus one) of each letter, six bits per letter. Distinct leaves get
/// distinct keys because letters are always packed in ascending tile order.
fn pack_letter(key: u64, position: usize, tile: usize) -> u64 {
    key | ((tile as u64 + 1) << (6 * position))
}

/// A multiply-mix hasher for [`pack_letter`] keys. Deriving a generation's
/// leaves looks up hundreds of millions of these, where the standard
/// SipHash's DoS resistance buys nothing (every key is server-generated) and
/// costs several times the time.
#[derive(Default)]
struct PackedLeaveHasher(u64);

impl std::hash::Hasher for PackedLeaveHasher {
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u64(self.0 ^ b as u64);
        }
    }
    fn write_u64(&mut self, x: u64) {
        let mixed = (x ^ (x >> 29)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        self.0 = mixed ^ (mixed >> 32);
    }
    fn finish(&self) -> u64 {
        self.0
    }
}

type PackedLeaveMap = HashMap<u64, u32, std::hash::BuildHasherDefault<PackedLeaveHasher>>;

/// Derives a generation's leave values from full-rack results, and builds the
/// KLV from them: a port of MAGPIE's `rack_list_write_to_klv` and
/// `generate_leaves` (`src/impl/rack_list.c`).
///
/// MAGPIE's leave generation observes full 7-tile racks, never leaves. Each
/// full rack `R` has a mean equity `m(R)` (0 if it never occurred) and a
/// weight `combos(R)`, the number of ways to draw it from a full bag:
/// the product over letters of `C(dist[l], R[l])`. Then:
///
/// - `average` is the combos-weighted mean of `m(R)` over every full rack.
/// - Every proper, non-empty sub-multiset `L` of `R` receives `m(R)` with
///   weight `C(dist[l] - L[l], R[l] - L[l])` multiplied over letters: the ways
///   to draw the rest of `R` once `L` is held.
/// - A leave's value is its weighted mean minus `average`, or 0 if no rack
///   contributed to it.
///
/// Racks are fed one at a time ([`Self::add_rack`]) so a generation's millions
/// of rows can be streamed rather than held in memory.
pub struct FullRackLeaves {
    layout: Layout,
    tile_by_letter: HashMap<char, usize>,
    tile_counts: Vec<u32>,
    /// `binomial[n][k]` for every `n` up to the largest tile count and `k` up
    /// to [`RACK_SIZE`].
    binomial: Vec<[u64; RACK_SIZE + 1]>,
    word_index_by_key: PackedLeaveMap,
    equity_sum: Vec<f64>,
    count_sum: Vec<u64>,
    weighted_sum: f64,
    combos_sum: u64,
    racks_added: u64,
}

impl FullRackLeaves {
    pub fn new(distribution: &LetterDistribution) -> AppResult<Self> {
        if distribution.tiles.len() > 63 {
            return Err(AppError::internal(format!(
                "a distribution of {} letters is too large to pack leaves for",
                distribution.tiles.len()
            )));
        }
        let layout = layout(distribution)?;
        let tile_by_letter: HashMap<char, usize> =
            distribution.tiles.iter().enumerate().map(|(i, t)| (t.letter, i)).collect();
        let tile_counts: Vec<u32> = distribution.tiles.iter().map(|t| t.count).collect();

        let max_count = tile_counts.iter().copied().max().unwrap_or(0) as usize;
        let mut binomial = vec![[0u64; RACK_SIZE + 1]; max_count + 1];
        for n in 0..=max_count {
            binomial[n][0] = 1;
            for k in 1..=RACK_SIZE.min(n) {
                binomial[n][k] = binomial[n - 1][k - 1] + binomial[n - 1][k];
            }
        }

        let mut word_index_by_key =
            PackedLeaveMap::with_capacity_and_hasher(layout.leaves.len(), Default::default());
        for (leave, &index) in layout.leaves.iter().zip(layout.word_indices.iter()) {
            let mut key = 0;
            for (position, letter) in leave.chars().enumerate() {
                key = pack_letter(key, position, tile_by_letter[&letter]);
            }
            word_index_by_key.insert(key, index);
        }

        let number_of_leaves = layout.leaves.len();
        Ok(Self {
            layout,
            tile_by_letter,
            tile_counts,
            binomial,
            word_index_by_key,
            equity_sum: vec![0.0; number_of_leaves],
            count_sum: vec![0; number_of_leaves],
            weighted_sum: 0.0,
            combos_sum: 0,
            racks_added: 0,
        })
    }

    /// Adds one full rack's results. `mean` is 0 for a rack that never
    /// occurred, as in MAGPIE, where such a rack still counts toward the
    /// average.
    pub fn add_rack(&mut self, rack: &str, mean: f64) -> AppResult<()> {
        // (tile index, how many of it), in ascending tile order.
        let mut groups: Vec<(usize, u32)> = Vec::with_capacity(RACK_SIZE);
        let mut tiles = 0;
        let mut letters: Vec<usize> = Vec::with_capacity(RACK_SIZE);
        for letter in rack.chars() {
            let tile = *self.tile_by_letter.get(&letter).ok_or_else(|| {
                AppError::internal(format!("rack {rack:?} has a letter {letter:?} not in the distribution"))
            })?;
            letters.push(tile);
            tiles += 1;
        }
        if tiles != RACK_SIZE {
            return Err(AppError::internal(format!(
                "rack {rack:?} has {tiles} tiles; leave generation tracks full racks of {RACK_SIZE}"
            )));
        }
        letters.sort_unstable();
        for tile in letters {
            match groups.last_mut() {
                Some((last, n)) if *last == tile => *n += 1,
                _ => groups.push((tile, 1)),
            }
        }

        let mut combos: u64 = 1;
        for &(tile, n) in &groups {
            let available = self.tile_counts[tile];
            if n > available {
                return Err(AppError::internal(format!(
                    "rack {rack:?} holds more of a letter than the bag has"
                )));
            }
            combos *= self.binomial[available as usize][n as usize];
        }
        self.weighted_sum += mean * combos as f64;
        self.combos_sum += combos;
        self.racks_added += 1;

        self.generate_leaves(&groups, 0, 0, 0, 1, mean);
        Ok(())
    }

    /// `generate_leaves`: every sub-multiset of `groups`, choosing how many of
    /// each letter the leave keeps. `count` accumulates the ways to draw what
    /// the leave does not keep.
    fn generate_leaves(
        &mut self,
        groups: &[(usize, u32)],
        group: usize,
        key: u64,
        leave_size: usize,
        count: u64,
        mean: f64,
    ) {
        if group == groups.len() {
            if leave_size > 0 && leave_size < RACK_SIZE {
                let index = self.word_index_by_key[&key] as usize;
                self.count_sum[index] += count;
                self.equity_sum[index] += mean * count as f64;
            }
            return;
        }
        let (tile, n) = groups[group];
        let available = self.tile_counts[tile];
        let mut key = key;
        for kept in 0..=n {
            if kept > 0 {
                key = pack_letter(key, leave_size + kept as usize - 1, tile);
            }
            let ways = self.binomial[(available - kept) as usize][(n - kept) as usize];
            self.generate_leaves(groups, group + 1, key, leave_size + kept as usize, count * ways, mean);
        }
    }

    /// How many racks have been added; a complete generation adds every full
    /// rack the distribution can draw.
    pub fn racks_added(&self) -> u64 {
        self.racks_added
    }

    /// Each leave's value in enumeration order, keyed like [`build`]'s map.
    fn leave_values(&self) -> impl Iterator<Item = (&str, u32, f64)> + '_ {
        let average =
            if self.combos_sum > 0 { self.weighted_sum / self.combos_sum as f64 } else { 0.0 };
        self.layout.leaves.iter().zip(self.layout.word_indices.iter()).map(move |(leave, &index)| {
            let i = index as usize;
            let value = if self.count_sum[i] > 0 {
                self.equity_sum[i] / self.count_sum[i] as f64 - average
            } else {
                0.0
            };
            (leave.as_str(), index, value)
        })
    }

    /// The KLV2 file for the racks added so far.
    pub fn build(&self) -> Vec<u8> {
        let mut leave_values = vec![0.0f32; self.layout.leaves.len()];
        for (_, index, value) in self.leave_values() {
            leave_values[index as usize] = mean_to_equity_f32(value);
        }
        serialize(&self.layout.nodes, &leave_values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::racks::Tile;

    fn tiny() -> LetterDistribution {
        // Deliberately not alphabetical-by-count, and more than one tile per
        // letter, to exercise real branching/merging paths.
        LetterDistribution::from_tiles_for_test(vec![
            Tile { letter: '?', count: 2 },
            Tile { letter: 'A', count: 3 },
            Tile { letter: 'B', count: 2 },
            Tile { letter: 'C', count: 1 },
        ])
    }

    /// Every leave the domain enumerates must get its own distinct word
    /// index -- a collision would mean two leaves silently share one stored
    /// value, and a gap would mean `leave_values` is the wrong length for
    /// what `klv_load` will compute when a real MAGPIE reads the file back.
    #[test]
    fn word_indices_are_a_bijection_onto_0_n() {
        let dist = tiny();
        let leaves = dist.enumerate_leaves(MAX_LEAVE_SIZE);
        let mut arena: Vec<TrieNode> = vec![TrieNode { tile: 0, accepts: false, children: Vec::new() }];
        let mut letters_by_leave = Vec::new();
        for rack in &leaves {
            let letters: Vec<u8> =
                rack.chars().map(|c| dist.machine_letter(c).unwrap()).collect();
            insert_leave(&mut arena, &letters);
            letters_by_leave.push(letters);
        }
        let (nodes, root) = flatten(&arena).unwrap();
        let counts = compute_counts(&nodes);

        let mut seen = vec![false; leaves.len()];
        for letters in &letters_by_leave {
            let idx = word_index_for(&nodes, &counts, root, letters) as usize;
            assert!(idx < leaves.len(), "index {idx} out of range for {} leaves", leaves.len());
            assert!(!seen[idx], "duplicate word index {idx}");
            seen[idx] = true;
        }
        assert!(seen.iter().all(|&s| s), "every index in 0..n must be used");
    }

    #[test]
    fn build_produces_the_documented_binary_layout() {
        let dist = tiny();
        let mut mean_by_rack = HashMap::new();
        mean_by_rack.insert("A".to_string(), 12.5);
        mean_by_rack.insert("AB".to_string(), -3.25);

        let bytes = build(&dist, &mean_by_rack).unwrap();
        let leaves = dist.enumerate_leaves(MAX_LEAVE_SIZE);

        let kwg_size = u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;
        let after_nodes = 4 + kwg_size * 4;
        let number_of_leaves =
            u32::from_le_bytes(bytes[after_nodes..after_nodes + 4].try_into().unwrap()) as usize;
        assert_eq!(number_of_leaves, leaves.len());
        assert_eq!(
            bytes.len(),
            4 + kwg_size * 4 + 4 + number_of_leaves * 4,
            "file length must match the header-declared sizes exactly"
        );
    }

    #[test]
    fn missing_leaves_are_zero_valued() {
        let dist = tiny();
        // No data at all: every leave must come back as exactly 0.0, matching
        // klv_create_zeroed_from_kwg's zero-init for anything the CSV never set.
        let bytes = build(&dist, &HashMap::new()).unwrap();
        let kwg_size = u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;
        let values_start = 4 + kwg_size * 4 + 4;
        let (values, _) = bytes[values_start..].as_chunks::<4>();
        for chunk in values {
            let v = f32::from_le_bytes(*chunk);
            assert_eq!(v, 0.0);
        }
    }

    /// A direct, slow statement of `rack_list_write_to_klv`'s definition,
    /// independent of the recursive enumeration and packed keys above: for
    /// every leave, scan every full rack containing it.
    fn reference_leave_values(
        dist: &LetterDistribution,
        mean_by_rack: &HashMap<String, f64>,
    ) -> HashMap<String, f64> {
        fn counts(dist: &LetterDistribution, s: &str) -> Vec<u32> {
            dist.tiles
                .iter()
                .map(|t| s.chars().filter(|&c| c == t.letter).count() as u32)
                .collect()
        }
        fn choose(n: u32, k: u32) -> f64 {
            if k > n {
                return 0.0;
            }
            (0..k).fold(1.0, |acc, i| acc * (n - i) as f64 / (i + 1) as f64)
        }
        let bag: Vec<u32> = dist.tiles.iter().map(|t| t.count).collect();
        let racks = dist.enumerate_racks(RACK_SIZE);
        let combos = |r: &[u32]| r.iter().zip(&bag).map(|(&n, &d)| choose(d, n)).product::<f64>();
        let (mut weighted, mut total) = (0.0, 0.0);
        for rack in &racks {
            let c = combos(&counts(dist, rack));
            weighted += mean_by_rack.get(rack).copied().unwrap_or(0.0) * c;
            total += c;
        }
        let average = weighted / total;

        let mut out = HashMap::new();
        for leave in dist.enumerate_leaves(MAX_LEAVE_SIZE) {
            let l = counts(dist, &leave);
            let (mut sum, mut count) = (0.0, 0.0);
            for rack in &racks {
                let r = counts(dist, rack);
                if r.iter().zip(&l).any(|(&rn, &ln)| ln > rn) {
                    continue;
                }
                let ways: f64 = (0..r.len()).map(|i| choose(bag[i] - l[i], r[i] - l[i])).product();
                sum += mean_by_rack.get(rack).copied().unwrap_or(0.0) * ways;
                count += ways;
            }
            out.insert(leave, if count > 0.0 { sum / count - average } else { 0.0 });
        }
        out
    }

    fn racks_dist() -> LetterDistribution {
        // Enough tiles for 7-tile racks, with repeats, a blank, and a letter
        // too scarce to fill most leaves.
        LetterDistribution::from_tiles_for_test(vec![
            Tile { letter: '?', count: 1 },
            Tile { letter: 'A', count: 4 },
            Tile { letter: 'B', count: 3 },
            Tile { letter: 'C', count: 2 },
            Tile { letter: 'D', count: 1 },
        ])
    }

    #[test]
    fn full_rack_derivation_matches_the_definition() {
        let dist = racks_dist();
        let racks = dist.enumerate_racks(RACK_SIZE);
        // Distinct, irregular means, with some racks left unobserved (mean 0)
        // so they still count toward the average, as in MAGPIE.
        let mut mean_by_rack = HashMap::new();
        for (i, rack) in racks.iter().enumerate() {
            if i % 5 != 3 {
                mean_by_rack.insert(rack.clone(), ((i * 37) % 101) as f64 * 0.25 - 12.0);
            }
        }

        let mut derived = FullRackLeaves::new(&dist).unwrap();
        for rack in &racks {
            // Racks arrive in any order and in any letter order.
            let shuffled: String = rack.chars().rev().collect();
            derived.add_rack(&shuffled, mean_by_rack.get(rack).copied().unwrap_or(0.0)).unwrap();
        }
        assert_eq!(derived.racks_added(), racks.len() as u64);

        let expected = reference_leave_values(&dist, &mean_by_rack);
        let mut compared = 0;
        for (leave, _, value) in derived.leave_values() {
            let want = expected[leave];
            assert!((value - want).abs() < 1e-9, "leave {leave:?}: derived {value}, expected {want}");
            compared += 1;
        }
        assert_eq!(compared, expected.len());

        // And the file carries exactly those values at their word indices.
        assert_eq!(derived.build(), build(&dist, &expected).unwrap());
    }

    #[test]
    fn full_rack_derivation_refuses_racks_that_are_not_full() {
        let mut derived = FullRackLeaves::new(&racks_dist()).unwrap();
        assert!(derived.add_rack("AAB", 1.0).is_err());
        assert!(derived.add_rack("AAAAABB?", 1.0).is_err());
        assert!(derived.add_rack("DDAAABB", 1.0).is_err(), "only one D in the bag");
        assert!(derived.add_rack("AAAABBZ", 1.0).is_err(), "Z is not in the distribution");
    }

    #[test]
    fn identical_rack_means_give_zero_leaves() {
        // Every rack worth the same: no leave is better than average.
        let dist = racks_dist();
        let mut derived = FullRackLeaves::new(&dist).unwrap();
        for rack in dist.enumerate_racks(RACK_SIZE) {
            derived.add_rack(&rack, 7.5).unwrap();
        }
        assert!(derived.leave_values().all(|(_, _, v)| v.abs() < 1e-9));
    }

    /// How long a real generation's derivation takes: every English full rack.
    /// Ignored (needs MAGPIE-DATA's english.csv, and wants `--release`):
    ///   cargo test --release --lib derives_every_english_rack -- --ignored --nocapture
    #[test]
    #[ignore]
    fn derives_every_english_rack() {
        let magpie_data = std::env::var("MAGPIE_DATA_PATH")
            .unwrap_or_else(|_| "../../MAGPIE/data".to_string());
        let bytes = std::fs::read(format!("{magpie_data}/letterdistributions/english.csv")).unwrap();
        let dist = LetterDistribution::parse(&bytes, "english").unwrap();
        let racks = dist.enumerate_racks(RACK_SIZE);
        assert_eq!(racks.len(), 3_199_724);

        let started = std::time::Instant::now();
        let mut derived = FullRackLeaves::new(&dist).unwrap();
        for (i, rack) in racks.iter().enumerate() {
            derived.add_rack(rack, (i % 97) as f64 - 48.0).unwrap();
        }
        let bytes = derived.build();
        println!("derived {} leaves from {} racks in {:?}", derived.layout.leaves.len(), racks.len(), started.elapsed());
        assert!(!bytes.is_empty());
    }

    /// Cross-validates against a real MAGPIE binary rather than trusting this
    /// module's own understanding of the format: builds a KLV giving every
    /// leave of `distribution_name` a distinct value, has a real `magpie
    /// convert klv2csv` read it back, and checks every leave comes back with
    /// exactly the value this module wrote -- which only holds if both the
    /// node byte-layout and the word-index algorithm are right, not merely
    /// self-consistent.
    ///
    /// Needs a MAGPIE checkout built at `MAGPIE_BIN` (env var, default
    /// `../../MAGPIE/bin/magpie` relative to this crate) and
    /// `MAGPIE_DATA_PATH` (default `../../MAGPIE/data`, for the default
    /// board layout MAGPIE loads before parsing `-path`, and for `english`).
    ///
    /// `distribution_bytes` is what birdtest parses and what MAGPIE is given:
    /// the same bytes go into the temp directory on MAGPIE's search path, so
    /// the two sides cannot be reading different copies of the alphabet. That
    /// is the whole point of pinning content, and it is why nothing here reads
    /// a `DATA_PATH`.
    fn assert_round_trips_through_a_real_magpie(
        distribution_name: &str,
        distribution_bytes: &[u8],
    ) {
        let magpie_bin = std::env::var("MAGPIE_BIN")
            .unwrap_or_else(|_| "../../MAGPIE/bin/magpie".to_string());
        let magpie_data = std::env::var("MAGPIE_DATA_PATH")
            .unwrap_or_else(|_| "../../MAGPIE/data".to_string());
        assert!(
            std::path::Path::new(&magpie_bin).exists(),
            "no magpie binary at {magpie_bin} -- set MAGPIE_BIN, or build one"
        );

        let distribution =
            LetterDistribution::parse(distribution_bytes, distribution_name).unwrap();
        let leaves = distribution.enumerate_leaves(MAX_LEAVE_SIZE);

        // A distinct, exactly-representable-in-f32-after-the-1000x-round-trip
        // value per leave, keyed by its position in enumeration order, so a
        // wrong word index shows up as leaf N getting leaf M's value.
        let mut mean_by_rack = HashMap::new();
        for (i, rack) in leaves.iter().enumerate() {
            mean_by_rack.insert(rack.clone(), (i as f64) * 0.001 - 10.0);
        }

        let bytes = build(&distribution, &mean_by_rack).unwrap();

        let dir = std::env::temp_dir().join(format!(
            "birdtest-klv-roundtrip-test-{distribution_name}-{}",
            std::process::id()
        ));
        let lexica_dir = dir.join("lexica");
        let ld_dir = dir.join("letterdistributions");
        std::fs::create_dir_all(&lexica_dir).unwrap();
        std::fs::create_dir_all(&ld_dir).unwrap();
        let name = "birdtest_klv_roundtrip_test";
        std::fs::write(lexica_dir.join(format!("{name}.klv2")), &bytes).unwrap();
        // MAGPIE reads the distribution by name off its search path, so the
        // fixture goes where MAGPIE will look. `testdist` is birdtest's own and
        // is not in MAGPIE-DATA at all.
        std::fs::write(
            ld_dir.join(format!("{distribution_name}.csv")),
            distribution_bytes,
        )
        .unwrap();

        let absolute = |p: &std::path::Path| {
            std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
        };
        let search_path = format!(
            "{}:{}",
            absolute(&dir).display(),
            absolute(std::path::Path::new(&magpie_data)).display()
        );

        let magpie_dir = std::path::Path::new(&magpie_bin).parent().unwrap().parent().unwrap();
        let output = std::process::Command::new(absolute(std::path::Path::new(&magpie_bin)))
            .arg("convert")
            .arg("klv2csv")
            .arg(name)
            .arg(distribution_name)
            .arg("-path")
            .arg(&search_path)
            .current_dir(magpie_dir)
            .output()
            .expect("failed to run magpie");
        assert!(
            output.status.success(),
            "magpie convert klv2csv failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let csv_path = lexica_dir.join(format!("{name}.csv"));
        let csv = std::fs::read_to_string(&csv_path)
            .unwrap_or_else(|e| panic!("magpie produced no CSV at {}: {e}", csv_path.display()));

        let mut seen = std::collections::HashSet::new();
        for line in csv.lines() {
            let (rack, value_str) = line.rsplit_once(',').expect("malformed CSV line");
            let value: f64 = value_str.parse().expect("non-numeric value");
            let expected = mean_to_equity_f32(*mean_by_rack.get(rack).unwrap_or_else(|| {
                panic!("magpie reported a rack birdtest never enumerated: {rack:?}")
            })) as f64;
            assert!(
                (value - expected).abs() < 1e-3,
                "rack {rack:?}: magpie read back {value}, birdtest wrote {expected}"
            );
            assert!(seen.insert(rack.to_string()), "duplicate rack in magpie's CSV: {rack:?}");
        }
        assert_eq!(seen.len(), leaves.len(), "magpie's CSV is missing some leaves");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // Both ignored by default (need a real MAGPIE build -- see the helper's
    // doc comment). Run explicitly with, e.g.:
    //   cargo test --bin birdtest round_trips -- --ignored

    /// birdtest's own tiny alphabet, small enough to enumerate exhaustively.
    /// Compiled in rather than read from disk: it is a test fixture, not
    /// runtime data, and birdtest no longer has a data directory.
    const TESTDIST: &[u8] = include_bytes!("testdata/testdist.csv");

    #[test]
    #[ignore]
    fn round_trips_through_a_real_magpie_testdist() {
        assert_round_trips_through_a_real_magpie("testdist", TESTDIST);
    }

    #[test]
    #[ignore]
    fn round_trips_through_a_real_magpie_english() {
        // The real English distribution, from the MAGPIE checkout the test
        // already requires -- birdtest keeps no copy of a MAGPIE-DATA file.
        let magpie_data = std::env::var("MAGPIE_DATA_PATH")
            .unwrap_or_else(|_| "../../MAGPIE/data".to_string());
        let path = std::path::Path::new(&magpie_data)
            .join("letterdistributions")
            .join("english.csv");
        let bytes = std::fs::read(&path)
            .unwrap_or_else(|e| panic!("no english.csv at {}: {e}", path.display()));
        assert_round_trips_through_a_real_magpie("english", &bytes);
    }
}
