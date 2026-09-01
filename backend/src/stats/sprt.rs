//! Sequential Probability Ratio Test over game (or game-pair) outcomes.
//!
//! The hypotheses are stated in Elo: H0 says the Elo difference is `elo_low`,
//! H1 says it is `elo_high`. We use the standard normal approximation to the
//! log-likelihood ratio used by fishtest: treat each unit's score (1 / 0.5 / 0)
//! as a draw from a distribution with unknown mean, and compare the likelihood
//! of the observed sample mean under the two hypothesised means.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SprtStatus {
    Running,
    /// LLR crossed the upper bound: H1 accepted.
    Passed,
    /// LLR crossed the lower bound: H0 accepted.
    Failed,
    /// Hard cap reached before either bound was crossed.
    TerminatedAtMax,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SprtResult {
    pub llr: f64,
    pub lower_bound: f64,
    pub upper_bound: f64,
    pub status: SprtStatus,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Tally {
    pub wins: u64,
    pub losses: u64,
    pub draws: u64,
}

impl Tally {
    pub fn total(&self) -> u64 {
        self.wins + self.losses + self.draws
    }
}

/// Expected score for a player `elo` points stronger than the opponent.
pub fn expected_score(elo: f64) -> f64 {
    1.0 / (1.0 + 10f64.powf(-elo / 400.0))
}

pub fn bounds(alpha: f64, beta: f64) -> (f64, f64) {
    ((beta / (1.0 - alpha)).ln(), ((1.0 - beta) / alpha).ln())
}

/// Log-likelihood ratio of H1 (`elo_high`) against H0 (`elo_low`).
///
/// Returns 0 for a degenerate sample (no units yet, or zero observed variance,
/// which happens before the first non-unanimous result) — the test simply has
/// not begun to discriminate.
pub fn llr(tally: &Tally, elo_low: f64, elo_high: f64) -> f64 {
    let n = tally.total() as f64;
    if n == 0.0 {
        return 0.0;
    }

    let wins = tally.wins as f64;
    let draws = tally.draws as f64;
    let mean = (wins + 0.5 * draws) / n;
    // Second moment of the per-unit score, which takes values 1, 0.5 and 0.
    let second_moment = (wins + 0.25 * draws) / n;
    let variance = second_moment - mean * mean;
    if variance <= 0.0 {
        return 0.0;
    }

    let mu0 = expected_score(elo_low);
    let mu1 = expected_score(elo_high);
    n * (mu1 - mu0) * (mean - 0.5 * (mu0 + mu1)) / variance
}

/// `units_completed` is deliberately separate from `tally.total()`. For a plain
/// `games` job they are the same number. For `game_pairs` they are not: the
/// job's `min_pairs` / `max_pairs` gates count *pairs played*, while the tally
/// that drives the LLR counts games within the divergent subset, which is a
/// smaller and different number.
pub fn evaluate(
    tally: &Tally,
    units_completed: u64,
    min_units: u64,
    max_units: u64,
    alpha: f64,
    beta: f64,
    elo_low: f64,
    elo_high: f64,
) -> SprtResult {
    let (lower_bound, upper_bound) = bounds(alpha, beta);
    let llr = llr(tally, elo_low, elo_high);
    let n = units_completed;

    // The minimum-units floor exists to stop an early lucky streak from ending
    // the job; below it the LLR is reported but never acted on.
    let status = if n < min_units {
        if n >= max_units {
            SprtStatus::TerminatedAtMax
        } else {
            SprtStatus::Running
        }
    } else if llr >= upper_bound {
        SprtStatus::Passed
    } else if llr <= lower_bound {
        SprtStatus::Failed
    } else if n >= max_units {
        SprtStatus::TerminatedAtMax
    } else {
        SprtStatus::Running
    };

    SprtResult { llr, lower_bound, upper_bound, status }
}

impl SprtStatus {
    pub fn is_finished(self) -> bool {
        !matches!(self, SprtStatus::Running)
    }
}
