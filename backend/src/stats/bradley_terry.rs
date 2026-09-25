//! Batch Bradley-Terry rating fit, anchored on one fixed player.
//!
//! Every rating in a pool is solved for jointly, from scratch, out of the whole
//! pairwise result matrix. Nothing is incremental and nothing is "established"
//! and then frozen.
//!
//! That is a deliberate departure from Elo and Glicko, which are sequential
//! filters built for humans whose strength drifts: they nudge a rating per
//! result because history is stale and cannot be refit. A player config is a
//! frozen set of MAGPIE flags — immutable once any job references it — so its
//! strength is a constant, there is no drift to track, and the machinery for
//! tracking drift buys nothing while costing path dependence. Two consequences
//! matter in practice:
//!
//! * Ratings do not depend on the order results arrived in.
//! * Adding or removing a config from the pool is a re-fit, not a surgical
//!   undo of its historical updates, so it is correct by construction.
//!
//! What this model *cannot* represent is non-transitivity: if A beats B, B
//! beats C and C beats A, no assignment of one number per player reproduces
//! that, and no scalar rating system can. The fit returns the best scalar
//! approximation and [`Fit::residuals`] reports where it is lying, which is the
//! honest way to surface a rock-paper-scissors triangle rather than letting it
//! quietly distort the numbers.

/// Elo points per unit of natural-log strength.
const ELO_PER_LN: f64 = 400.0 / std::f64::consts::LN_10;

/// Virtual drawn games against a player of the anchor's strength, added to
/// every competitor.
///
/// Without this the maximum-likelihood fit is unbounded whenever a player has
/// a perfect or zero score against everyone it played — an undefeated new bot
/// is not an edge case, it is what a strong config's first job looks like —
/// and it is undefined for a player with no games at all. A small prior keeps
/// every rating finite and pulls the barely-observed toward the anchor, where
/// the standard error then says how little the number is worth.
const DEFAULT_PRIOR_GAMES: f64 = 2.0;

const MAX_ITERATIONS: usize = 10_000;
const CONVERGENCE: f64 = 1e-12;

/// One player's accumulated results against one opponent.
#[derive(Debug, Clone, Copy, Default)]
pub struct Head2Head {
    /// Games played between the two.
    pub games: f64,
    /// Score earned by the lower-indexed player: 1 per win, 0.5 per draw.
    pub score: f64,
}

/// The input matrix: `n` players and their pairwise results.
#[derive(Debug, Clone)]
pub struct Matrix {
    n: usize,
    /// Row-major `n * n`. `cell(i, j)` holds i's games and score against j, so
    /// `cell(i, j).score + cell(j, i).score == cell(i, j).games`.
    cells: Vec<Head2Head>,
}

impl Matrix {
    pub fn new(n: usize) -> Self {
        Self { n, cells: vec![Head2Head::default(); n * n] }
    }

    pub fn len(&self) -> usize {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get(&self, i: usize, j: usize) -> Head2Head {
        self.cells[i * self.n + j]
    }

    /// Records `games` games between `i` and `j` in which `i` scored
    /// `score_i`. Symmetric: the mirrored cell is filled in too, so callers
    /// add each head-to-head once.
    pub fn add(&mut self, i: usize, j: usize, games: f64, score_i: f64) {
        if i == j || games <= 0.0 {
            return;
        }
        let n = self.n;
        let forward = &mut self.cells[i * n + j];
        forward.games += games;
        forward.score += score_i;
        let backward = &mut self.cells[j * n + i];
        backward.games += games;
        backward.score += games - score_i;
    }

    fn games_played(&self, i: usize) -> f64 {
        (0..self.n).map(|j| self.get(i, j).games).sum()
    }

    fn score_of(&self, i: usize) -> f64 {
        (0..self.n).map(|j| self.get(i, j).score).sum()
    }
}

/// One player's fitted rating.
#[derive(Debug, Clone, Copy)]
pub struct Rated {
    pub rating: f64,
    /// Approximate standard error in Elo, from the diagonal of the Fisher
    /// information. Two approximations pull it opposite ways, so it is neither
    /// bound on the true uncertainty: ignoring the off-diagonal terms makes it
    /// narrower, and counting each unit of `games` as one Bernoulli trial
    /// makes it wider when a unit is more than one game -- the ratings feed
    /// one paired game (two games, scored in quarters) per unit, whose
    /// information is at least twice that, so from pairs it reads at least
    /// √2 wide. Good enough to tell 1700 ± 15 from 1700 ± 200, which is the
    /// distinction the page needs to draw.
    pub stderr: f64,
    pub games: f64,
    /// Index of this player's connected component in the graph of who actually
    /// played whom. Only the anchor's component is identifiable; see
    /// [`Fit::anchor_component`].
    pub component: usize,
}

/// A residual: where the fitted scalar ratings disagree with what happened.
#[derive(Debug, Clone, Copy)]
pub struct Residual {
    pub i: usize,
    pub j: usize,
    pub games: f64,
    /// i's actual score rate against j.
    pub actual: f64,
    /// What the fitted ratings predict it should have been.
    pub predicted: f64,
}

#[derive(Debug, Clone)]
pub struct Fit {
    pub ratings: Vec<Rated>,
    /// The component containing the anchor. A player outside it has no path of
    /// games connecting it to the anchor, so its rating is determined by the
    /// prior alone and means nothing: display it as unrated rather than as a
    /// confident number.
    pub anchor_component: usize,
    pub iterations: usize,
    pub converged: bool,
}

impl Fit {
    pub fn is_rateable(&self, index: usize) -> bool {
        self.ratings[index].component == self.anchor_component && self.ratings[index].games > 0.0
    }

    /// Actual versus predicted score for every head-to-head with games in it,
    /// worst disagreement first. A non-transitive triangle shows up here as a
    /// set of large residuals that no rating assignment could remove.
    pub fn residuals(&self, matrix: &Matrix) -> Vec<Residual> {
        let mut out = Vec::new();
        for i in 0..matrix.len() {
            for j in (i + 1)..matrix.len() {
                let cell = matrix.get(i, j);
                if cell.games <= 0.0 {
                    continue;
                }
                let elo_diff = self.ratings[i].rating - self.ratings[j].rating;
                out.push(Residual {
                    i,
                    j,
                    games: cell.games,
                    actual: cell.score / cell.games,
                    predicted: 1.0 / (1.0 + 10f64.powf(-elo_diff / 400.0)),
                });
            }
        }
        out.sort_by(|a, b| {
            let (da, db) = ((a.actual - a.predicted).abs(), (b.actual - b.predicted).abs());
            db.partial_cmp(&da).unwrap_or(std::cmp::Ordering::Equal)
        });
        out
    }
}

/// Connected components over "played at least one game against".
fn components(matrix: &Matrix) -> Vec<usize> {
    let n = matrix.len();
    let mut component = vec![usize::MAX; n];
    let mut next = 0;
    for start in 0..n {
        if component[start] != usize::MAX {
            continue;
        }
        let mut stack = vec![start];
        component[start] = next;
        while let Some(current) = stack.pop() {
            for (other, slot) in component.iter_mut().enumerate() {
                if *slot == usize::MAX && matrix.get(current, other).games > 0.0 {
                    *slot = next;
                    stack.push(other);
                }
            }
        }
        next += 1;
    }
    component
}

/// Fits ratings by minorization-maximization, holding `anchor` at
/// `anchor_rating`.
///
/// MM is used rather than a gradient method because each step is a closed-form
/// ratio with no step size to tune, it cannot overshoot, and it converges
/// monotonically in the likelihood — for a few dozen players it lands in
/// microseconds, which is what makes recomputing the whole pool on every change
/// affordable.
pub fn fit(matrix: &Matrix, anchor: usize, anchor_rating: f64) -> Fit {
    fit_with_prior(matrix, anchor, anchor_rating, DEFAULT_PRIOR_GAMES)
}

pub fn fit_with_prior(matrix: &Matrix, anchor: usize, anchor_rating: f64, prior: f64) -> Fit {
    let n = matrix.len();
    let component = components(matrix);
    let anchor_component = component.get(anchor).copied().unwrap_or(0);

    // Work in strengths (gamma), converting to Elo only at the end.
    let anchor_gamma = 10f64.powf(anchor_rating / 400.0);
    let mut gamma = vec![anchor_gamma; n];

    let scores: Vec<f64> = (0..n).map(|i| matrix.score_of(i)).collect();

    let mut iterations = 0;
    let mut converged = false;
    while iterations < MAX_ITERATIONS {
        iterations += 1;
        let mut max_change: f64 = 0.0;
        for i in 0..n {
            if i == anchor {
                continue;
            }
            // Numerator: observed score, plus the prior's virtual draws.
            let numerator = scores[i] + prior / 2.0;
            // Denominator: expected-score normaliser over every opponent, plus
            // the prior's virtual opponent sitting at the anchor's strength.
            let mut denominator = prior / (gamma[i] + anchor_gamma);
            for j in 0..n {
                let games = matrix.get(i, j).games;
                if games > 0.0 {
                    denominator += games / (gamma[i] + gamma[j]);
                }
            }
            if denominator <= 0.0 {
                continue;
            }
            let updated = numerator / denominator;
            if updated > 0.0 && updated.is_finite() {
                let change = ((updated / gamma[i]).ln()).abs();
                max_change = max_change.max(change);
                gamma[i] = updated;
            }
        }
        if max_change < CONVERGENCE {
            converged = true;
            break;
        }
    }

    let ratings = (0..n)
        .map(|i| {
            // The anchor's rating is the input, not a round trip through its
            // strength: `400 * log10(10^(r / 400))` is not `r` in floating
            // point, and the pool's fixed point must be stored as stated.
            let rating = if i == anchor { anchor_rating } else { 400.0 * gamma[i].log10() };
            // Fisher information for player i in log-strength units: each game
            // contributes p(1-p), which is largest between evenly matched
            // players and vanishes in a mismatch — the reason games against a
            // far stronger or weaker opponent tell you so little.
            let mut information = 0.0;
            for j in 0..n {
                let games = matrix.get(i, j).games;
                if games > 0.0 {
                    let p = gamma[i] / (gamma[i] + gamma[j]);
                    information += games * p * (1.0 - p);
                }
            }
            let stderr =
                if information > 0.0 { ELO_PER_LN / information.sqrt() } else { f64::INFINITY };
            Rated { rating, stderr, games: matrix.games_played(i), component: component[i] }
        })
        .collect();

    Fit { ratings, anchor_component, iterations, converged }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANCHOR: f64 = 2000.0;

    /// `players` competitors; caller fills the matrix.
    fn matrix(n: usize) -> Matrix {
        Matrix::new(n)
    }

    #[test]
    fn the_anchor_stays_put() {
        let mut m = matrix(2);
        m.add(0, 1, 100.0, 70.0);
        let fit = fit(&m, 0, ANCHOR);
        assert!((fit.ratings[0].rating - ANCHOR).abs() < 1e-9);
    }

    /// An even head-to-head puts both players at the anchor's rating; a
    /// lopsided one separates them in the right direction.
    #[test]
    fn even_results_rate_equal_and_wins_rate_higher() {
        let mut even = matrix(2);
        even.add(0, 1, 1_000.0, 500.0);
        let fit_even = fit(&even, 0, ANCHOR);
        assert!((fit_even.ratings[1].rating - ANCHOR).abs() < 1.0);

        let mut lopsided = matrix(2);
        lopsided.add(0, 1, 1_000.0, 250.0);
        let fit_lopsided = fit(&lopsided, 0, ANCHOR);
        assert!(fit_lopsided.ratings[1].rating > ANCHOR + 100.0);
    }

    /// A 75% score rate is about 191 Elo in the Bradley-Terry model, the same
    /// number the Elo formula gives, since they are the same model.
    #[test]
    fn the_scale_matches_the_elo_formula() {
        let mut m = matrix(2);
        m.add(0, 1, 100_000.0, 25_000.0);
        let fit = fit(&m, 0, ANCHOR);
        let expected = 400.0 * (0.75f64 / 0.25).log10();
        assert!((fit.ratings[1].rating - ANCHOR - expected).abs() < 1.0);
    }

    /// The fit does not depend on the order results were added, which is the
    /// property an incremental filter cannot offer.
    #[test]
    fn the_fit_is_order_independent() {
        let mut forward = matrix(3);
        forward.add(0, 1, 100.0, 60.0);
        forward.add(1, 2, 100.0, 55.0);
        forward.add(0, 2, 100.0, 70.0);

        let mut backward = matrix(3);
        backward.add(0, 2, 100.0, 70.0);
        backward.add(1, 2, 100.0, 55.0);
        backward.add(0, 1, 100.0, 60.0);

        let a = fit(&forward, 0, ANCHOR);
        let b = fit(&backward, 0, ANCHOR);
        for i in 0..3 {
            assert!((a.ratings[i].rating - b.ratings[i].rating).abs() < 1e-6);
        }
    }

    /// An undefeated player would send the unregularised maximum likelihood to
    /// infinity. The prior keeps it finite — and the standard error stays wide,
    /// which is how the page knows not to trust it.
    #[test]
    fn an_undefeated_player_gets_a_finite_rating() {
        let mut m = matrix(2);
        m.add(0, 1, 20.0, 0.0);
        let fit = fit(&m, 0, ANCHOR);
        assert!(fit.ratings[1].rating.is_finite());
        assert!(fit.ratings[1].rating > ANCHOR);
    }

    /// A player with no games at all is pulled to the anchor by the prior and
    /// reported with infinite uncertainty, rather than being an unsolvable
    /// singularity in the fit.
    #[test]
    fn a_player_with_no_games_is_flagged_not_guessed() {
        let mut m = matrix(3);
        m.add(0, 1, 100.0, 50.0);
        let fit = fit(&m, 0, ANCHOR);
        assert!(!fit.is_rateable(2));
        assert!(fit.ratings[2].stderr.is_infinite());
    }

    /// Two configs that only ever played each other have no path to the anchor,
    /// so their ratings are unidentifiable. The fit must say so rather than
    /// return a confident number.
    #[test]
    fn a_disconnected_component_is_not_rateable() {
        let mut m = matrix(4);
        m.add(0, 1, 100.0, 50.0); // anchor's component
        m.add(2, 3, 100.0, 90.0); // an island
        let fit = fit(&m, 0, ANCHOR);
        assert!(fit.is_rateable(1));
        assert!(!fit.is_rateable(2));
        assert!(!fit.is_rateable(3));
        assert_ne!(fit.ratings[2].component, fit.anchor_component);
    }

    /// Standard error narrows as evidence accumulates. This is what lets the
    /// page distinguish 1700 ± 200 from 1700 ± 15 rather than showing two
    /// identical-looking numbers.
    #[test]
    fn more_games_narrow_the_standard_error() {
        let mut few = matrix(2);
        few.add(0, 1, 50.0, 25.0);
        let mut many = matrix(2);
        many.add(0, 1, 5_000.0, 2_500.0);
        let (few, many) = (fit(&few, 0, ANCHOR), fit(&many, 0, ANCHOR));
        assert!(many.ratings[1].stderr < few.ratings[1].stderr);

        // And by exactly as much as the analytic standard error says:
        // (400 / ln 10) / sqrt(n·p·(1-p)), with p = 1/2 for an even
        // head-to-head. Computed outside this code: 49.134811709710… at 50
        // games and a tenth of that at 5,000.
        let close = |got: f64, want: f64| {
            assert!((got - want).abs() < want * 1e-6, "{got} != {want}");
        };
        close(few.ratings[1].stderr, 49.134_811_709_710_03);
        close(many.ratings[1].stderr, 4.913_481_170_971_004);

        // Away from even the p(1-p) term matters: 75% over 100,000 games is
        // 1.268655383138… Elo. The prior's two virtual draws move p by about
        // 1e-5, so this is held to 0.1%.
        let mut lopsided = matrix(2);
        lopsided.add(0, 1, 100_000.0, 25_000.0);
        let got = fit(&lopsided, 0, ANCHOR).ratings[1].stderr;
        assert!((got - 1.268_655_383_138_294).abs() < 1.268_655_383_138_294e-3, "{got}");
    }

    /// The case that rules out freezing a rating once it is "established": A
    /// beats B, B beats C, C beats A. No assignment of one number per player
    /// reproduces that, so the fit returns the best scalar approximation — here
    /// three near-identical ratings — and the residuals carry the truth the
    /// ratings cannot.
    #[test]
    fn non_transitivity_collapses_to_flat_ratings_and_loud_residuals() {
        let mut m = matrix(3);
        m.add(0, 1, 1_000.0, 750.0); // A beats B
        m.add(1, 2, 1_000.0, 750.0); // B beats C
        m.add(2, 0, 1_000.0, 750.0); // C beats A
        let fit = fit(&m, 0, ANCHOR);

        let spread = fit.ratings.iter().map(|r| r.rating).fold(f64::MIN, f64::max)
            - fit.ratings.iter().map(|r| r.rating).fold(f64::MAX, f64::min);
        assert!(spread < 1.0, "a cycle has no scalar solution, so ratings flatten out");

        // Every head-to-head is badly mispredicted, and the residuals say so.
        let residuals = fit.residuals(&m);
        assert_eq!(residuals.len(), 3);
        for residual in &residuals {
            assert!((residual.actual - residual.predicted).abs() > 0.2);
        }
    }

    /// A transitive ladder, by contrast, is fit almost exactly: the residuals
    /// are what distinguish "the model works here" from "the model cannot".
    #[test]
    fn a_transitive_ladder_has_small_residuals() {
        let mut m = matrix(3);
        m.add(0, 1, 1_000.0, 600.0);
        m.add(1, 2, 1_000.0, 600.0);
        m.add(0, 2, 1_000.0, 690.0);
        let fit = fit(&m, 0, ANCHOR);
        for residual in fit.residuals(&m) {
            assert!((residual.actual - residual.predicted).abs() < 0.05);
        }
    }

    #[test]
    fn removing_a_player_moves_everyone_else() {
        let mut with_c = matrix(3);
        with_c.add(0, 1, 500.0, 250.0);
        with_c.add(1, 2, 500.0, 100.0); // B is thrashed by C
        with_c.add(0, 2, 500.0, 250.0);

        let mut without_c = matrix(3);
        without_c.add(0, 1, 500.0, 250.0);

        let b_with = fit(&with_c, 0, ANCHOR).ratings[1].rating;
        let b_without = fit(&without_c, 0, ANCHOR).ratings[1].rating;
        assert!(
            (b_with - b_without).abs() > 1.0,
            "dropping a config drops its games as evidence, which is why membership \
             changes force a full refit"
        );
    }

    #[test]
    fn the_fit_converges() {
        let mut m = matrix(5);
        for i in 0..5 {
            for j in (i + 1)..5 {
                m.add(i, j, 200.0, 100.0 + (i as f64 - j as f64) * 5.0);
            }
        }
        let fit = fit(&m, 0, ANCHOR);
        assert!(fit.converged, "took {} iterations", fit.iterations);
    }
}
