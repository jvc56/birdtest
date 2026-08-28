//! Letter-distribution loading and rack enumeration.
//!
//! Files mirror MAGPIE-DATA's `data/letterdistributions/` CSV layout:
//! `UPPER,lower,count,score,is_vowel`. Only the uppercase letter and the count
//! matter for enumeration; the remaining columns are kept so the files can be
//! copied between the two projects unchanged.

use crate::error::{AppError, AppResult};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct Tile {
    pub letter: char,
    pub count: u32,
}

#[derive(Debug, Clone)]
pub struct LetterDistribution {
    pub tiles: Vec<Tile>,
}

impl LetterDistribution {
    /// Loads a distribution by name -- `english`, `testdist` -- as the job
    /// states it. Nothing is inferred from the lexicon: MAGPIE derives a
    /// distribution from the lexicon's prefix, and mirroring that inference
    /// here made birdtest guess at something the job can simply say.
    pub fn load(data_path: &Path, name: &str) -> AppResult<Self> {
        let path = data_path
            .join("letterdistributions")
            .join(format!("{}.csv", name.to_lowercase()));
        let text = std::fs::read_to_string(&path).map_err(|e| {
            AppError::bad_request(format!(
                "no letter distribution named {name:?} at {}: {e}",
                path.display()
            ))
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
                    "malformed letter distribution line in {}: {line:?}",
                    path.display()
                )));
            }
            let letter = cols[0].chars().next().ok_or_else(|| {
                AppError::internal(format!("empty letter in {}", path.display()))
            })?;
            let count: u32 = cols[2].trim().parse().map_err(|_| {
                AppError::internal(format!("non-numeric tile count in {line:?}"))
            })?;
            if count > 0 {
                tiles.push(Tile { letter, count });
            }
        }

        if tiles.is_empty() {
            return Err(AppError::internal(format!("{} contains no tiles", path.display())));
        }
        // Canonical rack strings are sorted, so sorting the distribution once
        // means the enumeration emits already-canonical strings.
        tiles.sort_by_key(|t| t.letter);
        Ok(Self { tiles })
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
        LetterDistribution {
            tiles: vec![
                Tile { letter: 'A', count: 2 },
                Tile { letter: 'B', count: 1 },
                Tile { letter: 'C', count: 3 },
            ],
        }
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
