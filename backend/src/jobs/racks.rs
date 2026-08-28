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

/// The letter distribution a lexicon uses.
///
/// MAGPIE derives this from the lexicon name's prefix (`ld_get_type_from_lex_name`)
/// rather than storing it, and there is no `nwl23` distribution file — NWL23 and
/// CSW21 both use `english`. This mirrors that mapping so both sides agree on
/// which file to read, and so the name handed to `magpie convert csv2klv`
/// resolves. An unrecognized lexicon falls back to its own lowercased name,
/// which is what makes the `testdist` development bag work.
pub fn letter_distribution_name(lexicon: &str) -> String {
    const ENGLISH: [&str; 7] = ["CSW", "NWL", "OSPD", "OSW", "TWL", "AMERICA", "CEL"];
    let upper = lexicon.to_uppercase();

    if ENGLISH.iter().any(|prefix| upper.starts_with(prefix)) {
        return "english".to_string();
    }
    for (prefix, name) in [
        ("RD", "german"),
        ("NSF", "norwegian"),
        ("DISC", "catalan"),
        ("FRA", "french"),
        ("OSPS", "polish"),
        ("DSW", "dutch"),
    ] {
        if upper.starts_with(prefix) {
            return name.to_string();
        }
    }
    lexicon.to_lowercase()
}

#[derive(Debug, Clone)]
pub struct LetterDistribution {
    pub tiles: Vec<Tile>,
}

impl LetterDistribution {
    pub fn load(data_path: &Path, lexicon: &str) -> AppResult<Self> {
        let dir = data_path.join("letterdistributions");
        let path = dir.join(format!("{}.csv", letter_distribution_name(lexicon)));
        let text = std::fs::read_to_string(&path).map_err(|e| {
            AppError::bad_request(format!(
                "no letter distribution for lexicon {lexicon:?} at {}: {e}",
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
