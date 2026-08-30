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

/// MAGPIE's `RACK_SIZE - 1`. `RACK_SIZE` is fixed at 7 across this whole
/// system (both MAGPIE's own build and birdtest's schema assume it), so a
/// KLV's leave domain -- unlike `job_leave_config.max_leave_size`, which only
/// bounds what a job actually *observes* -- is always every leave of 1..=6
/// tiles. `klv_write_to_csv`/`klv_create_empty` in MAGPIE hardcode this same
/// bound; matching it here is what keeps a birdtest-built KLV byte-for-byte
/// interchangeable with one MAGPIE would have built for the same data.
pub const MAX_LEAVE_SIZE: usize = 6;

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

/// Builds a complete KLV2 file: every leave of 1..=[`MAX_LEAVE_SIZE`] tiles
/// drawable from `distribution`, valued from `mean_by_rack` where present and
/// zero everywhere else -- exactly `magpie convert csv2klv`'s behavior for a
/// leaves CSV that doesn't mention every leave (`job_leave_config`'s own
/// `max_leave_size` is typically smaller than 6, so most of the domain is
/// zero by design, not by omission).
pub fn build(
    distribution: &LetterDistribution,
    mean_by_rack: &HashMap<String, f64>,
) -> AppResult<Vec<u8>> {
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

    let mut leave_values = vec![0.0f32; leaves.len()];
    for (rack, letters) in leaves.iter().zip(leaf_letters.iter()) {
        let index = word_index_for(&nodes, &counts, root, letters) as usize;
        let mean = mean_by_rack.get(rack).copied().unwrap_or(0.0);
        leave_values[index] = mean_to_equity_f32(mean);
    }

    Ok(serialize(&nodes, &leave_values))
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
        for chunk in bytes[values_start..].chunks_exact(4) {
            let v = f32::from_le_bytes(chunk.try_into().unwrap());
            assert_eq!(v, 0.0);
        }
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
    /// board layout MAGPIE loads before parsing `-path`).
    fn assert_round_trips_through_a_real_magpie(distribution_name: &str) {
        let magpie_bin = std::env::var("MAGPIE_BIN")
            .unwrap_or_else(|_| "../../MAGPIE/bin/magpie".to_string());
        let magpie_data = std::env::var("MAGPIE_DATA_PATH")
            .unwrap_or_else(|_| "../../MAGPIE/data".to_string());
        assert!(
            std::path::Path::new(&magpie_bin).exists(),
            "no magpie binary at {magpie_bin} -- set MAGPIE_BIN, or build one"
        );

        let data_path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../data");
        let distribution = LetterDistribution::load(&data_path, distribution_name).unwrap();
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
        std::fs::create_dir_all(&lexica_dir).unwrap();
        let name = "birdtest_klv_roundtrip_test";
        std::fs::write(lexica_dir.join(format!("{name}.klv2")), &bytes).unwrap();

        let absolute = |p: &std::path::Path| {
            std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
        };
        let search_path = format!(
            "{}:{}:{}",
            absolute(&dir).display(),
            // Non-MAGPIE-DATA distributions (testdist) need birdtest's own
            // data dir on the path too -- matching what the old shelled-out
            // run_transition did.
            absolute(&data_path).display(),
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

    #[test]
    #[ignore]
    fn round_trips_through_a_real_magpie_testdist() {
        assert_round_trips_through_a_real_magpie("testdist");
    }

    #[test]
    #[ignore]
    fn round_trips_through_a_real_magpie_english() {
        assert_round_trips_through_a_real_magpie("english");
    }
}
