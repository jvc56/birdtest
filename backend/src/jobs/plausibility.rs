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

use super::handler::{GameAggregate, MoveEntry, RackOccurrence};
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

/// A rack as the worker spelled it. Blanks are conventionally `?`, so this
/// counts characters rather than validating the alphabet — the letter
/// distribution is what decides which letters exist, and that check belongs
/// with the distribution, not here.
pub fn check_rack(rack: &str, context: &str) -> AppResult<()> {
    let tiles = rack.chars().count();
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
        if (num_moves as usize) < moves.len() {
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
        let tiles = occurrence.rack.chars().count();
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
}

#[cfg(test)]
mod fixture_tests {
    use super::super::handler::PositionAnalysisResponse;

    /// A real `fake_worker.py` opening-rack submission, captured verbatim.
    ///
    /// The fake worker previously read a `position` field the request does not
    /// have and answered with a bare `{"moves": [...]}`, which no opening-rack
    /// job could ever accept. Pinning one of its submissions here means the two
    /// sides cannot drift apart again without a test failing.
    const OPENING_RACK_SUBMISSION: &str = include_str!("testdata/fake_worker_opening_rack.json");

    #[test]
    fn the_fake_worker_speaks_the_opening_rack_response_shape() {
        let response: PositionAnalysisResponse =
            serde_json::from_str(OPENING_RACK_SUBMISSION).expect("should deserialize");
        assert_eq!(response.racks.len(), 2);
        assert_eq!(response.racks[0].rack, "AEINRST");
        assert!(!response.racks[0].moves.is_empty());
        // And it must satisfy the plausibility rules it will be checked against.
        for analysis in &response.racks {
            super::check_rack(&analysis.rack, "fixture").unwrap();
            super::check_moves(&analysis.moves, None, "fixture").unwrap();
        }
    }
}
