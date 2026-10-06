//! The match test: is one player better than the other, at a stated
//! confidence?
//!
//! A games or game-pairs job that runs a test keeps a confidence interval for
//! player 1's score -- per game, 1 for a win, ½ for a draw -- over every unit
//! it has played (a game, or a pair scored `i / 4`; see [`super::outcomes`]),
//! and stops as soon as the interval excludes an even score:
//!
//! * lower bound above ½: **player 1 is better**;
//! * upper bound below ½: **player 2 is better**;
//! * neither by `max_units`: **inconclusive**, and the interval says how
//!   large a difference the games rule out.
//!
//! The interval is an *asymptotic confidence sequence* (Waudby-Smith, Arbour,
//! Sinha, Kennedy & Ramdas, "Time-uniform central limit theory and asymptotic
//! confidence sequences", 2021): the Robbins normal-mixture boundary with the
//! sample's own variance,
//!
//! ```text
//! half_width(n) = sqrt( 2·(n·σ²·ρ² + 1) / (n²·ρ²) · ln( sqrt(n·σ²·ρ² + 1) / α ) )
//! ρ²            = (−2·ln α + ln(−2·ln α + 1)) / (n*·v)
//! ```
//!
//! with α = 1 − confidence. Unlike a fixed-sample interval it holds at every
//! `n` at once: the chance it ever excludes the true score, however often it
//! is looked at, is about α. So checking it after every few submissions (see
//! `state::TEST_CHECK_EVERY`) and stopping the moment it decides is valid,
//! where stopping the first time an ordinary 95% interval excluded ½ would
//! declare a winner between equal players far more often than one time in
//! twenty. Equal players are kept apart from a winner by the interval alone:
//! the chance of ever naming one between them is at most about α, split
//! between the two.
//!
//! It replaced an SPRT between two Elo hypotheses (−10 and +10 by default),
//! which answered a different question -- which of the two the data favoured
//! -- and so named a winner between equal players half the time, and asked
//! the admin for an Elo margin and two error rates that "is A better, at 95%?"
//! has no place for.
//!
//! Three choices are worth knowing:
//!
//! * **Asymptotic.** It assumes enough units for the score's mean to be close
//!   to normal, which is why a job states `min_units`, and nothing is acted on
//!   before it. An exact, nonasymptotic sequence (a betting one) needs each
//!   unit's outcome in order, which batches do not keep; this needs only the
//!   count, the mean and the variance, which the stored tallies give at any
//!   moment, so it is recomputed from them on every read.
//! * **Tuned at n\* = √(min_units · max_units)**, where the boundary is
//!   tightest ([`tuning_units`]), for a unit whose score has the planning
//!   variance `v` -- ¼ for a game, the most a score in [0, 1] can have, and
//!   1/16 for a pair, about what a paired match's pair scores have
//!   ([`planning_variance`]). The paper's ρ is tuned for a variance of 1:
//!   used as it is on scores of variance near 1/20, it put the tightest point
//!   twenty times past n\*, and the interval early in a job was wide enough
//!   that 90 wins in 100 games decided nothing. The boundary is flat around
//!   its tightest point -- at 95%, within a tenth of the narrowest any tuning
//!   gives, from a fifth of n\* to twenty times it -- so neither guess needs
//!   to be close. Both are fixed
//!   before any game is played, as the boundary must be for the guarantee to
//!   hold; the variance used *inside* it is the sample's own.
//! * **No observed variance is still an interval.** Every pair split (σ² = 0)
//!   leaves a half-width of `sqrt(2·ln(1/α) / (n²·ρ²))`, which shrinks with
//!   `n` but never reaches zero, so a run of identical pairs cannot decide
//!   anything on its own.

use super::outcomes::Sample;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TestStatus {
    Running,
    /// The interval's lower bound is above an even score.
    Player1Better,
    /// Its upper bound is below an even score.
    Player2Better,
    /// The cap was reached with the interval still around an even score.
    Inconclusive,
}

impl TestStatus {
    pub fn is_finished(self) -> bool {
        !matches!(self, TestStatus::Running)
    }

    /// As serialized, and as `jobs.test_decided_status` stores it.
    pub fn as_str(self) -> &'static str {
        match self {
            TestStatus::Running => "running",
            TestStatus::Player1Better => "player1_better",
            TestStatus::Player2Better => "player2_better",
            TestStatus::Inconclusive => "inconclusive",
        }
    }
}

/// Where the test stands: player 1's score per game and the interval around
/// it, the same three as Elo, the confidence asked for, and the decision.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct TestResult {
    /// Player 1's mean score per game; ½ before anything is played.
    pub mean: f64,
    /// The interval, clipped to the scores that exist, 0 to 1.
    pub lower: f64,
    pub upper: f64,
    /// Player 1's Elo difference: [`elo`] of the three above.
    pub elo: f64,
    pub elo_lower: f64,
    pub elo_upper: f64,
    pub confidence_pct: f64,
    pub status: TestStatus,
}

/// The largest Elo difference reported: a score of 0 or 1 is infinitely far
/// off the scale.
pub const MAX_ELO: f64 = 1000.0;

/// The Elo difference at which a player expects this score per game,
/// `−400·log10(1/s − 1)`, within ±[`MAX_ELO`].
pub fn elo(score: f64) -> f64 {
    if score <= 0.0 {
        return -MAX_ELO;
    }
    if score >= 1.0 {
        return MAX_ELO;
    }
    (-400.0 * (1.0 / score - 1.0).log10()).clamp(-MAX_ELO, MAX_ELO)
}

/// Where the boundary is tightest: the geometric mean of the first units the
/// test is acted on and its cap, each at least 1.
pub fn tuning_units(min_units: u64, max_units: u64) -> f64 {
    (min_units.max(1) as f64 * max_units.max(1) as f64).sqrt()
}

/// What the unit's score variance is planned to be, which with
/// [`tuning_units`] places the boundary's tightest point: the most a game's
/// score can vary, and about what a paired match's pair scores do.
pub fn planning_variance(unit: Unit) -> f64 {
    match unit {
        Unit::Game => 0.25,
        Unit::Pair => 0.0625,
    }
}

/// What one observation is: a game, scored 1, ½ or 0, or a pair, scored `i / 4`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Game,
    Pair,
}

/// The mixture's ρ²: tightest after `n_star` units of variance `variance`.
pub fn rho_squared(alpha: f64, n_star: f64, variance: f64) -> f64 {
    let log_alpha = alpha.ln();
    (-2.0 * log_alpha + (-2.0 * log_alpha + 1.0).ln()) / (n_star * variance)
}

/// The interval's half-width after `n` units of variance `variance`, at
/// coverage `1 − alpha`, for the mixture `rho2` ([`rho_squared`]). Infinite
/// before any unit is played.
pub fn half_width(n: u64, variance: f64, alpha: f64, rho2: f64) -> f64 {
    if n == 0 {
        return f64::INFINITY;
    }
    let n = n as f64;
    let x = n * variance.max(0.0) * rho2 + 1.0;
    (2.0 * x / (n * n * rho2) * (x.sqrt() / alpha).ln()).sqrt()
}

/// The test over `sample`, with `units_completed` the count the job's
/// `min_units` and `max_units` gate on (games, or pairs -- the sample's own
/// size). Below `min_units` the interval is reported and not acted on, unless
/// the cap is lower still: a job whose cap is below its floor is inconclusive
/// at its cap rather than running for ever.
pub fn evaluate(
    sample: &Sample,
    unit: Unit,
    units_completed: u64,
    confidence_pct: f64,
    min_units: u64,
    max_units: u64,
) -> TestResult {
    let alpha = 1.0 - confidence_pct / 100.0;
    let rho2 = rho_squared(alpha, tuning_units(min_units, max_units), planning_variance(unit));
    let width = half_width(sample.n, sample.variance, alpha, rho2);
    let mean = if sample.n == 0 { 0.5 } else { sample.mean };
    let (lower, upper) = ((mean - width).max(0.0), (mean + width).min(1.0));
    let n = units_completed;
    let status = if n < min_units {
        if n >= max_units {
            TestStatus::Inconclusive
        } else {
            TestStatus::Running
        }
    } else if lower > 0.5 {
        TestStatus::Player1Better
    } else if upper < 0.5 {
        TestStatus::Player2Better
    } else if n >= max_units {
        TestStatus::Inconclusive
    } else {
        TestStatus::Running
    };
    TestResult {
        mean,
        lower,
        upper,
        elo: elo(mean),
        elo_lower: elo(lower),
        elo_upper: elo(upper),
        confidence_pct,
        status,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::outcomes::{Pentanomial, Tally};
    use rand::{Rng, SeedableRng};

    fn approx(a: f64, b: f64, tolerance: f64) {
        assert!((a - b).abs() < tolerance, "{a} != {b}");
    }

    // The numbers below were computed outside this code, in 40-digit decimal
    // arithmetic straight from the module's formula. They pin the arithmetic,
    // so a rewrite that agrees with itself but not with the documented
    // boundary fails here.

    /// U-STATS-1: n = 1000, σ² = 1/16, α = 0.05, tuned at 1000 units
    /// of variance ¼, gives a half-width of 0.025806505014905885…
    #[test]
    fn the_half_width_is_the_documented_boundary() {
        let rho2 = rho_squared(0.05, 1000.0, 0.25);
        approx(half_width(1000, 0.0625, 0.05, rho2), 0.025_806_505_014_905_885, 1e-12);
        assert_eq!(half_width(0, 0.25, 0.05, rho2), f64::INFINITY);
    }

    /// U-STATS-1: W21 L7 D2 at 95%, floor 10 and cap 1,000 (n* = 100):
    /// mean 11/15, interval 0.476214864843405827… to 0.990451801823260840…
    #[test]
    fn a_games_tally_gets_the_documented_interval() {
        let sample = Sample::from_games(&Tally { wins: 21, losses: 7, draws: 2 });
        let result = evaluate(&sample, Unit::Game, 30, 95.0, 10, 1000);
        approx(result.mean, 11.0 / 15.0, 1e-15);
        approx(result.lower, 0.476_214_864_843_405_8, 1e-12);
        approx(result.upper, 0.990_451_801_823_260_8, 1e-12);
        assert_eq!(result.status, TestStatus::Running);
        // A wider one is clipped to the scores that exist.
        let few = evaluate(&Sample::from_games(&Tally { wins: 5, losses: 0, draws: 0 }), Unit::Game, 5, 95.0, 10, 1000);
        assert_eq!(few.upper, 1.0);
    }

    /// U-STATS-2: the pentanomial [1, 3, 7, 3, 2] at 90%, floor and
    /// cap 16 (n* = 16, variance 1/16): mean 17/32, half-width
    /// 0.181952165436550379…, and inconclusive at its cap.
    #[test]
    fn a_pentanomial_gets_the_documented_interval() {
        let sample = Sample::from_pentanomial(&Pentanomial { counts: [1, 3, 7, 3, 2] });
        let result = evaluate(&sample, Unit::Pair, 16, 90.0, 16, 16);
        approx(result.lower, 17.0 / 32.0 - 0.181_952_165_436_550_3, 1e-12);
        approx(result.upper, 17.0 / 32.0 + 0.181_952_165_436_550_3, 1e-12);
        assert_eq!(result.status, TestStatus::Inconclusive);
    }

    /// U-STATS-3: over 1,000 games without a draw at 95% (floor 100,
    /// cap 10,000, so n* = 1000), 548 wins is the fewest that decide: its
    /// lower bound is 0.500229941162600039…, and 547's is
    /// 0.499220820372402546…. The mirror images decide for player 2 and run.
    #[test]
    fn the_fewest_wins_that_decide_and_one_short() {
        let at = |wins: u64| {
            let sample = Sample::from_games(&Tally { wins, losses: 1000 - wins, draws: 0 });
            evaluate(&sample, Unit::Game, 1000, 95.0, 100, 10_000)
        };
        let decided = at(548);
        approx(decided.lower, 0.500_229_941_162_6, 1e-12);
        assert_eq!(decided.status, TestStatus::Player1Better);
        let short = at(547);
        approx(short.lower, 0.499_220_820_372_402_5, 1e-12);
        assert_eq!(short.status, TestStatus::Running);
        assert_eq!(at(452).status, TestStatus::Player2Better);
        assert_eq!(at(453).status, TestStatus::Running);
    }

    /// U-STATS-3: every pair split is no evidence either way: the interval
    /// stays around an even score however many there are, and the job ends
    /// inconclusive.
    #[test]
    fn identical_pairs_alone_never_decide() {
        let sample = Sample::from_pentanomial(&Pentanomial { counts: [0, 0, 10_000, 0, 0] });
        let result = evaluate(&sample, Unit::Pair, 10_000, 95.0, 500, 10_000);
        approx(result.upper - 0.5, 0.001_027_173_831_966_772_8, 1e-12);
        assert_eq!(result.status, TestStatus::Inconclusive);
        assert_eq!((result.elo, result.elo_lower < 0.0, result.elo_upper > 0.0), (0.0, true, true));
    }

    #[test]
    fn the_floor_holds_until_it_does_not_and_the_cap_applies_below_it() {
        let sample = Sample::from_pentanomial(&Pentanomial { counts: [0, 0, 10, 0, 90] });
        let running = evaluate(&sample, Unit::Pair, 100, 95.0, 1_000, 10_000);
        assert_eq!(running.status, TestStatus::Running);
        assert!(running.lower > 0.5, "the interval is reported, just not acted on");
        assert_eq!(evaluate(&sample, Unit::Pair, 100, 95.0, 10, 10_000).status, TestStatus::Player1Better);
        let even = Sample::from_pentanomial(&Pentanomial { counts: [0, 0, 100, 0, 0] });
        assert_eq!(evaluate(&even, Unit::Pair, 100, 95.0, 1_000, 50).status, TestStatus::Inconclusive);
    }

    #[test]
    fn nothing_played_is_an_even_score_with_every_score_possible() {
        let empty = Sample::from_pentanomial(&Pentanomial::default());
        let result = evaluate(&empty, Unit::Pair, 0, 95.0, 100, 1_000);
        assert_eq!((result.mean, result.lower, result.upper), (0.5, 0.0, 1.0));
        assert_eq!((result.elo_lower, result.elo_upper), (-MAX_ELO, MAX_ELO));
        assert_eq!(result.status, TestStatus::Running);
    }

    #[test]
    fn elo_is_the_logistic_inverse_within_its_bounds() {
        assert_eq!(elo(0.5), 0.0);
        approx(elo(0.75), 400.0 * 3f64.log10(), 1e-12);
        approx(elo(0.25), -400.0 * 3f64.log10(), 1e-12);
        assert_eq!((elo(0.0), elo(1.0)), (-MAX_ELO, MAX_ELO));
        assert_eq!(elo(1e-12), -MAX_ELO);
    }

    /// Pairs drawn from a pentanomial with these bucket probabilities, the
    /// test checked after every batch of `batch` pairs from the floor to the
    /// cap, as the finish check does: how many runs out of `runs` ended in
    /// each verdict.
    fn simulate(probabilities: [f64; 5], runs: usize, batch: u64, min: u64, max: u64) -> [usize; 3] {
        let mut rng = rand::rngs::StdRng::seed_from_u64(0x6d61_7463_6874_6573);
        let mut verdicts = [0usize; 3];
        for _ in 0..runs {
            let mut counts = [0u64; 5];
            let mut played = 0;
            let status = loop {
                for _ in 0..batch {
                    let mut draw: f64 = rng.gen();
                    let bucket = probabilities
                        .iter()
                        .position(|&p| {
                            draw -= p;
                            draw < 0.0
                        })
                        .unwrap_or(4);
                    counts[bucket] += 1;
                }
                played += batch;
                let sample = Sample::from_pentanomial(&Pentanomial { counts });
                let status = evaluate(&sample, Unit::Pair, played, 95.0, min, max).status;
                if status.is_finished() {
                    break status;
                }
            };
            verdicts[match status {
                TestStatus::Player1Better => 0,
                TestStatus::Player2Better => 1,
                _ => 2,
            }] += 1;
        }
        verdicts
    }

    /// U-STATS-3b: checked after every batch, the test names a
    /// winner between equal players in at most about α of runs. A plain 95%
    /// interval checked as often names one in several times that.
    #[test]
    fn equal_players_rarely_get_a_winner_however_often_it_is_checked() {
        // Equal, with three pairs in five split: a typical paired match.
        let runs = 1000;
        let [p1, p2, inconclusive] = simulate([0.05, 0.15, 0.6, 0.15, 0.05], runs, 50, 500, 10_000);
        let false_rate = (p1 + p2) as f64 / runs as f64;
        assert!(false_rate <= 0.05 + 0.02, "{p1} + {p2} winners in {runs} runs between equal players");
        assert_eq!(p1 + p2 + inconclusive, runs);
    }

    /// U-STATS-3b: a player scoring 53.5% per game (about +24
    /// Elo) is found better in nearly every run before the cap.
    #[test]
    fn a_better_player_is_found() {
        let runs = 200;
        let [p1, p2, _] = simulate([0.03, 0.12, 0.6, 0.18, 0.07], runs, 50, 500, 10_000);
        assert!(p1 as f64 >= 0.9 * runs as f64, "found in {p1} of {runs} runs");
        assert_eq!(p2, 0, "never the wrong way");
    }
}
