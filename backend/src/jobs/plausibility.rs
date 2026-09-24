//! Domain plausibility checks on worker submissions.
//!
//! Every rule here rejects something **impossible** rather than something
//! unusual: a standard deviation below zero, a play scoring negative points, a
//! rack with more tiles than a rack has, a batch reporting more games than the
//! task dispatched. That is the whole design constraint, and it is what makes
//! these safe to enforce at submission time — an honest worker cannot trip
//! them, so there is no false-positive rate to trade against.
//!
//! This is deliberately **not** the population-comparison anomaly detection
//! (per-worker chi-square) that fishnet uses. That approach depends on
//! replication: two honest chess clients analysing the same position at the
//! same depth return the same evaluation, so disagreement is proof. birdtest
//! has no such ground truth. Workers are handed *different* seeds, so no two
//! workers ever play the same games, and the only cross-worker statistic
//! available is the win rate — which is exactly what SPRT is measuring. A test
//! on it cannot separate "this worker is broken" from "these seeds favoured
//! player 2", so it would flag honest contributors at its own alpha rate while
//! missing an attacker biasing results by a percent. See PLAN.md, "Worker
//! Integrity".
//!
//! What is caught here instead is the failure that actually happens in
//! practice: a **broken client**. Those do not produce subtly shifted
//! distributions, they produce garbage — NaN out of an uninitialised buffer, a
//! count that cannot correspond to any game, a truncated batch reported as
//! complete.

use super::handler::{GameAggregate, MoveEntry, PlyStats, RackOccurrence};
use crate::error::{AppError, AppResult};

/// Tiles on a rack. Matches MAGPIE's `RACK_SIZE`; a submission naming more is
/// describing something that cannot be dealt.
const MAX_RACK_TILES: usize = 7;

/// Generous absolute bounds on a per-batch mean score. A word game cannot
/// average a negative score or a four-figure one; these are set far outside
/// any real result rather than at the edge of one, because the job here is to
/// catch garbage, not to police unusual play.
const MIN_SCORE_MEAN: f64 = -100.0;
const MAX_SCORE_MEAN: f64 = 3000.0;

/// The theoretical maximum for a single play is a little over 1,700 points.
/// Rounded up; a play cannot score negative points, and a pass or exchange
/// scores exactly zero.
const MAX_MOVE_SCORE: i32 = 2000;

/// Equity is a score-scale quantity, so it lives in the same order of
/// magnitude as a score plus a leave adjustment.
const MAX_ABS_EQUITY: f64 = 5000.0;

fn finite(value: f64, field: &str) -> AppResult<()> {
    if !value.is_finite() {
        return Err(AppError::bad_request(format!(
            "{field} is not a finite number ({value}) — the client sent uninitialised or \
             corrupted data"
        )));
    }
    Ok(())
}

/// Score moments on a game aggregate.
///
/// The standard-deviation rule is the sharpest one: a negative standard
/// deviation is not an unlikely sample, it is arithmetically impossible, so
/// seeing one means the number did not come from a variance calculation at all.
pub fn check_game_aggregate(aggregate: &GameAggregate, field: &str) -> AppResult<()> {
    for (value, name) in [
        (aggregate.p1_score_mean, "p1_score_mean"),
        (aggregate.p1_score_sd, "p1_score_sd"),
        (aggregate.p2_score_mean, "p2_score_mean"),
        (aggregate.p2_score_sd, "p2_score_sd"),
    ] {
        finite(value, &format!("{field}.{name}"))?;
    }

    if aggregate.p1_score_sd < 0.0 || aggregate.p2_score_sd < 0.0 {
        return Err(AppError::bad_request(format!(
            "{field}: a standard deviation cannot be negative"
        )));
    }

    for (mean, name) in
        [(aggregate.p1_score_mean, "p1_score_mean"), (aggregate.p2_score_mean, "p2_score_mean")]
    {
        if !(MIN_SCORE_MEAN..=MAX_SCORE_MEAN).contains(&mean) {
            return Err(AppError::bad_request(format!(
                "{field}.{name} is {mean}, outside anything a word game can produce"
            )));
        }
    }
    Ok(())
}

/// How many tiles a rack spelled the way MAGPIE spells one holds, or `None`
/// if the spelling is malformed.
///
/// A tile is one character, except that a letter whose name is more than one
/// character is written in brackets (`ld_ml_to_hl`): Catalan's `[L·L]`,
/// `[NY]` and `[QU]` are one tile each. Counted as characters, a full Catalan
/// rack holding one was ten or eleven "tiles", and every captured position of
/// a Catalan games job was refused.
pub fn rack_tiles(rack: &str) -> Option<usize> {
    let mut tiles = 0;
    let mut chars = rack.chars();
    while let Some(c) = chars.next() {
        match c {
            '[' => {
                let mut letter_chars = 0;
                loop {
                    match chars.next()? {
                        ']' => break,
                        '[' => return None,
                        _ => letter_chars += 1,
                    }
                }
                if letter_chars == 0 {
                    return None;
                }
            }
            ']' => return None,
            _ => {}
        }
        tiles += 1;
    }
    Some(tiles)
}

/// A rack as the worker spelled it. Blanks are conventionally `?`, so this
/// counts tiles rather than validating the alphabet — the letter
/// distribution is what decides which letters exist, and that check belongs
/// with the distribution, not here.
pub fn check_rack(rack: &str, context: &str) -> AppResult<()> {
    let tiles = rack_tiles(rack).ok_or_else(|| {
        AppError::bad_request(format!("{context}: rack {rack:?} has an unclosed bracket"))
    })?;
    if tiles == 0 {
        return Err(AppError::bad_request(format!("{context}: empty rack")));
    }
    if tiles > MAX_RACK_TILES {
        return Err(AppError::bad_request(format!(
            "{context}: rack {rack:?} has {tiles} tiles, more than a rack holds"
        )));
    }
    Ok(())
}

/// One ranked move.
///
/// `num_moves` is what the worker says it ranked *before* truncating to the
/// job's reporting cap, so it can legitimately be far larger than the list
/// sent — but never smaller than it, which would mean the worker reported
/// more moves than it claims to have generated.
pub fn check_moves(moves: &[MoveEntry], num_moves: Option<i32>, context: &str) -> AppResult<()> {
    if let Some(num_moves) = num_moves {
        // Negative first: cast to usize it is enormous, and passed.
        if num_moves < 0 || (num_moves as usize) < moves.len() {
            return Err(AppError::bad_request(format!(
                "{context}: reported {} moves but claims only {num_moves} were generated",
                moves.len()
            )));
        }
    }

    for entry in moves {
        if entry.score < 0 || entry.score > MAX_MOVE_SCORE {
            return Err(AppError::bad_request(format!(
                "{context}: play {:?} scores {}, which no play can score",
                entry.play, entry.score
            )));
        }
        finite(entry.equity, &format!("{context}: equity of {:?}", entry.play))?;
        if entry.equity.abs() > MAX_ABS_EQUITY {
            return Err(AppError::bad_request(format!(
                "{context}: play {:?} has an implausible equity {}",
                entry.play, entry.equity
            )));
        }
        // Both are probabilities MAGPIE reports on a 0-100 and 0-1 scale
        // respectively; the shared rule is that a probability is bounded.
        if let Some(win_percentage) = entry.win_percentage {
            finite(win_percentage, &format!("{context}: win_percentage"))?;
            if !(0.0..=100.0).contains(&win_percentage) {
                return Err(AppError::bad_request(format!(
                    "{context}: win percentage {win_percentage} is not a percentage"
                )));
            }
        }
        if let Some(blended) = entry.blended_utility {
            finite(blended, &format!("{context}: blended_utility"))?;
            if !(0.0..=1.0).contains(&blended) {
                return Err(AppError::bad_request(format!(
                    "{context}: blended utility {blended} is outside [0, 1]"
                )));
            }
        }
        check_plies(&entry.plies, &format!("{context}: play {:?}", entry.play))?;
    }
    Ok(())
}

/// A simulated play's per-ply statistics. MAGPIE numbers the plies from 0 in
/// order, so a ply out of order or repeated is a broken client; a percentage
/// outside [0, 100] or a mean score no play can reach is not a statistic.
/// How many are *kept* is the player config's `num_plies_recorded`, applied
/// where they are stored.
fn check_plies(plies: &[PlyStats], context: &str) -> AppResult<()> {
    let mut previous: Option<i16> = None;
    for ply in plies {
        if ply.ply < 0 || previous.is_some_and(|p| ply.ply <= p) {
            return Err(AppError::bad_request(format!(
                "{context}: ply {} is out of order; plies are numbered from 0, ascending",
                ply.ply
            )));
        }
        previous = Some(ply.ply);
        finite(ply.bingo_percentage, &format!("{context}: bingo_percentage"))?;
        if !(0.0..=100.0).contains(&ply.bingo_percentage) {
            return Err(AppError::bad_request(format!(
                "{context}: bingo percentage {} at ply {} is not a percentage",
                ply.bingo_percentage, ply.ply
            )));
        }
        finite(ply.average_score, &format!("{context}: average_score"))?;
        if !(0.0..=f64::from(MAX_MOVE_SCORE)).contains(&ply.average_score) {
            return Err(AppError::bad_request(format!(
                "{context}: average score {} at ply {} is one no play can score",
                ply.average_score, ply.ply
            )));
        }
    }
    Ok(())
}

/// Rack occurrences from a leave-generation batch.
///
/// The duplicate rule is the one that matters. The server folds these into
/// `leave_rack_progress` with an upsert that *adds* occurrences, so a rack
/// listed twice in one submission is counted twice — a generation would then
/// reach its target on inflated coverage and close early. MAGPIE reports each
/// rack once, so a duplicate is a broken client, and it is silent damage
/// rather than a visible error, which is exactly why it is worth rejecting.
pub fn check_rack_occurrences(racks: &[RackOccurrence]) -> AppResult<()> {
    let mut seen = std::collections::HashSet::with_capacity(racks.len());
    for occurrence in racks {
        check_rack(&occurrence.rack, "leave result")?;
        // Leave generation observes full racks only. Anything shorter would
        // name no row of the generation's rack universe.
        let tiles = rack_tiles(&occurrence.rack).unwrap_or(0);
        if tiles != MAX_RACK_TILES {
            return Err(AppError::bad_request(format!(
                "leave result: rack {:?} has {tiles} tiles; leave generation reports full \
                 racks of {MAX_RACK_TILES}",
                occurrence.rack
            )));
        }
        if occurrence.count < 1 {
            return Err(AppError::bad_request(format!(
                "leave result: rack {:?} reports {} occurrences; a rack that did not occur \
                 should not be reported",
                occurrence.rack, occurrence.count
            )));
        }
        finite(occurrence.mean, &format!("leave result: mean equity of {:?}", occurrence.rack))?;
        if occurrence.mean.abs() > MAX_ABS_EQUITY {
            return Err(AppError::bad_request(format!(
                "leave result: rack {:?} has an implausible mean equity {}",
                occurrence.rack, occurrence.mean
            )));
        }
        if !seen.insert(occurrence.rack.as_str()) {
            return Err(AppError::bad_request(format!(
                "leave result: rack {:?} appears more than once; occurrences are summed on \
                 receipt, so a duplicate would inflate the generation's coverage",
                occurrence.rack
            )));
        }
    }
    Ok(())
}

/// The most rack occurrences one game can report. MAGPIE's leave generation
/// records the rack of the player on turn, and on a turn where it forces a rare
/// rack, that rack as well: two a turn. At the 400-turn ceiling a captured
/// position's `turn_number` is held to (`game::MAX_TURNS_PER_GAME`), that is
/// 800, rounded up. A real game is some twenty-odd turns.
const MAX_RACK_OCCURRENCES_PER_GAME: i64 = 1000;

/// A leave batch cannot report more rack occurrences than its games drew
/// racks.
///
/// [`check_rack_occurrences`] bounds a count from below and nothing bounded it
/// from above, and this is the one job type where that is not only wrong data
/// but a wedge. Occurrences are **summed** -- into `bigint` columns, by a merge
/// that folds every staged result of the generation in one statement. A count
/// out of an uninitialised buffer is as likely to be near 2^63 as anywhere:
/// staged, it made that statement fail with `bigint out of range`, every time,
/// for every merge of the generation -- the periodic one, the one a claim asks
/// for, and the drain a transition will not close without. The generation could
/// never close and nothing short of deleting the staged row by hand fixed it.
/// A smaller lie was quieter and as permanent: a rack reported a million times
/// is at target for good, on coverage nobody played, and a fold cannot be
/// subtracted back out.
///
/// Like the batch-size rule it needs the task -- the games it was dispatched
/// with -- so it runs from `registry::store_result`. The bound is on the total,
/// which bounds every count in it.
pub fn check_rack_occurrence_total(racks: &[RackOccurrence], num_games: i32) -> AppResult<()> {
    let ceiling = i64::from(num_games.max(0)).saturating_mul(MAX_RACK_OCCURRENCES_PER_GAME);
    let mut total: i64 = 0;
    for occurrence in racks {
        total = total.saturating_add(occurrence.count);
        if total > ceiling {
            return Err(AppError::bad_request(format!(
                "leave result: more than {ceiling} rack occurrences reported for a task of \
                 {num_games} games; a game cannot draw that many racks"
            )));
        }
    }
    Ok(())
}

/// Checks a game batch against the size the task was dispatched with.
///
/// The size rule needs the job, which `process_response` does not have, so it
/// runs from `registry::store_result` instead. It is the strongest check
/// available at submission time and the only one that catches a worker
/// reporting work it did not do: the batch size was fixed when the task was
/// dispatched, so a result of any other size is answering a question nobody
/// asked. `expected_games` is the job's batch size -- doubled for pairs, which
/// play two games each -- which every request of the job denormalizes and
/// which the job's template carries, so no request row is read for it.
/// The games a game task was dispatched to play, which is what
/// [`check_batch_size`] holds its result to: the job's batch size, which counts
/// pairs -- two games each -- when `game_pairs` is set, as on the request.
pub fn games_dispatched(batch_size: i32, game_pairs: bool) -> i32 {
    if game_pairs {
        batch_size.saturating_mul(2)
    } else {
        batch_size
    }
}

pub fn check_batch_size(reported_games: i32, expected_games: i32) -> AppResult<()> {
    if reported_games != expected_games {
        return Err(AppError::bad_request(format!(
            "result reports {reported_games} games but this task dispatched {expected_games}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aggregate() -> GameAggregate {
        GameAggregate {
            games: 10,
            wins: 5,
            losses: 5,
            ties: 0,
            p1_score_mean: 420.0,
            p1_score_sd: 60.0,
            p2_score_mean: 415.0,
            p2_score_sd: 58.0,
        }
    }

    #[test]
    fn an_ordinary_aggregate_passes() {
        assert!(check_game_aggregate(&aggregate(), "all_games").is_ok());
    }

    #[test]
    fn nan_is_rejected() {
        let mut bad = aggregate();
        bad.p1_score_mean = f64::NAN;
        assert!(check_game_aggregate(&bad, "all_games").is_err());
    }

    #[test]
    fn infinity_is_rejected() {
        let mut bad = aggregate();
        bad.p2_score_sd = f64::INFINITY;
        assert!(check_game_aggregate(&bad, "all_games").is_err());
    }

    /// Arithmetically impossible, so it cannot have come from a variance
    /// calculation at all.
    #[test]
    fn a_negative_standard_deviation_is_rejected() {
        let mut bad = aggregate();
        bad.p1_score_sd = -1.0;
        assert!(check_game_aggregate(&bad, "all_games").is_err());
    }

    #[test]
    fn absurd_score_means_are_rejected_but_real_ones_are_not() {
        let mut bad = aggregate();
        bad.p1_score_mean = 50_000.0;
        assert!(check_game_aggregate(&bad, "all_games").is_err());

        // The bounds must not clip real play: a blowout and a grind both pass.
        for mean in [180.0, 700.0] {
            let mut ok = aggregate();
            ok.p1_score_mean = mean;
            assert!(check_game_aggregate(&ok, "all_games").is_ok(), "rejected mean {mean}");
        }
    }

    #[test]
    fn racks_are_bounded_by_what_a_rack_holds() {
        assert!(check_rack("AEINRST", "x").is_ok());
        assert!(check_rack("AE?", "x").is_ok(), "blanks are legal tiles");
        assert!(check_rack("", "x").is_err());
        assert!(check_rack("AEINRSTU", "x").is_err());
        // A multi-character letter is one tile, bracketed as MAGPIE writes it.
        assert!(check_rack("A[L·L]E[NY]I[QU]S", "x").is_ok(), "seven Catalan tiles");
        assert!(check_rack("A[L·L]E[NY]I[QU]ST", "x").is_err(), "eight Catalan tiles");
        assert_eq!(rack_tiles("[L·L][L·L]"), Some(2));
        assert_eq!(rack_tiles("A[NY"), None);
        assert_eq!(rack_tiles("A]"), None);
        assert_eq!(rack_tiles("[]"), None);
    }

    fn move_entry(score: i32, equity: f64) -> MoveEntry {
        MoveEntry {
            play: "8D WORD".into(),
            score,
            equity,
            win_percentage: None,
            blended_utility: None,
            plies: Vec::new(),
        }
    }

    #[test]
    fn impossible_move_scores_are_rejected() {
        assert!(check_moves(&[move_entry(74, 80.0)], None, "x").is_ok());
        assert!(check_moves(&[move_entry(0, -12.0)], None, "x").is_ok(), "a pass scores zero");
        assert!(check_moves(&[move_entry(-5, 0.0)], None, "x").is_err());
        assert!(check_moves(&[move_entry(9_000, 0.0)], None, "x").is_err());
        assert!(check_moves(&[move_entry(30, f64::NAN)], None, "x").is_err());
    }

    #[test]
    fn a_worker_cannot_report_more_moves_than_it_generated() {
        let moves = vec![move_entry(30, 30.0), move_entry(20, 20.0)];
        assert!(check_moves(&moves, Some(50), "x").is_ok(), "truncation is normal");
        assert!(check_moves(&moves, Some(2), "x").is_ok());
        assert!(check_moves(&moves, Some(1), "x").is_err());
        // Cast to usize, -1 was larger than any list and passed.
        assert!(check_moves(&moves, Some(-1), "x").is_err());
    }

    fn ply(ply: i16, bingo_percentage: f64, average_score: f64) -> PlyStats {
        PlyStats { ply, bingo_percentage, average_score }
    }

    #[test]
    fn per_ply_statistics_are_statistics() {
        let with = |plies: Vec<PlyStats>| {
            let mut entry = move_entry(30, 30.0);
            entry.plies = plies;
            check_moves(&[entry], None, "x")
        };
        assert!(with(vec![ply(0, 12.5, 31.0), ply(1, 20.0, 35.2)]).is_ok());
        assert!(with(vec![ply(0, 140.0, 31.0)]).is_err(), "not a percentage");
        assert!(with(vec![ply(0, 12.5, -3.0)]).is_err(), "no play scores negative");
        assert!(with(vec![ply(0, f64::NAN, 3.0)]).is_err());
        assert!(with(vec![ply(-1, 12.5, 31.0)]).is_err());
        assert!(with(vec![ply(1, 12.5, 31.0), ply(0, 12.5, 31.0)]).is_err(), "out of order");
        assert!(with(vec![ply(0, 12.5, 31.0), ply(0, 12.5, 31.0)]).is_err(), "repeated");
    }

    #[test]
    fn probabilities_must_be_probabilities() {
        let mut entry = move_entry(30, 30.0);
        entry.win_percentage = Some(140.0);
        assert!(check_moves(&[entry], None, "x").is_err());

        let mut entry = move_entry(30, 30.0);
        entry.blended_utility = Some(-0.2);
        assert!(check_moves(&[entry], None, "x").is_err());
    }

    fn occurrence(rack: &str, count: i64) -> RackOccurrence {
        RackOccurrence { rack: rack.into(), count, mean: 12.5 }
    }

    /// The damaging one: occurrences are summed on receipt, so a duplicate
    /// silently inflates a generation's coverage rather than erroring.
    #[test]
    fn duplicate_racks_are_rejected() {
        assert!(check_rack_occurrences(&[occurrence("AEINRST", 3)]).is_ok());
        assert!(
            check_rack_occurrences(&[occurrence("AEINRST", 3), occurrence("AEINRST", 1)]).is_err()
        );
    }

    #[test]
    fn leave_results_report_full_racks_only() {
        assert!(check_rack_occurrences(&[occurrence("AEINRS", 3)]).is_err());
        assert!(check_rack_occurrences(&[occurrence("?", 3)]).is_err());
    }

    #[test]
    fn a_rack_that_did_not_occur_is_rejected() {
        assert!(check_rack_occurrences(&[occurrence("AEINRST", 0)]).is_err());
    }

    /// Occurrences are summed into `bigint` columns by one statement per
    /// generation, so a count no game could produce is not only wrong: near
    /// 2^63 it overflows that statement on every merge, and the generation can
    /// never close.
    #[test]
    fn a_leave_batch_cannot_report_more_occurrences_than_its_games_drew() {
        // Twenty-odd turns a game, two racks a turn at most: an honest
        // hundred-game task is a few thousand occurrences.
        let honest: Vec<_> = (0..500).map(|i| occurrence(&format!("R{i:06}"), 9)).collect();
        assert!(check_rack_occurrence_total(&honest, 100).is_ok());
        assert!(check_rack_occurrence_total(&[occurrence("AEINRST", 100_000)], 100).is_ok());

        assert!(check_rack_occurrence_total(&[occurrence("AEINRST", 100_001)], 100).is_err());
        assert!(check_rack_occurrence_total(&[occurrence("AEINRST", i64::MAX)], 100).is_err());
        // The total, not each count: and adding them up must not itself wrap.
        let garbage = [occurrence("AEINRST", i64::MAX), occurrence("AEINRSU", i64::MAX)];
        assert!(check_rack_occurrence_total(&garbage, 100).is_err());
        let spread: Vec<_> = (0..101).map(|i| occurrence(&format!("R{i:06}"), 1_000)).collect();
        assert!(check_rack_occurrence_total(&spread, 100).is_err());
    }

    /// U-PLAUS-1: a pairs job's batch size counts pairs, so the games it
    /// dispatched are twice that; a games job's batch is already games. The
    /// confusion the `min_pairs`/`max_pairs` naming exists to prevent.
    /// (TESTING.md calls this `check_against_task`; the rule is
    /// `check_batch_size` against `games_dispatched`, which `registry::
    /// store_result` runs for both job types.)
    #[test]
    fn a_pairs_batch_dispatches_two_games_a_pair_and_a_games_batch_does_not() {
        assert_eq!(games_dispatched(10, true), 20);
        assert_eq!(games_dispatched(10, false), 10);

        // Ten pairs are answered with twenty games, never ten.
        assert!(check_batch_size(20, games_dispatched(10, true)).is_ok());
        assert!(check_batch_size(10, games_dispatched(10, true)).is_err());
        // Ten games are answered with ten, never twenty.
        assert!(check_batch_size(10, games_dispatched(10, false)).is_ok());
        assert!(check_batch_size(20, games_dispatched(10, false)).is_err());

        // A batch size at the top of i32 cannot wrap into a small target.
        assert_eq!(games_dispatched(i32::MAX, true), i32::MAX);
    }

    /// U-PLAUS-2: exactly the dispatched count passes; one more or one fewer,
    /// for either job type, is refused and says why.
    #[test]
    fn a_batch_one_game_off_in_either_direction_is_rejected() {
        for (batch, pairs) in [(10, false), (10, true), (1, false), (1, true)] {
            let dispatched = games_dispatched(batch, pairs);
            assert!(check_batch_size(dispatched, dispatched).is_ok());
            for reported in [dispatched - 1, dispatched + 1] {
                let error = check_batch_size(reported, dispatched).unwrap_err();
                assert_eq!(error.status, axum::http::StatusCode::BAD_REQUEST);
                assert!(
                    error.message.contains(&format!(
                        "reports {reported} games but this task dispatched {dispatched}"
                    )),
                    "{}",
                    error.message
                );
            }
        }
    }
}

/// Captured `worker/fake_worker.py` submissions, checked against what the
/// server runs on a submission before storing it.
///
/// The fake worker is the only client below tier 6, so a shape drift there is
/// invisible until a job of that type is actually run: its opening-rack
/// submission once read a `position` field the request does not have and
/// answered with a bare `{"moves": [...]}`, which no opening-rack job could
/// ever accept. Every fixture here is the worker's own output, never written
/// by hand: `testdata/README.md` has the command that regenerates each one,
/// and each test repeats its own.
#[cfg(test)]
mod fixture_tests {
    use super::super::game::GameHandler;
    use super::super::game_pair::GamePairHandler;
    use super::super::handler::{
        GameResultsRecord, JobHandler, LeaveRecord, LeaveResponse, PositionAnalysisResponse,
    };
    use super::super::leave_gen::LeaveGenHandler;
    use super::super::opening_rack::OpeningRackHandler;
    use super::{check_batch_size, check_rack_occurrence_total, games_dispatched};
    use crate::error::{AppError, AppResult};
    use crate::stats::sprt::Pentanomial;
    use serde_json::Value;

    const GAMES: &str = include_str!("testdata/fake_worker_games.json");
    const GAMES_CAPTURED: &str = include_str!("testdata/fake_worker_games_captured.json");
    const GAME_PAIRS: &str = include_str!("testdata/fake_worker_game_pairs.json");
    const OPENING_RACK: &str = include_str!("testdata/fake_worker_opening_rack.json");
    const LEAVE_GENERATION: &str = include_str!("testdata/fake_worker_leave_generation.json");
    const STALE: &str = include_str!("testdata/fake_worker_stale.json");
    const ABANDON: &str = include_str!("testdata/fake_worker_abandon.json");
    const MALFORMED: [(&str, &str); 4] = [
        ("games", include_str!("testdata/fake_worker_malformed_games.json")),
        ("game_pairs", include_str!("testdata/fake_worker_malformed_game_pairs.json")),
        ("opening_rack", include_str!("testdata/fake_worker_malformed_opening_rack.json")),
        (
            "leave_generation",
            include_str!("testdata/fake_worker_malformed_leave_generation.json"),
        ),
    ];

    /// The batch every fixture was captured against: the contract
    /// assignments' `num_games` (10 games, or 10 pairs, and 10,000 games for
    /// leave generation), which is the job setting `store_result` reads.
    const BATCH: i32 = 10;
    const LEAVE_GAMES: i32 = 10_000;

    fn decode<T: serde::de::DeserializeOwned>(payload: Value) -> AppResult<T> {
        serde_json::from_value(payload)
            .map_err(|e| AppError::bad_request(format!("malformed task response: {e}")))
    }

    // What `registry::store_result` runs on a submission before it stores it,
    // one job type each, in its order: decode, `process_response`, then the
    // checks against the task. Everything short of the database -- which for
    // an opening-rack batch is also `check_batch_against_task`, comparing the
    // racks with the task's range.

    fn games(payload: Value, batch: i32) -> AppResult<GameResultsRecord> {
        let record = GameHandler::process_response(decode(payload)?)?;
        check_batch_size(record.all_games.games, games_dispatched(batch, false))?;
        Ok(record)
    }

    fn game_pairs(payload: Value, batch: i32) -> AppResult<GameResultsRecord> {
        let record = GamePairHandler::process_response(decode(payload)?)?;
        check_batch_size(record.all_games.games, games_dispatched(batch, true))?;
        Ok(record)
    }

    fn leave_generation(payload: Value, num_games: i32) -> AppResult<LeaveRecord> {
        let record = LeaveGenHandler::process_response(decode(payload)?)?;
        check_rack_occurrence_total(&record.racks, num_games)?;
        Ok(record)
    }

    fn validate(job_type: &str, payload: Value) -> AppResult<()> {
        match job_type {
            "games" => games(payload, BATCH).map(drop),
            "game_pairs" => game_pairs(payload, BATCH).map(drop),
            "opening_rack" => OpeningRackHandler::process_response(decode(payload)?).map(drop),
            "leave_generation" => leave_generation(payload, LEAVE_GAMES).map(drop),
            other => panic!("no job type {other}"),
        }
    }

    fn json(fixture: &str) -> Value {
        serde_json::from_str(fixture).expect("a fixture is JSON")
    }

    /// U-FAKE-1: a `games` submission, plain and with captured positions.
    ///
    /// Regenerate with:
    /// `python3 worker/fake_worker.py --emit-fixture contract-fixtures/assignment-games.json`
    /// and, for the captured one, the same with
    /// `--override capture_positions=true --override num_games=1`.
    #[test]
    fn the_fake_workers_games_submission_passes_validation() {
        let record = games(json(GAMES), BATCH).unwrap();
        assert_eq!(record.all_games.games, BATCH);
        assert!(record.pentanomial.is_none() && record.positions.is_empty());

        let record = games(json(GAMES_CAPTURED), 1).unwrap();
        assert!(record.positions.len() >= 18, "about twenty turns a game");
        assert!(record.positions.iter().all(|p| p.game_index == Some(0) && !p.moves.is_empty()));
        assert!(record.positions[0].previous_move.is_none(), "turn 0 follows nothing");
        assert!(record.positions[1].previous_move.is_some());
    }

    /// U-FAKE-2: a `game_pairs` submission, including the pentanomial
    /// cross-checks -- which the fixture is shown to be exercising, not
    /// skipping, by breaking each one.
    ///
    /// Regenerate with:
    /// `python3 worker/fake_worker.py --emit-fixture contract-fixtures/assignment-games.json
    /// --override job_type='"game_pairs"' --override game_pairs=true`
    #[test]
    fn the_fake_workers_game_pairs_submission_passes_the_pentanomial_cross_checks() {
        let record = game_pairs(json(GAME_PAIRS), BATCH).unwrap();
        let pentanomial = record.pentanomial.expect("a pairs result carries a pentanomial");
        let counts = Pentanomial { counts: pentanomial.map(|c| c as u64) };
        assert_eq!(counts.pairs(), BATCH as u64);
        assert_eq!(counts.pairs() * 2, record.all_games.games as u64);
        assert_eq!(
            counts.half_points(),
            2 * record.all_games.wins as u64 + record.all_games.ties as u64
        );
        let divergent = record.divergent_games.expect("the fake worker reports the subset");
        assert!(divergent.games % 2 == 0 && divergent.games <= record.all_games.games);

        // One split pair turned into a 3-1: the pair count still agrees, player
        // 1's half-points no longer do.
        assert!(pentanomial[2] > 0, "the fixture should have a split pair to move");
        let mut shifted = json(GAME_PAIRS);
        shifted["pentanomial"][2] = (pentanomial[2] - 1).into();
        shifted["pentanomial"][3] = (pentanomial[3] + 1).into();
        let error = game_pairs(shifted, BATCH).unwrap_err();
        assert!(error.message.contains("about player 1's score"), "{}", error.message);

        // One pair too many, in the bucket worth no half-points: only the pair
        // count disagrees.
        let mut extra = json(GAME_PAIRS);
        extra["pentanomial"][0] = (pentanomial[0] + 1).into();
        let error = game_pairs(extra, BATCH).unwrap_err();
        assert!(error.message.contains("exactly half the games"), "{}", error.message);
    }

    /// U-FAKE-3: an `opening_rack` submission answers every requested rack.
    ///
    /// Regenerate with:
    /// `python3 worker/fake_worker.py --emit-fixture contract-fixtures/assignment-opening-rack.json`
    #[test]
    fn the_fake_worker_speaks_the_opening_rack_response_shape() {
        let response: PositionAnalysisResponse =
            serde_json::from_str(OPENING_RACK).expect("should deserialize");
        let racks: Vec<&str> = response.racks.iter().map(|r| r.rack.as_str()).collect();
        // The racks of the assignment it answered, in order.
        assert_eq!(racks, ["AEINRST", "?AEILNT", "AABBCDE"]);
        assert!(response.racks.iter().all(|r| !r.moves.is_empty() && r.num_moves.is_some()));
        // And it must satisfy the plausibility rules it will be checked against.
        let record = OpeningRackHandler::process_response(response).unwrap();
        assert_eq!(record.positions.len(), 3);
    }

    /// U-FAKE-4: a `leave_generation` submission passes the occurrence rules,
    /// both the per-rack ones and the total against the task's games.
    ///
    /// Regenerate with:
    /// `python3 worker/fake_worker.py --emit-fixture contract-fixtures/assignment-leave-generation.json`
    #[test]
    fn the_fake_workers_leave_submission_passes_the_occurrence_rules() {
        let response: LeaveResponse = serde_json::from_str(LEAVE_GENERATION).unwrap();
        super::check_rack_occurrences(&response.racks).unwrap();
        let record = leave_generation(json(LEAVE_GENERATION), LEAVE_GAMES).unwrap();
        // It reports the forced racks it was sent.
        let racks: Vec<&str> = record.racks.iter().map(|r| r.rack.as_str()).collect();
        assert_eq!(racks, ["AEINRST", "?AEGLNT"]);
    }

    /// U-FAKE-5, `--mode malformed`: every corruption, against every job
    /// type, is refused by the server's validation, each by the rule it was
    /// written to break. `odd_pair_count` is a consistent tally of eleven
    /// games, so for a plain games job it is the batch-size rule that catches
    /// it; the variants that only make sense for a game result fall back to a
    /// body with no `racks` for the other two types.
    ///
    /// Regenerate with, for each assignment (`game_pairs` from the games one,
    /// with the two `--override`s above):
    /// `python3 worker/fake_worker.py --mode malformed --emit-fixture
    /// contract-fixtures/assignment-<type>.json`
    #[test]
    fn every_malformed_submission_is_rejected_for_the_rule_it_breaks() {
        const DECODE: &str = "malformed task response";
        let expected: [(&str, [(&str, &str); 5]); 4] = [
            (
                "games",
                [
                    ("wrong_type", DECODE),
                    ("missing_field", "missing field `wins`"),
                    ("inconsistent_counts", "wins + losses + ties must equal games"),
                    ("odd_pair_count", "reports 11 games but this task dispatched 10"),
                    ("empty", "missing field `all_games`"),
                ],
            ),
            (
                "game_pairs",
                [
                    ("wrong_type", DECODE),
                    ("missing_field", "missing field `wins`"),
                    ("inconsistent_counts", "wins + losses + ties must equal games"),
                    ("odd_pair_count", "an even, non-zero number of games"),
                    ("empty", "missing field `all_games`"),
                ],
            ),
            (
                "opening_rack",
                [
                    ("wrong_type", DECODE),
                    ("missing_field", "missing field `racks`"),
                    ("inconsistent_counts", "missing field `racks`"),
                    ("odd_pair_count", "missing field `racks`"),
                    ("empty", "missing field `racks`"),
                ],
            ),
            (
                "leave_generation",
                [
                    ("wrong_type", DECODE),
                    ("missing_field", "missing field `racks`"),
                    ("inconsistent_counts", "missing field `racks`"),
                    ("odd_pair_count", "missing field `racks`"),
                    ("empty", "missing field `racks`"),
                ],
            ),
        ];

        for ((job_type, fixture), (expected_type, reasons)) in MALFORMED.iter().zip(expected) {
            assert_eq!(*job_type, expected_type);
            let variants = json(fixture);
            let variants = variants.as_object().unwrap();
            assert_eq!(variants.len(), reasons.len(), "{job_type}: a corruption is missing");
            for (variant, reason) in reasons {
                let payload = variants[variant].clone();
                let error = validate(job_type, payload)
                    .expect_err("a malformed submission must be rejected");
                assert_eq!(error.status, axum::http::StatusCode::BAD_REQUEST);
                assert!(
                    error.message.contains(reason),
                    "{job_type}/{variant}: {}",
                    error.message
                );
            }
        }
    }

    /// U-FAKE-5, `--mode stale`: the result is a perfectly valid one -- so
    /// what the server refuses is the token alone -- under a claim token that
    /// is not the one the assignment issued. What the server then does with a
    /// token it never issued (`accepted: false`, nothing stored) needs the
    /// database, and is `tests/fake_worker.rs`, which posts this fixture.
    ///
    /// Regenerate with:
    /// `python3 worker/fake_worker.py --mode stale --emit-fixture contract-fixtures/assignment-games.json`
    #[test]
    fn a_stale_submission_differs_from_a_valid_one_only_in_its_token() {
        let body = json(STALE);
        let assignment: Value =
            serde_json::from_str(include_str!("../../../contract-fixtures/assignment-games.json"))
                .unwrap();
        let issued = assignment["claim_token"].as_str().unwrap();
        let token = body["claim_token"].as_str().unwrap();
        // A well-formed UUID, so the route reaches the claim lookup instead of
        // failing to parse the body -- and not the one that was issued.
        assert!(uuid::Uuid::parse_str(token).is_ok(), "{token}");
        assert_ne!(token, issued);
        games(body["result"].clone(), BATCH).unwrap();
    }

    /// U-FAKE-5, `--mode abandon`: the worker submits nothing at all, so the
    /// only server-side outcome is the heartbeat timeout reclaiming the claim
    /// (`worker_api::a_claim_still_silent_after_the_grace_is_reclaimed`).
    ///
    /// Regenerate with:
    /// `python3 worker/fake_worker.py --mode abandon --emit-fixture contract-fixtures/assignment-games.json`
    #[test]
    fn an_abandoning_worker_submits_nothing() {
        assert_eq!(json(ABANDON), Value::Null);
    }
}
