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
    /// Why this distribution's racks cannot be enumerated, if they cannot.
    ///
    /// A rack here is a string of one-character letters, so a distribution
    /// with a multi-character letter -- Catalan's `L·L`, `NY` and `QU` --
    /// has no faithful rack space in this representation: `L·L` would be read
    /// as a second `L`, and `QU` as a `Q` MAGPIE has never heard of. Parsing
    /// still succeeds, because a games job only hands the pinned bytes to
    /// MAGPIE and never enumerates anything; [`RackIndex::new`] is where it is
    /// refused, which is everything that does.
    unenumerable: Option<String>,
}

/// MAGPIE's `MAX_ALPHABET_SIZE` (`src/def/letter_distribution_defs.h`).
pub const MAGPIE_MAX_ALPHABET_SIZE: usize = 50;
/// MAGPIE's `MAX_SHIPPED_LETTER_BYTE_LENGTH`: the longest letter every one of
/// its letter buffers holds (Catalan's `L·L`). Its parser's own ceiling is 5
/// bytes (`MAX_LETTER_BYTE_LENGTH`, 6 with the terminator), but some buffers
/// are sized for the shipped letters and would cut a longer one short.
const MAGPIE_MAX_LETTER_BYTES: usize = 4;

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
        // Every row, zero-count ones included: MAGPIE numbers each row it
        // reads, so a row this parser dropped would shift every letter after
        // it (see the field comment).
        let mut machine_letters: Vec<char> = Vec::new();
        let mut unenumerable = None;
        // Read as MAGPIE reads it (`ld_create_internal`), so that a file this
        // accepts -- job creation's check -- is one every worker and builder
        // accepts, numbered the same way: lines split on '\n' with a trailing
        // '\r' dropped, empty lines skipped and nothing else (no comments, no
        // trimming: a `#` row is a letter to MAGPIE, and a whitespace-only line
        // an error), empty fields dropped, then five or seven columns -- upper,
        // lower, count, score, is_vowel, and the two fullwidth display forms --
        // with integer count and score and a vowel flag of 0 or 1. This used to
        // trim, skip `#` lines and want three columns, so a file MAGPIE refused
        // passed, and a `#` letter shifted every machine letter after it.
        let malformed = |line: &str, why: &str| {
            AppError::internal(format!(
                "malformed letter distribution line in {origin}: {line:?} ({why})"
            ))
        };
        for raw in text.split('\n') {
            // Only a line that is empty before its '\r' is dropped: MAGPIE
            // skips the empty items between two '\n's, but keeps a lone "\r"
            // -- the blank line of a CRLF file -- and refuses it for having no
            // columns.
            if raw.is_empty() {
                continue;
            }
            let line = raw.strip_suffix('\r').unwrap_or(raw);
            // Empty fields dropped first, then a trailing '\r' stripped, in
            // MAGPIE's order: a field that is only "\r" is a (then empty)
            // column to it, and a line with one has a column too many.
            let cols: Vec<&str> = line
                .split(',')
                .filter(|c| !c.is_empty())
                .map(|c| c.strip_suffix('\r').unwrap_or(c))
                .collect();
            if cols.len() != 5 && cols.len() != 7 {
                return Err(malformed(line, "expected 5 or 7 columns"));
            }
            let token = cols[0];
            if token.trim() != token || cols[1].trim() != cols[1] {
                return Err(malformed(line, "space around a letter"));
            }
            let letter = token.chars().next().ok_or_else(|| malformed(line, "empty letter"))?;
            // A letter MAGPIE can hold everywhere: past its parser's buffer a
            // letter ran into the next row's, and past the shipped length
            // some of its buffers cut it short.
            if token.len() > MAGPIE_MAX_LETTER_BYTES || cols[1].len() > MAGPIE_MAX_LETTER_BYTES {
                return Err(malformed(line, "a letter longer than 4 bytes"));
            }
            // The fullwidth display forms, when given, have MAGPIE's parser's
            // own ceiling (5 bytes; a fullwidth letter is 3): past it they
            // were copied without their terminator.
            if cols.len() == 7 && (cols[5].len() > 5 || cols[6].len() > 5) {
                return Err(malformed(line, "a display form longer than 5 bytes"));
            }
            // MAGPIE's string_to_int allows surrounding blanks around numbers.
            fn number(c: &str) -> &str {
                c.trim_matches([' ', '\t'])
            }
            let count: u32 = number(cols[2]).parse().map_err(|_| {
                AppError::internal(format!("non-numeric tile count in {origin}: {line:?}"))
            })?;
            // MAGPIE stores a letter's count in a byte: 256 read as none, and
            // a larger one overran its bag.
            if count > 255 {
                return Err(malformed(line, "a tile count above 255"));
            }
            number(cols[3]).parse::<i32>().map_err(|_| malformed(line, "non-numeric score"))?;
            if !matches!(number(cols[4]), "0" | "1") {
                return Err(malformed(line, "is_vowel must be 0 or 1"));
            }
            // A letter listed twice would enumerate every rack holding it
            // twice, and give it two machine-letter numbers.
            if unenumerable.is_none() {
                if token.chars().count() > 1 {
                    unenumerable = Some(format!(
                        "{origin} has the multi-character letter {token:?}, and racks here are \
                         strings of one-character letters"
                    ));
                } else if machine_letters.contains(&letter) {
                    unenumerable = Some(format!("duplicate letter {letter:?} in {origin}"));
                }
            }
            machine_letters.push(letter);
            if count > 0 {
                tiles.push(Tile { letter, count });
            }
        }

        if tiles.is_empty() {
            return Err(AppError::internal(format!("{origin} contains no tiles")));
        }
        // MAGPIE's per-letter arrays hold MAX_ALPHABET_SIZE letters; a longer
        // file was written past all of them by every build and worker that
        // loaded it (and MAGPIE now refuses it, a builder failing mid-job).
        if machine_letters.len() > MAGPIE_MAX_ALPHABET_SIZE {
            return Err(AppError::internal(format!(
                "{origin} has {} letters, and MAGPIE holds at most {MAGPIE_MAX_ALPHABET_SIZE}",
                machine_letters.len()
            )));
        }
        // Canonical rack strings are sorted, so sorting the distribution once
        // means the enumeration emits already-canonical strings.
        tiles.sort_by_key(|t| t.letter);
        Ok(Self {
            name: origin.to_string(),
            bytes: bytes.to_vec(),
            tiles,
            machine_letters,
            unenumerable,
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
        Self { name: "test".into(), bytes, tiles, machine_letters, unenumerable: None }
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
    /// The rack space of `size` tiles over `distribution`, or a `400` naming
    /// why it has none this representation can express (see
    /// `LetterDistribution::unenumerable`). Every job that hands out or seeds
    /// racks comes through here, so a distribution it cannot enumerate is
    /// refused rather than enumerated wrongly.
    pub fn new(distribution: &LetterDistribution, size: usize) -> AppResult<Self> {
        if let Some(reason) = &distribution.unenumerable {
            return Err(AppError::bad_request(format!(
                "cannot enumerate the racks of this distribution: {reason}"
            )));
        }
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
        Ok(Self { tiles, counts, size })
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

    /// U-RACK-5: counts above 2 (`C` in `tiny`, `A`/`E` in `testdist.csv`)
    /// exercise multiset repetition, and `testdist.csv` adds a blank.
    #[test]
    fn unranking_matches_full_enumeration() {
        // The scattered index must still cover exactly the set the naive walk
        // produces -- every rack once, none twice -- or a job would analyse
        // some racks twice and miss others entirely.
        for (distribution, size) in (1..=4)
            .map(|size| (tiny(), size))
            .chain((1..=7).map(|size| (testdist(), size)))
        {
            let index = RackIndex::new(&distribution, size).unwrap();
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
        let index = RackIndex::new(&distribution, 3).unwrap();
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
        let index = RackIndex::new(&tiny(), 3).unwrap();
        assert!(index.rack_at(index.total()).is_none());
        // A range that runs off the end yields only what exists, which is how
        // the final batch of a job comes up short.
        let tail = index.racks_in_range(index.total() - 2, 10);
        assert_eq!(tail.len(), 2);
    }

    /// MAGPIE's own `english.csv` from `data-20251004.tgz`, copied verbatim
    /// from MAGPIE's `data/letterdistributions/`. Its digest is the one
    /// `contract-fixtures/` pins, which the first test below checks.
    const ENGLISH: &[u8] = include_bytes!("testdata/english.csv");
    /// Five letters and a blank, with counts of 1, 2 and 3.
    const TESTDIST: &[u8] = include_bytes!("testdata/testdist.csv");

    fn english() -> LetterDistribution {
        LetterDistribution::parse(ENGLISH, "english").unwrap()
    }

    fn testdist() -> LetterDistribution {
        LetterDistribution::parse(TESTDIST, "testdist").unwrap()
    }

    fn count_of(distribution: &LetterDistribution, letter: char) -> Option<u32> {
        distribution.tiles.iter().find(|t| t.letter == letter).map(|t| t.count)
    }

    /// U-RACK-1: the real English file parses to its 27 rows and 100 tiles,
    /// numbered in file order the way MAGPIE numbers them.
    #[test]
    fn the_real_english_distribution_parses_with_magpies_numbering() {
        use sha2::{Digest, Sha256};
        assert_eq!(
            hex::encode(Sha256::digest(ENGLISH)),
            "e698ce0b93e025daccd3390107a914d74581c4603de38df65131768b1c6f9102",
            "testdata/english.csv is not the pinned data-20251004 file"
        );

        let english = english();
        assert_eq!(english.name, "english");
        assert_eq!(english.bytes, ENGLISH, "the pinned bytes are kept, not re-rendered");
        assert_eq!(english.tiles.len(), 27);
        assert_eq!(english.tiles.iter().map(|t| t.count).sum::<u32>(), 100);
        for (letter, count) in [('?', 2), ('A', 9), ('E', 12), ('Q', 1), ('Z', 1)] {
            assert_eq!(count_of(&english, letter), Some(count), "{letter}");
        }
        // Sorted for enumeration, and the blank sorts first.
        let letters: Vec<char> = english.tiles.iter().map(|t| t.letter).collect();
        assert!(letters.windows(2).all(|w| w[0] < w[1]), "{letters:?}");
        assert_eq!(letters[0], '?');

        assert_eq!(english.machine_letter('?'), Some(0));
        assert_eq!(english.machine_letter('A'), Some(1));
        assert_eq!(english.machine_letter('Z'), Some(26));
        assert_eq!(english.machine_letter('a'), None, "the lowercase column is not a letter");
        assert_eq!(english.machine_letter('!'), None);
    }

    /// U-RACK-1: the smallest distribution there is. Written out of order, so
    /// the machine-letter numbering (file order) and the enumeration order
    /// (sorted) visibly differ; with a blank line, a CRLF ending and the short
    /// five-column form, which the parser skips and accepts, as MAGPIE does.
    #[test]
    fn a_minimal_two_letter_distribution_parses() {
        let text = b"B,b,1,3,0\r\n\nA,a,2,1,1\n";
        let distribution = LetterDistribution::parse(text, "two").unwrap();
        assert_eq!(distribution.tiles.len(), 2);
        assert_eq!(count_of(&distribution, 'A'), Some(2));
        assert_eq!(count_of(&distribution, 'B'), Some(1));
        assert_eq!(distribution.tiles[0].letter, 'A', "tiles are sorted");
        assert_eq!(distribution.machine_letter('B'), Some(0), "numbered in file order");
        assert_eq!(distribution.machine_letter('A'), Some(1));
        assert_eq!(distribution.enumerate_racks(2), ["AA", "AB"]);
    }

    /// U-RACK-1: MAGPIE numbers every row it reads, so a zero-count row still
    /// takes a machine letter. Dropping it along with its tiles would shift
    /// every letter after it by one in a KWG built from this numbering.
    #[test]
    fn a_zero_count_row_keeps_its_machine_letter_but_has_no_tiles() {
        let distribution =
            LetterDistribution::parse(b"A,a,1,1,1\nB,b,0,3,0\nC,c,1,3,0\n", "zero").unwrap();
        assert_eq!(count_of(&distribution, 'B'), None, "no tiles to draw");
        assert_eq!(distribution.machine_letter('C'), Some(2));
        assert_eq!(distribution.enumerate_racks(2), ["AC"]);
    }

    fn parse_error(text: &str) -> String {
        match LetterDistribution::parse(text.as_bytes(), "origin-name.csv") {
            Ok(_) => panic!("{text:?} should be rejected"),
            Err(e) => e.message,
        }
    }

    /// U-RACK-9: a distribution MAGPIE cannot hold -- more letters than its
    /// `MAX_ALPHABET_SIZE` -- is refused, naming the file; one at the limit
    /// parses. (MAGPIE loaded a longer one and wrote past every per-letter
    /// array; job creation now refuses it up front.)
    #[test]
    fn a_distribution_past_magpies_alphabet_is_refused() {
        let rows = |n: usize| -> String {
            (0..n)
                .map(|i| {
                    let letter = char::from_u32(0x100 + i as u32).unwrap();
                    format!("{letter},{letter},1,1,0\n")
                })
                .collect()
        };
        assert!(LetterDistribution::parse(rows(MAGPIE_MAX_ALPHABET_SIZE).as_bytes(), "fifty").is_ok());
        let message = parse_error(&rows(MAGPIE_MAX_ALPHABET_SIZE + 1));
        assert!(message.contains("at most 50"), "{message}");
        assert!(message.contains("origin-name.csv"), "{message}");
    }

    /// U-RACK-10: what MAGPIE refuses, this refuses, and what MAGPIE numbers,
    /// this numbers the same. A comment line, a whitespace-only line, the wrong
    /// column count, a non-integer score, a vowel flag other than 0 or 1 and a
    /// letter with a space round it are each refused (MAGPIE refuses the first
    /// five, and would read the last as a two-character letter); a `#` row is a
    /// letter, and takes its machine letter.
    #[test]
    fn it_reads_a_distribution_as_magpie_does() {
        for (text, why) in [
            ("# upper,lower,count,score,vowel\nA,a,1,1,1\n", "a comment"),
            ("A,a,1,1,1\n  \n", "a whitespace-only line"),
            ("A,a,1,1,1\r\n\r\nB,b,1,1,0\r\n", "a CRLF blank line"),
            ("A,a,1,1,1\r\n\r\n", "a trailing CRLF blank line"),
            ("C,\r,c,2,3,0\n", "a field that is only a carriage return"),
            ("A,a,256,1,1\n", "a count above 255"),
            ("ABCDE,abcde,1,1,1\n", "a five-byte letter"),
            ("A,a,1,1,1,AAAAAA,a\n", "a six-byte display form"),
            ("A,a,1,1\n", "four columns"),
            ("A,a,1,1,1,A\n", "six columns"),
            ("A,a,1,one,1\n", "a non-integer score"),
            ("A,a,1,1,2\n", "a vowel flag of 2"),
            (" A,a,1,1,1\n", "a space before the letter"),
        ] {
            // Refused (parse_error panics otherwise), naming the file.
            let message = parse_error(text);
            assert!(message.contains("origin-name.csv"), "{why}: {message}");
        }
        let hash = LetterDistribution::parse(b"#,#,1,1,0\nA,a,1,1,1\n", "hash").unwrap();
        assert_eq!(hash.machine_letter('#'), Some(0));
        assert_eq!(hash.machine_letter('A'), Some(1), "numbered after the `#` row");
    }

    /// U-RACK-2: each malformed shape is refused for its own reason, and every
    /// message names the file it came from.
    #[test]
    fn each_malformed_distribution_is_rejected_for_its_own_reason() {
        let cases = [
            ("A,a,9,1,1\nB,b\n", "malformed letter distribution line"),
            ("A,a,nine,1,1\n", "non-numeric tile count"),
            ("", "contains no tiles"),
            ("\n\n", "contains no tiles"),
        ];
        for (text, reason) in cases {
            let message = parse_error(text);
            assert!(message.contains(reason), "{text:?}: {message}");
            assert!(message.contains("origin-name.csv"), "{text:?}: {message}");
        }
        // Distinct: no one message would satisfy two of the reasons.
        let short = parse_error(cases[0].0);
        assert!(!short.contains("non-numeric") && !short.contains("no tiles"));
        assert!(!parse_error(cases[1].0).contains("malformed"));
    }

    const CATALAN: &[u8] = include_bytes!("testdata/catalan.csv");

    /// U-RACK-2: a distribution this representation cannot enumerate still
    /// parses -- a games job only hands its bytes to MAGPIE -- and every rack
    /// space over it is refused with the reason: a letter listed twice, and
    /// the real Catalan file, whose `L·L`, `NY` and `QU` are one tile each to
    /// MAGPIE and would be enumerated here as a second `L`, a second `N` and a
    /// `Q` MAGPIE does not have. Refusing the duplicate in `parse`, as this
    /// once did, made every Catalan job fail to load, games included.
    #[test]
    fn a_distribution_whose_racks_cannot_be_spelt_parses_but_has_no_rack_space() {
        let duplicate =
            LetterDistribution::parse(b"A,a,9,1,1\nB,b,2,3,0\nA,a,1,1,1\n", "dup.csv").unwrap();
        let err = RackIndex::new(&duplicate, 2).err().expect("refused");
        assert_eq!(err.status, axum::http::StatusCode::BAD_REQUEST);
        assert!(err.message.contains("duplicate letter 'A' in dup.csv"), "{}", err.message);

        let catalan = LetterDistribution::parse(CATALAN, "catalan").unwrap();
        assert_eq!(catalan.bytes, CATALAN, "the pinned bytes go to MAGPIE untouched");
        for size in [1, 7] {
            let err = RackIndex::new(&catalan, size).err().expect("refused");
            assert_eq!(err.status, axum::http::StatusCode::BAD_REQUEST);
            assert!(
                err.message.contains("multi-character letter \"L·L\"")
                    && err.message.contains("catalan"),
                "{}",
                err.message
            );
        }
        assert!(crate::jobs::opening_rack::total_racks(&catalan, 7).is_err());
    }

    /// U-RACK-3: every rack of every size is sorted, distinct, drawable from
    /// the bag, and there are exactly `total()` of them.
    #[test]
    fn every_enumerated_rack_is_canonical_distinct_and_counted() {
        for (distribution, max) in [(tiny(), 6), (testdist(), 7)] {
            for size in 0..=max {
                let racks = distribution.enumerate_racks(size);
                assert_eq!(
                    racks.len() as u64,
                    RackIndex::new(&distribution, size).unwrap().total(),
                    "size {size}"
                );
                let distinct: std::collections::HashSet<&String> = racks.iter().collect();
                assert_eq!(distinct.len(), racks.len(), "size {size}: duplicates");
                for rack in &racks {
                    let letters: Vec<char> = rack.chars().collect();
                    assert_eq!(letters.len(), size, "{rack}");
                    assert!(letters.windows(2).all(|w| w[0] <= w[1]), "{rack} is not sorted");
                    for tile in &distribution.tiles {
                        let used = letters.iter().filter(|&&c| c == tile.letter).count();
                        assert!(used as u32 <= tile.count, "{rack} overdraws {}", tile.letter);
                    }
                }
            }
        }
        // Past the bag: seven tiles cannot be drawn from a bag of six.
        assert!(tiny().enumerate_racks(7).is_empty());
        assert_eq!(RackIndex::new(&tiny(), 7).unwrap().total(), 0);
    }

    /// U-RACK-4: the leave universe is every size from 1 to the maximum, in
    /// size order, and nothing else.
    #[test]
    fn the_leave_universe_is_every_size_up_to_the_maximum() {
        let distribution = testdist();
        for max in 1..=6 {
            let leaves = distribution.enumerate_leaves(max);
            let by_size: Vec<String> =
                (1..=max).flat_map(|size| distribution.enumerate_racks(size)).collect();
            assert_eq!(leaves, by_size, "max {max}");
            let counted: u64 =
                (1..=max).map(|size| RackIndex::new(&distribution, size).unwrap().total()).sum();
            assert_eq!(leaves.len() as u64, counted, "max {max}");
            assert!(leaves.iter().all(|l| (1..=max).contains(&l.chars().count())));
        }
        assert!(distribution.enumerate_leaves(0).is_empty());
    }

    /// U-RACK-6: a range is the racks at those indices, in index order, and
    /// one that runs off the end stops there.
    ///
    /// TESTING.md words this as "equals the slice of `enumerate_racks`", which
    /// is not the design: `racks_in_range` scatters (see `rack_at`), so its
    /// slices tile the enumeration's *set*, not its order. What is a plain
    /// slice is `racks_in_enumeration_range`, which leave generation walks,
    /// and its order is exactly `enumerate_racks` reversed -- the walk takes
    /// the most of each tile first, the index the fewest.
    #[test]
    fn a_range_is_the_racks_at_its_indices_and_stops_at_the_end() {
        let distribution = testdist();
        let size = 4;
        let index = RackIndex::new(&distribution, size).unwrap();
        let total = index.total();
        let mut enumerated = distribution.enumerate_racks(size);
        enumerated.reverse();

        for (start, count) in [(0, 5), (7, 11), (total - 3, 3), (total - 3, 10), (total, 4)] {
            let expected: Vec<String> =
                (start..(start + count).min(total)).map(|i| index.rack_at(i).unwrap()).collect();
            assert_eq!(index.racks_in_range(start, count), expected, "{start}+{count}");

            let end = ((start + count).min(total)) as usize;
            assert_eq!(
                index.racks_in_enumeration_range(start, count),
                enumerated[start as usize..end],
                "raw {start}+{count}"
            );
        }
        assert_eq!(index.racks_in_range(total - 3, 10).len(), 3);
        assert!(index.racks_in_range(u64::MAX - 1, 5).is_empty(), "no overflow at the top");

        // Consecutive batches -- how a job hands out the space -- cover every
        // rack exactly once.
        let mut batched: Vec<String> =
            (0..total).step_by(6).flat_map(|start| index.racks_in_range(start, 6)).collect();
        assert_eq!(batched.len() as u64, total);
        batched.sort();
        enumerated.sort();
        assert_eq!(batched, enumerated);
    }

    /// U-RACK-7: the two sizes PLAN.md quotes and job creation is costed on,
    /// from the counting table alone. Also pins a handful of rack ids: results
    /// are stored against them, so a change to the ordering or the scatter
    /// would silently re-point every existing row. The expected racks were
    /// computed by an independent model of the documented order, not read off
    /// this implementation.
    #[test]
    fn real_english_has_3199724_racks_and_914624_leaves() {
        let english = english();
        let racks = RackIndex::new(&english, 7).unwrap();
        assert_eq!(racks.total(), 3_199_724);
        let leaves: u64 = (1..=6).map(|size| RackIndex::new(&english, size).unwrap().total()).sum();
        assert_eq!(leaves, 914_624);

        for (id, rack) in [
            (0, "VWWXYYZ"),
            (1, "MQVVXYY"),
            (2, "LMORTUX"),
            (1_000, "?BFHQTW"),
            (1_599_862, "BFFNOPX"),
            (3_199_723, "??BDNPW"),
        ] {
            assert_eq!(racks.rack_at(id).as_deref(), Some(rack), "rack id {id}");
        }
        assert_eq!(racks.racks_in_enumeration_range(0, 1), ["VWWXYYZ"]);
        assert_eq!(racks.racks_in_enumeration_range(3_199_723, 1), ["??AAAAA"]);
    }

    /// U-RACK-8: the blank is the character the file uses (`?`), sorts before
    /// every letter, so it leads any canonical rack holding one; in the raw
    /// index order, which takes the fewest of each tile first, every rack
    /// with a blank comes after every rack without. And a rack holding one
    /// comes back out of `rack_at` exactly once, blank first.
    #[test]
    fn blanks_lead_their_racks_and_sit_at_the_end_of_the_index() {
        let english = english();
        let index = RackIndex::new(&english, 2).unwrap();
        let raw = index.racks_in_enumeration_range(0, index.total());
        let first_blank = raw.iter().position(|r| r.contains('?')).unwrap();
        assert!(raw[first_blank..].iter().all(|r| r.starts_with('?')), "{raw:?}");
        assert!(raw[..first_blank].iter().all(|r| !r.contains('?')));
        // 26 racks of a blank and a letter, and the double blank.
        assert_eq!(raw.len() - first_blank, 27);
        assert_eq!(raw.last().map(String::as_str), Some("??"));

        let distribution = testdist();
        let index = RackIndex::new(&distribution, 3).unwrap();
        let unranked: Vec<String> =
            (0..index.total()).map(|i| index.rack_at(i).unwrap()).collect();
        for rack in ["?AB", "?EE", "?AA"] {
            assert_eq!(unranked.iter().filter(|r| *r == rack).count(), 1, "{rack}");
        }
        assert!(unranked.iter().filter(|r| r.contains('?')).all(|r| r.starts_with('?')));
        assert!(!unranked.iter().any(|r| r.matches('?').count() > 1), "only one blank in the bag");
    }
}
