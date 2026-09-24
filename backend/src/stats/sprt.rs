//! Sequential Probability Ratio Test over game (or game-pair) outcomes.
//!
//! The hypotheses are stated in Elo: H0 says the Elo difference is `elo_low`,
//! H1 says it is `elo_high`. We use the standard normal approximation to the
//! log-likelihood ratio used by fishtest: treat each unit's score as a draw
//! from a distribution with unknown mean, and compare the likelihood of the
//! observed sample mean under the two hypothesised means.
//!
//! What the *unit* is differs by job type, and that choice is the whole
//! statistical content of this module:
//!
//! * A plain `games` job observes one game at a time, scored 1 / 0.5 / 0.
//! * A `game_pairs` job observes one **pair** at a time — two games sharing a
//!   seed with the players swapped — scored 0, 0.25, 0.5, 0.75 or 1 (the
//!   pair's half-point total over 4). This is the pentanomial model, and it is
//!   where paired play's variance reduction actually comes from: the noise
//!   common to the two games cancels within the pair.
//!
//! Every completed pair belongs in the sample, including pairs whose two games
//! played identically. Those are guaranteed 1-1 ties, they score exactly 0.5,
//! and they pull the variance down — which is the benefit, not something to be
//! filtered out. Testing only the pairs that *did* diverge conditions the
//! sample on its own outcome. Filtering does not flip the direction — the
//! identical pairs contribute exactly 0.5 each, so both views land on the same
//! side of even — but it destroys the magnitude and with it the test's whole
//! purpose. Take 20,000 games split 9,950-10,050, of which only 100 diverged
//! and one player took 99: over every pair that is a score rate of 0.4975, or
//! about -1.7 Elo. Over the divergent games alone it is 0.01, or about -800
//! Elo. A test fed the second number crosses any boundary it is given, almost
//! immediately, on a hundredth of the evidence — so an SPRT configured to
//! distinguish ±10 Elo stops being able to distinguish anything at all.

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

/// Per-game outcome counts. The unit of a plain `games` job, and the shape the
/// dashboard reports for both job types.
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

/// Counts of completed pairs by player 1's half-point score across the pair:
/// index 0 is "lost both", 2 is "split" (where every identically-played pair
/// lands), 4 is "won both". Produced by MAGPIE's `-gp` mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Pentanomial {
    pub counts: [u64; 5],
}

impl Pentanomial {
    pub fn pairs(&self) -> u64 {
        self.counts.iter().sum()
    }

    /// Player 1's total half-points across every pair. Two views of the same
    /// games must agree on this: it equals `2 * wins + draws` of the per-game
    /// tally over the same pairs, which is what makes the two cross-checkable
    /// at submission time.
    pub fn half_points(&self) -> u64 {
        self.counts.iter().enumerate().map(|(i, c)| i as u64 * c).sum()
    }
}

/// A sample reduced to what the LLR needs: how many independent observations,
/// their mean score, and the variance of that score. Keeping this separate from
/// how the counts were gathered is what lets games and pairs share one test.
#[derive(Debug, Clone, Copy)]
pub struct Sample {
    pub n: u64,
    pub mean: f64,
    pub variance: f64,
}

impl Sample {
    /// One game per observation, scored 1 / 0.5 / 0.
    pub fn from_games(tally: &Tally) -> Self {
        let n = tally.total();
        if n == 0 {
            return Self { n: 0, mean: 0.0, variance: 0.0 };
        }
        let n_f = n as f64;
        let (wins, draws) = (tally.wins as f64, tally.draws as f64);
        let mean = (wins + 0.5 * draws) / n_f;
        // Second moment of a score taking values 1, 0.5 and 0.
        let second_moment = (wins + 0.25 * draws) / n_f;
        Self { n, mean, variance: second_moment - mean * mean }
    }

    /// One **pair** per observation, scored `i / 4` for bucket `i`. The mean is
    /// therefore on the same per-game scale as `from_games` — a pair scoring
    /// 0.5 is one game's worth each way — so it is directly comparable against
    /// `expected_score(elo)`, while `n` counts pairs.
    pub fn from_pentanomial(pentanomial: &Pentanomial) -> Self {
        let n = pentanomial.pairs();
        if n == 0 {
            return Self { n: 0, mean: 0.0, variance: 0.0 };
        }
        let n_f = n as f64;
        let mut mean = 0.0;
        let mut second_moment = 0.0;
        for (i, &count) in pentanomial.counts.iter().enumerate() {
            let score = i as f64 / 4.0;
            let weight = count as f64 / n_f;
            mean += score * weight;
            second_moment += score * score * weight;
        }
        Self { n, mean, variance: second_moment - mean * mean }
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
/// Returns 0 for a degenerate sample (no observations yet, or zero observed
/// variance, which happens before the first non-unanimous result) — the test
/// simply has not begun to discriminate.
pub fn llr(sample: &Sample, elo_low: f64, elo_high: f64) -> f64 {
    if sample.n == 0 || sample.variance <= 0.0 {
        return 0.0;
    }
    let mu0 = expected_score(elo_low);
    let mu1 = expected_score(elo_high);
    sample.n as f64 * (mu1 - mu0) * (sample.mean - 0.5 * (mu0 + mu1)) / sample.variance
}

/// `units_completed` is what the job's `min`/`max` gates count, which for a
/// pairs job is pairs played. Under the pentanomial model that is also the
/// sample size, so the two now agree — they did not when the LLR was computed
/// over a filtered subset of games.
#[allow(clippy::too_many_arguments)]
pub fn evaluate(
    sample: &Sample,
    units_completed: u64,
    min_units: u64,
    max_units: u64,
    alpha: f64,
    beta: f64,
    elo_low: f64,
    elo_high: f64,
) -> SprtResult {
    let (lower_bound, upper_bound) = bounds(alpha, beta);
    let llr = llr(sample, elo_low, elo_high);
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

    /// As serialized, and as `jobs.sprt_decided_status` stores it.
    pub fn as_str(self) -> &'static str {
        match self {
            SprtStatus::Running => "running",
            SprtStatus::Passed => "passed",
            SprtStatus::Failed => "failed",
            SprtStatus::TerminatedAtMax => "terminated_at_max",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64, tolerance: f64) {
        assert!((a - b).abs() < tolerance, "{a} != {b}");
    }

    /// The scenario this model exists for. 10,000 pairs, of which 9,950 played
    /// identically (a guaranteed 1-1 split) and 50 saw player 1 lose both
    /// games: 20,000 games split 9,950-10,050.
    fn near_even_pool() -> Pentanomial {
        Pentanomial { counts: [50, 0, 9_950, 0, 0] }
    }

    #[test]
    fn pentanomial_mean_is_the_true_score_rate() {
        let sample = Sample::from_pentanomial(&near_even_pool());
        assert_eq!(sample.n, 10_000);
        // 9,950 wins and 10,050 losses over 20,000 games.
        approx(sample.mean, 9_950.0 / 20_000.0, 1e-12);
    }

    /// The pentanomial and the per-game tally are two views of the same games,
    /// so they must agree on the mean. They differ only in sample size — pairs
    /// rather than games — which is the point: the two games of a pair are not
    /// independent observations.
    #[test]
    fn pentanomial_and_game_views_agree_on_the_mean() {
        let pentanomial = near_even_pool();
        let pairs = Sample::from_pentanomial(&pentanomial);
        let games = Sample::from_games(&Tally { wins: 9_950, losses: 10_050, draws: 0 });
        approx(pairs.mean, games.mean, 1e-12);
        assert_eq!(pairs.n * 2, games.n);
        // The half-point identity the submission path checks.
        assert_eq!(pentanomial.half_points(), 2 * 9_950);
    }

    /// Pairing's variance reduction, made concrete: the identical pairs sit
    /// exactly at the mean, so the pair-level spread is a fraction of the
    /// game-level one. This is the benefit that filtering them out throws away.
    #[test]
    fn identical_pairs_reduce_variance_rather_than_being_noise() {
        let pairs = Sample::from_pentanomial(&near_even_pool());
        let games = Sample::from_games(&Tally { wins: 9_950, losses: 10_050, draws: 0 });
        assert!(pairs.variance < games.variance / 100.0);
    }

    /// Filtering to the pairs that diverged reports a difference two orders of
    /// magnitude too large. Same games, same direction, unusable magnitude.
    #[test]
    fn divergent_only_wildly_overstates_the_difference() {
        let all_pairs = Sample::from_pentanomial(&near_even_pool());
        // The 50 divergent pairs on their own: player 1 lost both games of each.
        let divergent_only = Sample::from_pentanomial(&Pentanomial { counts: [50, 0, 0, 0, 0] });

        let elo = |mean: f64| 400.0 * (mean / (1.0 - mean)).log10();
        approx(elo(all_pairs.mean), -1.74, 0.01);
        // A score rate of exactly 0 is off the Elo scale entirely; the point is
        // that it is nowhere near the -1.74 the games actually show.
        assert_eq!(divergent_only.mean, 0.0);
        assert!(all_pairs.mean > 0.49);
    }

    #[test]
    fn degenerate_samples_score_zero_rather_than_dividing_by_zero() {
        let empty = Sample::from_pentanomial(&Pentanomial::default());
        assert_eq!(llr(&empty, -10.0, 10.0), 0.0);
        // Every pair split: a real sample, but with no observed variance yet.
        let unanimous = Sample::from_pentanomial(&Pentanomial { counts: [0, 0, 100, 0, 0] });
        assert_eq!(unanimous.variance, 0.0);
        assert_eq!(llr(&unanimous, -10.0, 10.0), 0.0);
    }

    /// For a pairs job the sample size and the progress count are now the same
    /// number, which is what removes the old trap where the LLR counted a
    /// different population than `min_pairs` / `max_pairs` gated on.
    #[test]
    fn pairs_are_both_the_sample_and_the_progress_unit() {
        let pentanomial = near_even_pool();
        let sample = Sample::from_pentanomial(&pentanomial);
        assert_eq!(sample.n, pentanomial.pairs());
    }

    #[test]
    fn the_min_units_floor_holds_until_it_does_not() {
        let sample = Sample::from_pentanomial(&Pentanomial { counts: [0, 0, 10, 0, 90] });
        let running = evaluate(&sample, 100, 1_000, 10_000, 0.05, 0.05, -10.0, 10.0);
        assert_eq!(running.status, SprtStatus::Running);
        assert!(running.llr > running.upper_bound, "the LLR is reported, just not acted on");

        let acted = evaluate(&sample, 100, 10, 10_000, 0.05, 0.05, -10.0, 10.0);
        assert_eq!(acted.status, SprtStatus::Passed);
    }

    /// A job whose cap is below its floor terminates rather than running for
    /// ever.
    #[test]
    fn the_hard_cap_applies_even_below_the_floor() {
        let sample = Sample::from_pentanomial(&Pentanomial { counts: [0, 0, 100, 0, 0] });
        let result = evaluate(&sample, 100, 1_000, 50, 0.05, 0.05, -10.0, 10.0);
        assert_eq!(result.status, SprtStatus::TerminatedAtMax);
    }

    // The numbers below were computed outside this code, in 40-digit decimal
    // arithmetic straight from PLAN.md's formulas ("How the LLR is
    // computed"): `expected_score(±10)` = 0.485612815834001343… and
    // 0.514387184165998656…, and `llr = n·(µ₁-µ₀)·(mean-(µ₀+µ₁)/2)/variance`
    // with the per-game or per-pair mean and variance its table gives. They
    // pin the arithmetic, so a rewrite that agrees with itself but not with the
    // documented test fails here.

    /// I-STATS-9 (maths): the bounds at the schema's default α = β = 0.05 are
    /// ln(0.05/0.95) and ln(0.95/0.05), ±2.944438979166440460…
    #[test]
    fn the_default_bounds_are_the_documented_logarithms() {
        let (lower, upper) = bounds(0.05, 0.05);
        approx(lower, -2.944_438_979_166_440_5, 1e-12);
        approx(upper, 2.944_438_979_166_440_5, 1e-12);
        // Asymmetric error rates move each bound on its own.
        let (lower, upper) = bounds(0.05, 0.10);
        approx(lower, (0.10f64 / 0.95).ln(), 1e-15);
        approx(upper, 18f64.ln(), 1e-15);
    }

    /// I-STATS-1 (maths): W21 L7 D2 at ±10 Elo. Per game: mean 11/15, second
    /// moment 43/60, variance 161/900, LLR 1.125953543425981809…
    #[test]
    fn a_games_tally_scores_the_documented_llr() {
        let sample = Sample::from_games(&Tally { wins: 21, losses: 7, draws: 2 });
        assert_eq!(sample.n, 30);
        approx(sample.mean, 11.0 / 15.0, 1e-15);
        approx(sample.variance, 161.0 / 900.0, 1e-15);
        approx(llr(&sample, -10.0, 10.0), 1.125_953_543_425_981_8, 1e-12);
    }

    /// I-STATS-2 (maths): the pentanomial [1, 3, 7, 3, 2] is 16 pairs scored
    /// i/4: mean 17/32, variance 71/1024, LLR 0.207499670225107383…
    #[test]
    fn a_pentanomial_scores_the_documented_llr() {
        let sample = Sample::from_pentanomial(&Pentanomial { counts: [1, 3, 7, 3, 2] });
        assert_eq!(sample.n, 16);
        approx(sample.mean, 17.0 / 32.0, 1e-15);
        approx(sample.variance, 71.0 / 1024.0, 1e-15);
        approx(llr(&sample, -10.0, 10.0), 0.207_499_670_225_107_38, 1e-12);
    }

    /// I-STATS-9 (maths): a losing tally reaches H0. 28-72 over 100 games is
    /// LLR -3.140060036229865496…, below the lower bound, so the test fails
    /// (H0 accepted) once the floor is met; 29-71 is -2.934734021233334488…,
    /// just inside, and is still running. The mirror images pass and run.
    #[test]
    fn a_losing_tally_reaches_h0_and_one_game_short_does_not() {
        let games = |wins: u64| Sample::from_games(&Tally { wins, losses: 100 - wins, draws: 0 });
        let at = |wins: u64| evaluate(&games(wins), 100, 100, 1_000_000, 0.05, 0.05, -10.0, 10.0);

        let failed = at(28);
        approx(failed.llr, -3.140_060_036_229_865_5, 1e-12);
        assert_eq!(failed.status, SprtStatus::Failed);
        let inside = at(29);
        approx(inside.llr, -2.934_734_021_233_334_5, 1e-12);
        assert_eq!(inside.status, SprtStatus::Running);

        let passed = at(72);
        approx(passed.llr, 3.140_060_036_229_865_5, 1e-12);
        assert_eq!(passed.status, SprtStatus::Passed);
        let inside = at(71);
        approx(inside.llr, 2.934_734_021_233_334_5, 1e-12);
        assert_eq!(inside.status, SprtStatus::Running);
    }
}
