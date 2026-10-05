//! What a games or game-pairs job's results reduce to: per-game outcome
//! counts, pair-outcome counts, and a sample of either -- how many independent
//! observations, their mean score and its variance -- which is what the match
//! test (`super::match_test`) and the rating fits read.
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
//! Elo. A test fed the second number decides almost immediately, on a
//! hundredth of the evidence, whatever confidence it is asked for.

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

/// A sample reduced to what the test needs: how many independent
/// observations, their mean score, and the variance of that score. Keeping
/// this separate from how the counts were gathered is what lets games and
/// pairs share one test.
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
    /// 0.5 is one game's worth each way — so it reads as a per-game score and
    /// a per-game Elo, while `n` counts pairs.
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

    /// U-STATS-1: W21 L7 D2 per game: mean 11/15, second moment
    /// 43/60, variance 161/900.
    #[test]
    fn a_games_tally_has_the_documented_mean_and_variance() {
        let sample = Sample::from_games(&Tally { wins: 21, losses: 7, draws: 2 });
        assert_eq!(sample.n, 30);
        approx(sample.mean, 11.0 / 15.0, 1e-15);
        approx(sample.variance, 161.0 / 900.0, 1e-15);
    }

    /// U-STATS-2: the pentanomial [1, 3, 7, 3, 2] is 16 pairs scored
    /// i/4: mean 17/32, variance 71/1024.
    #[test]
    fn a_pentanomial_has_the_documented_mean_and_variance() {
        let sample = Sample::from_pentanomial(&Pentanomial { counts: [1, 3, 7, 3, 2] });
        assert_eq!(sample.n, 16);
        approx(sample.mean, 17.0 / 32.0, 1e-15);
        approx(sample.variance, 71.0 / 1024.0, 1e-15);
    }

    /// For a pairs job the sample size and the progress count are the same
    /// number: the test counts the population `min_pairs` / `max_pairs` gate
    /// on.
    #[test]
    fn pairs_are_both_the_sample_and_the_progress_unit() {
        let pentanomial = near_even_pool();
        let sample = Sample::from_pentanomial(&pentanomial);
        assert_eq!(sample.n, pentanomial.pairs());
    }
}
