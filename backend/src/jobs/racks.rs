//! Letter-distribution loading and rack enumeration.
//!
//! Files mirror MAGPIE-DATA's `data/letterdistributions/` CSV layout:
//! `UPPER,lower,count,score,is_vowel`. Only the uppercase letter and the count
//! matter for enumeration; the remaining columns are kept so the files can be
//! copied between the two projects unchanged.

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone)]
pub struct Tile {
    pub letter: char,
    pub count: u32,
}

#[derive(Debug, Clone)]
pub struct LetterDistribution {
    /// The `input_data` name this was parsed from -- `english`, `polish`.
    ///
    /// Carried because the server now hands the distribution to MAGPIE by
    /// name: a conversion writes `letterdistributions/<name>.csv` into a
    /// scratch directory and states it on the command line. Stating it is not
    /// optional -- MAGPIE would otherwise infer one from the lexicon's name,
    /// and a wordmap built against an inferred distribution is not necessarily
    /// the one built against the distribution the job pins.
    pub name: String,
    /// The exact bytes of the pinned row.
    ///
    /// Kept rather than re-serialized from `tiles`: what MAGPIE reads has to
    /// be the bytes the job pinned and the worker verified, not this parser's
    /// idea of them. Round-tripping through the parse would drop the columns
    /// this file does not read -- scores, vowel flags, display forms -- every
    /// one of which MAGPIE does read.
    pub bytes: Vec<u8>,
    pub tiles: Vec<Tile>,
    /// Letters in the order they appeared in the distribution file, before
    /// the canonical sort below. This is MAGPIE's own machine-letter
    /// numbering: it assigns index 0, 1, 2, ... to each row as it reads the
    /// file, in file order, never re-sorted. `klv.rs` needs this exact
    /// numbering baked into a KWG's node bytes; nothing else in this struct
    /// does, which is why it's kept separately rather than replacing `tiles`.
    machine_letters: Vec<char>,
}

impl LetterDistribution {
    /// Parses a distribution from the bytes of the `input_data` row a job
    /// pins. There is no path-taking constructor and no `DATA_PATH`: the
    /// server computes over exactly the bytes the worker is checked against,
    /// so the two cannot drift. `origin` names the file for error messages.
    pub fn parse(bytes: &[u8], origin: &str) -> AppResult<Self> {
        let text = std::str::from_utf8(bytes).map_err(|e| {
            AppError::internal(format!("letter distribution {origin} is not UTF-8: {e}"))
        })?;

        let mut tiles = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // MAGPIE's files carry seven columns — upper, lower, count, score,
            // is_vowel, and the two fullwidth display forms. Only the letter and
            // the count matter here; the rest are read by MAGPIE itself.
            let cols: Vec<&str> = line.split(',').collect();
            if cols.len() < 3 {
                return Err(AppError::internal(format!(
                    "malformed letter distribution line in {origin}: {line:?}"
                )));
            }
            let letter = cols[0].chars().next().ok_or_else(|| {
                AppError::internal(format!("empty letter in {origin}"))
            })?;
            let count: u32 = cols[2].trim().parse().map_err(|_| {
                AppError::internal(format!("non-numeric tile count in {line:?}"))
            })?;
            if count > 0 {
                tiles.push(Tile { letter, count });
            }
        }

        if tiles.is_empty() {
            return Err(AppError::internal(format!("{origin} contains no tiles")));
        }
        let machine_letters: Vec<char> = tiles.iter().map(|t| t.letter).collect();
        // Canonical rack strings are sorted, so sorting the distribution once
        // means the enumeration emits already-canonical strings.
        tiles.sort_by_key(|t| t.letter);
        Ok(Self {
            name: origin.to_string(),
            bytes: bytes.to_vec(),
            tiles,
            machine_letters,
        })
    }

    /// MAGPIE's machine-letter index for `letter` -- see the `machine_letters`
    /// field comment. `None` means the letter isn't in this distribution at
    /// all, which for a leave actually enumerated from it is a bug, not a
    /// legitimate input to handle gracefully.
    pub fn machine_letter(&self, letter: char) -> Option<u8> {
        self.machine_letters
            .iter()
            .position(|&c| c == letter)
            .map(|i| i as u8)
    }

    /// Builds a distribution from an explicit tile list, for tests only --
    /// everywhere else goes through [`Self::parse`], which is the only place
    /// that should construct one from the bytes a job pins.
    ///
    /// The bytes it carries are a rendering of the tiles rather than the
    /// original file, which is exactly why this is test-only: what MAGPIE
    /// reads has to be the pinned bytes, not a reconstruction of them.
    #[cfg(test)]
    pub fn from_tiles_for_test(tiles: Vec<Tile>) -> Self {
        let machine_letters: Vec<char> = tiles.iter().map(|t| t.letter).collect();
        let bytes = tiles
            .iter()
            .map(|t| format!("{},{},{},1,0\n", t.letter, t.letter.to_lowercase(), t.count))
            .collect::<String>()
            .into_bytes();
        let mut tiles = tiles;
        tiles.sort_by_key(|t| t.letter);
        Self { name: "test".into(), bytes, tiles, machine_letters }
    }

    /// Every distinct multiset of exactly `size` tiles drawable from the bag,
    /// as a canonical sorted string. Blanks appear as whatever character the
    /// distribution file uses for them (`?` in the MAGPIE files).
    pub fn enumerate_racks(&self, size: usize) -> Vec<String> {
        let mut out = Vec::new();
        let mut current = String::with_capacity(size);
        self.walk(0, size, &mut current, &mut out);
        out
    }

    /// Every distinct multiset of 1..=`max_size` tiles — the leave universe for
    /// a leave-generation job.
    pub fn enumerate_leaves(&self, max_size: usize) -> Vec<String> {
        let mut out = Vec::new();
        for size in 1..=max_size {
            out.extend(self.enumerate_racks(size));
        }
        out
    }

    fn walk(&self, index: usize, remaining: usize, current: &mut String, out: &mut Vec<String>) {
        if remaining == 0 {
            out.push(current.clone());
            return;
        }
        if index >= self.tiles.len() {
            return;
        }
        // Prune branches that cannot possibly fill the rack from what's left.
        let available: u32 = self.tiles[index..].iter().map(|t| t.count).sum();
        if (available as usize) < remaining {
            return;
        }

        let tile = &self.tiles[index];
        let max_take = tile.count.min(remaining as u32);
        for take in (0..=max_take).rev() {
            for _ in 0..take {
                current.push(tile.letter);
            }
            self.walk(index + 1, remaining - take as usize, current, out);
            for _ in 0..take {
                current.pop();
            }
        }
    }
}

/// Indexes the space of distinct racks so a task can name a *range* of them
/// rather than carrying the racks themselves.
///
/// `counts[i][k]` is how many distinct k-tile racks can be drawn from tiles
/// `i..`, which is enough both to count the whole space and to address the
/// k-th rack in it directly. The table is tiny -- 27 letters by 8 sizes for
/// English -- so unranking one rack is a handful of additions rather than a
/// walk over the millions of racks that precede it.
pub struct RackIndex {
    tiles: Vec<Tile>,
    counts: Vec<Vec<u64>>,
    size: usize,
}

impl RackIndex {
    pub fn new(distribution: &LetterDistribution, size: usize) -> Self {
        let tiles = distribution.tiles.clone();
        let n = tiles.len();
        // The extra row is the empty suffix, which can only make the empty rack.
        let mut counts = vec![vec![0u64; size + 1]; n + 1];
        counts[n][0] = 1;
        for i in (0..n).rev() {
            let available = tiles[i].count as usize;
            for k in 0..=size {
                let mut total: u64 = 0;
                for take in 0..=available.min(k) {
                    total = total.saturating_add(counts[i + 1][k - take]);
                }
                counts[i][k] = total;
            }
        }
        Self { tiles, counts, size }
    }

    /// How many distinct racks of this size exist -- the job's total.
    pub fn total(&self) -> u64 {
        self.counts[0][self.size]
    }

    /// The rack at `index`, or `None` past the end.
    ///
    /// Ordering is by ascending count of each tile in distribution order, and
    /// must stay stable: results are recorded against racks unranked from an
    /// index, so changing the order would silently re-point old results.
    pub fn rack_at(&self, index: u64) -> Option<String> {
        if index >= self.total() {
            return None;
        }
        self.rack_at_enumeration_index(self.scatter(index))
    }

    /// Maps an index onto a different one, bijectively.
    ///
    /// Multiplying by a value coprime with the total permutes `[0, total)`, so
    /// every index still names exactly one distinct rack and the space is still
    /// covered exactly once. The multiplier is a prime larger than any plausible
    /// rack space, which makes it coprime with every total.
    fn scatter(&self, index: u64) -> u64 {
        // 2^61 - 1, a Mersenne prime.
        const MULTIPLIER: u128 = 2_305_843_009_213_693_951;
        ((index as u128 * MULTIPLIER) % self.total() as u128) as u64
    }

    /// The rack at a raw enumeration index, in ascending tile-count order.
    fn rack_at_enumeration_index(&self, index: u64) -> Option<String> {
        if index >= self.total() {
            return None;
        }
        let mut remaining_index = index;
        let mut remaining_size = self.size;
        let mut rack = String::with_capacity(self.size);

        for i in 0..self.tiles.len() {
            let available = self.tiles[i].count as usize;
            for take in 0..=available.min(remaining_size) {
                let block = self.counts[i + 1][remaining_size - take];
                if remaining_index < block {
                    for _ in 0..take {
                        rack.push(self.tiles[i].letter);
                    }
                    remaining_size -= take;
                    break;
                }
                remaining_index -= block;
            }
        }
        Some(rack)
    }

    /// The racks at raw enumeration indices `[start, start + count)`, without
    /// scattering, stopping at the end of the space. For walking the whole
    /// space in bounded chunks.
    pub fn racks_in_enumeration_range(&self, start: u64, count: u64) -> Vec<String> {
        (start..start.saturating_add(count))
            .map_while(|index| self.rack_at_enumeration_index(index))
            .collect()
    }

    /// The racks in `[start, start + count)`, stopping at the end of the space.
    /// The final batch of a job comes up short this way.
    pub fn racks_in_range(&self, start: u64, count: u64) -> Vec<String> {
        (start..start.saturating_add(count))
            .map_while(|index| self.rack_at(index))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny() -> LetterDistribution {
        LetterDistribution::from_tiles_for_test(vec![
            Tile { letter: 'A', count: 2 },
            Tile { letter: 'B', count: 1 },
            Tile { letter: 'C', count: 3 },
        ])
    }

    #[test]
    fn unranking_matches_full_enumeration() {
        // The scattered index must still cover exactly the set the naive walk
        // produces -- every rack once, none twice -- or a job would analyse
        // some racks twice and miss others entirely.
        for size in 1..=4 {
            let distribution = tiny();
            let index = RackIndex::new(&distribution, size);
            let mut enumerated = distribution.enumerate_racks(size);
            enumerated.sort();

            assert_eq!(index.total() as usize, enumerated.len(), "size {size}");

            let mut unranked: Vec<String> =
                (0..index.total()).filter_map(|i| index.rack_at(i)).collect();
            assert_eq!(unranked.len(), enumerated.len(), "size {size}");
            unranked.sort();
            assert_eq!(unranked, enumerated, "size {size}");
        }
    }

    #[test]
    fn scattering_spreads_adjacent_indices() {
        // The point of scattering: a contiguous batch must not be a run of
        // near-identical racks. In the raw enumeration the first few indices
        // share a prefix; scattered, they should not.
        let distribution = tiny();
        let index = RackIndex::new(&distribution, 3);
        let batch = index.racks_in_range(0, 4);
        let distinct_first_letters: std::collections::HashSet<char> =
            batch.iter().filter_map(|rack| rack.chars().next()).collect();
        assert!(
            distinct_first_letters.len() > 1,
            "a batch should span the space, got {batch:?}"
        );
    }

    #[test]
    fn unranking_is_past_the_end_safe() {
        let index = RackIndex::new(&tiny(), 3);
        assert!(index.rack_at(index.total()).is_none());
        // A range that runs off the end yields only what exists, which is how
        // the final batch of a job comes up short.
        let tail = index.racks_in_range(index.total() - 2, 10);
        assert_eq!(tail.len(), 2);
    }
}
