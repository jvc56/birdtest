//! MAGPIE's lexicon / leaves / letter-distribution compatibility rules, ported.
//!
//! birdtest must not be able to build a job MAGPIE would refuse to load, and
//! MAGPIE decides that from *names*: both players' lexicons must agree on a
//! letter-distribution type, each player's leaves must agree with its own
//! lexicon, and everything must agree with the job's distribution.
//!
//! This is a second copy of rules that live in MAGPIE
//! (`ld_get_type_from_lex_name` and `ld_get_type_from_ld_name` in
//! `src/ent/letter_distribution.h`, `lex_lex_compat` / `lex_ld_compat` /
//! `lexicons_and_leaves_compat` in `src/impl/config.c`). The risk of a second
//! copy is drift, so the port is pinned by a table of known-good and known-bad
//! combinations below: a divergence shows up as a failing test rather than as a
//! job that builds here and refuses to load there.
//!
//! The one thing this must not do is guess. A name matching no rule is
//! `Unknown`, and `Unknown` is compatible with nothing -- including itself --
//! so an unrecognised combination is rejected and the table grows.

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LdType {
    English,
    German,
    Norwegian,
    Catalan,
    Polish,
    Dutch,
    French,
    Unknown,
}

fn has_iprefix(prefix: &str, name: &str) -> bool {
    name.len() >= prefix.len() && name[..prefix.len()].eq_ignore_ascii_case(prefix)
}

/// The distribution a lexicon name implies. Mirrors
/// `ld_get_type_from_lex_name`.
pub fn ld_type_from_lexicon(lexicon: &str) -> LdType {
    // MAGPIE takes the base filename first; birdtest's names are already bare,
    // but a name that somehow carries a directory should not silently match on
    // the directory.
    let name = lexicon.rsplit('/').next().unwrap_or(lexicon);
    for prefix in ["CSW", "NWL", "OSPD", "OSW", "TWL", "America", "CEL"] {
        if has_iprefix(prefix, name) {
            return LdType::English;
        }
    }
    if has_iprefix("RD", name) {
        LdType::German
    } else if has_iprefix("NSF", name) {
        LdType::Norwegian
    } else if has_iprefix("DISC", name) {
        LdType::Catalan
    } else if has_iprefix("FRA", name) {
        LdType::French
    } else if has_iprefix("OSPS", name) {
        LdType::Polish
    } else if has_iprefix("DSW", name) {
        LdType::Dutch
    } else {
        LdType::Unknown
    }
}

/// The type a letter-distribution name implies. Mirrors
/// `ld_get_type_from_ld_name`, including that `english_super` is English.
pub fn ld_type_from_distribution(ld_name: &str) -> LdType {
    let name = ld_name.rsplit('/').next().unwrap_or(ld_name);
    if has_iprefix("english", name) {
        LdType::English
    } else if has_iprefix("german", name) {
        LdType::German
    } else if has_iprefix("norwegian", name) {
        LdType::Norwegian
    } else if has_iprefix("catalan", name) {
        LdType::Catalan
    } else if has_iprefix("polish", name) {
        LdType::Polish
    } else if has_iprefix("dutch", name) {
        LdType::Dutch
    } else if has_iprefix("french", name) {
        LdType::French
    } else {
        LdType::Unknown
    }
}

/// `ld_types_compat`: equality, with `Unknown` matching nothing. MAGPIE reaches
/// the same outcome by pushing an error rather than returning a type that
/// compares equal, which is why `Unknown == Unknown` must be false here.
fn types_compat(a: LdType, b: LdType) -> bool {
    a != LdType::Unknown && a == b
}

/// Two lexicon-ish names (a lexicon or a leaves file, which MAGPIE names after
/// its lexicon) belong to the same distribution. Mirrors `lex_lex_compat`.
pub fn lex_lex_compat(a: &str, b: &str) -> bool {
    types_compat(ld_type_from_lexicon(a), ld_type_from_lexicon(b))
}

/// A lexicon and a letter distribution belong together. Mirrors
/// `lex_ld_compat`.
pub fn lex_ld_compat(lexicon: &str, ld_name: &str) -> bool {
    types_compat(ld_type_from_lexicon(lexicon), ld_type_from_distribution(ld_name))
}

/// One player's files, by name, as they will be passed to MAGPIE.
pub struct PlayerFiles<'a> {
    pub label: &'a str,
    pub lexicon: &'a str,
    pub leaves: &'a str,
}

/// The whole check a job must pass, as one error message a person can act on.
///
/// Mirrors `lexicons_and_leaves_compat` (the two players' leaves agree, and
/// each player's lexicon agrees with its own leaves) and adds the job's single
/// letter distribution, which MAGPIE checks separately through `lex_ld_compat`:
/// two players cannot draw from different bags.
pub fn validate_job_files(players: &[PlayerFiles<'_>], ld_name: &str) -> AppResult<()> {
    for player in players {
        if !lex_lex_compat(player.lexicon, player.leaves) {
            return Err(AppError::bad_request(format!(
                "{}: leaves {:?} are not compatible with lexicon {:?}",
                player.label, player.leaves, player.lexicon
            )));
        }
        if !lex_ld_compat(player.lexicon, ld_name) {
            return Err(AppError::bad_request(format!(
                "{}: lexicon {:?} is not compatible with letter distribution {:?}",
                player.label, player.lexicon, ld_name
            )));
        }
    }

    if let [first, rest @ ..] = players {
        for other in rest {
            if !lex_lex_compat(first.lexicon, other.lexicon) {
                return Err(AppError::bad_request(format!(
                    "{} and {} are on lexicons from different letter distributions: {:?} and {:?}",
                    first.label, other.label, first.lexicon, other.lexicon
                )));
            }
            if !lex_lex_compat(first.leaves, other.leaves) {
                return Err(AppError::bad_request(format!(
                    "{} and {} are on leaves from different letter distributions: {:?} and {:?}",
                    first.label, other.label, first.leaves, other.leaves
                )));
            }
        }
    }

    Ok(())
}

/// One player's lexicon and leaves, checked on their own -- the part of
/// `lexicons_and_leaves_compat` that does not need a second player. Used when a
/// player config is created, so an incompatible pairing is refused at the point
/// it is written rather than at the point a job tries to use it.
pub fn validate_lexicon_and_leaves(lexicon: &str, leaves: &str) -> AppResult<()> {
    if !lex_lex_compat(lexicon, leaves) {
        return Err(AppError::bad_request(format!(
            "leaves {leaves:?} are not compatible with lexicon {lexicon:?}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table that pins this port to MAGPIE's rules. Every row is
    /// (lexicon, leaves, letter distribution, is this a job MAGPIE would load).
    /// A divergence between the two implementations fails here rather than at
    /// a contributor's machine.
    const CASES: &[(&str, &str, &str, bool)] = &[
        // Known good: each family with its own distribution.
        ("NWL23", "NWL23", "english", true),
        ("CSW21", "CSW21", "english", true),
        ("TWL06", "TWL06", "english", true),
        ("OSPD5", "OSPD5", "english", true),
        ("OSWEnglish", "OSWEnglish", "english", true),
        ("America", "America", "english", true),
        ("CEL6", "CEL6", "english", true),
        ("RD29", "RD29", "german", true),
        ("NSF23", "NSF23", "norwegian", true),
        ("DISC2", "DISC2", "catalan", true),
        ("FRA24", "FRA24", "french", true),
        ("OSPS50", "OSPS50", "polish", true),
        ("DSW02", "DSW02", "dutch", true),
        // Case-insensitive prefixes, as `has_iprefix`.
        ("nwl23", "NWL23", "ENGLISH", true),
        // english_super is still English.
        ("NWL23", "NWL23", "english_super", true),
        // Leaves named after a different-but-compatible lexicon are fine:
        // MAGPIE compares distribution types, not names.
        ("NWL23", "CSW21", "english", true),
        // Known bad: leaves from another language.
        ("NWL23", "RD29", "english", false),
        // Known bad: lexicon against the wrong distribution.
        ("NWL23", "NWL23", "german", false),
        ("RD29", "RD29", "english", false),
        // Known bad: unrecognised names match nothing, including each other.
        ("MADEUP", "MADEUP", "english", false),
        ("NWL23", "NWL23", "klingon", false),
        ("MADEUP", "MADEUP", "alsomadeup", false),
    ];

    #[test]
    fn single_player_table() {
        for (lexicon, leaves, ld, expected) in CASES {
            let players = [PlayerFiles { label: "player", lexicon, leaves }];
            let ok = validate_job_files(&players, ld).is_ok();
            assert_eq!(ok, *expected, "({lexicon}, {leaves}, {ld})");
        }
    }

    #[test]
    fn two_players_must_agree_with_each_other() {
        let ok = validate_job_files(
            &[
                PlayerFiles { label: "player1", lexicon: "NWL23", leaves: "NWL23" },
                PlayerFiles { label: "player2", lexicon: "CSW21", leaves: "CSW21" },
            ],
            "english",
        );
        assert!(ok.is_ok(), "two English lexicons are a legitimate comparison");

        let err = validate_job_files(
            &[
                PlayerFiles { label: "player1", lexicon: "NWL23", leaves: "NWL23" },
                PlayerFiles { label: "player2", lexicon: "RD29", leaves: "RD29" },
            ],
            "english",
        )
        .unwrap_err();
        assert!(err.message.contains("player2"), "{}", err.message);
    }

    #[test]
    fn unknown_is_compatible_with_nothing_including_itself() {
        // MAGPIE pushes an error rather than returning a type that compares
        // equal, so two unrecognised names must not be judged compatible.
        assert!(!lex_lex_compat("MADEUP", "MADEUP"));
        assert!(!lex_ld_compat("MADEUP", "alsomadeup"));
        assert_eq!(ld_type_from_lexicon("MADEUP"), LdType::Unknown);
        assert_eq!(ld_type_from_distribution("alsomadeup"), LdType::Unknown);
    }
}
