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

/// Virtual drawn games a config with no games plays against the pool's centre;
/// fewer as it plays more (`PRIOR_FADE_GAMES`).
///
/// The maximum likelihood alone is unbounded whenever a group of configs never
/// lost a point to the rest of its pool, or never took one from it — an
/// undefeated new bot is not an edge case, it is what a strong config's first
/// job looks like — and undefined for a config with no games. So every config,
/// the anchor included, plays up to `PRIOR_GAMES` virtual drawn games against a
/// virtual config at the pool's centre, the plain mean of every config's
/// strength, on a scale `PRIOR_SCALE` times as wide as real games' (see there).
/// (A centre fitted as a strength of its own followed the configs whose
/// virtual games had not faded: in a mature pool, a newcomer's own, so its
/// first job was barely shrunk.) The
/// objective stays strictly concave: one answer, continuous in the scores, and
/// conceding a point lowers a config's rating against its opponent's (its own
/// moves the same way too, all but about once in 500 fuzzed cases, and then by
/// under an Elo while its opponent's moves further the right way). The pull is toward the pool's
/// centre, not the anchor, so a field or group far from the anchor is not
/// dragged back to it; and it joins configs only through the centre, never to
/// each other, so configs that everyone swept tie no two others together.
///
/// Every prior this fit had before pulled some shape of pool where the
/// evidence was fine (KL-74, and the thirty-second audit's adversarial checks):
/// two draws per config against the anchor, a field or thinly linked group far
/// from it (a 30-member group 200 Elo low); two per config spread over its
/// opponents, a config over a gauntlet of lightly played ones; draws only where
/// the maximum likelihood diverges, a jump of 200 Elo when a newcomer conceded
/// a quarter point; and Firth's penalty, which is not concave, two answers for
/// a config between far-apart opponents.
const PRIOR_GAMES: f64 = 2.0;

/// How much wider the virtual games' logistic is than real games': 2 is about
/// 350 Elo, against 174. A config's pull toward the centre levels off, however
/// far it is, at its virtual games over `2 · PRIOR_SCALE` of a game, and along
/// a thin chain of configs — a ladder, each played only against the next —
/// those pulls add up; at the real games' scale and unfaded they compressed a
/// 12-rung ladder 100 Elo apart by some 190 Elo at its top, where at this scale
/// it was some 60 before the fade and some 40 after (the audit's Monte Carlo,
/// against the old prior's 390). Wider still shrinks a barely played config
/// less, and it swings more: this is where the two costs meet.
const PRIOR_SCALE: f64 = 2.0;

/// The games at which a config's virtual games are halved: a config with `g`
/// real ones plays `PRIOR_GAMES / (1 + g / PRIOR_FADE_GAMES)`. The prior is
/// for the barely played; left whole on the well played, its pulls — levelling
/// off at a constant each, however far a config is from the centre — added up
/// across any group far from the centre and joined to it thinly: twelve strong
/// configs at +1,000 over one 300-pair link, 345 Elo low, nearly four errors
/// (the audit's adversarial check). Faded at 200, that group is some 100 low,
/// about half an error, and a newcomer's first few pairs are shrunk almost as
/// before. It depends only on the game counts, so the objective stays concave.
const PRIOR_FADE_GAMES: f64 = 200.0;

/// The fewest virtual games a config keeps, however much it has played. The
/// virtual games are all that hold a block that swept, or was swept by,
/// everything outside it — an anchor that never took a point — to the rest of
/// the pool; at a twentieth of a game each, such a block floated thousands of
/// Elo on unrelated configs' pulls, with every error infinite (the audit's
/// adversarial check). A fifth holds it, at the price of a stronger pull on a
/// strong tier joined thinly (KL-79).
const PRIOR_FLOOR_GAMES: f64 = 0.2;

/// Newton steps before giving up. A step solves the whole pool at once, so a
/// pool converges in a handful of steps, or some dozens when `MAX_STEP` binds;
/// the limit only bounds a pathology.
const MAX_ITERATIONS: usize = 1_000;
/// Largest full Newton step in any log-strength (natural-log units; 1e-6 is
/// about 2e-4 Elo) below which the fit has converged. Measured on the full
/// step, not the accepted one: rounding in an objective of hundreds of
/// thousands (head-to-heads of a million pairs) can make a line search accept
/// a tiny fraction of a step that is not tiny.
const CONVERGENCE: f64 = 1e-6;
/// Within this radius (natural-log units, about 87 Elo) the full Newton step is
/// taken without the line search: that close, rounding in the objective rather
/// than its curvature is what would reject it, and a fit at the answer then
/// crept at 1/512 of a step until it ran out of iterations, marked unconverged.
const FULL_STEP_RADIUS: f64 = 0.5;
/// The most any config's log-strength may move in one step: 8 is about 1,400
/// Elo. The line search asks only that the whole objective improve, so an
/// unbounded step could throw one weakly held config thousands of Elo away
/// while the others' gains paid for it, saturating every head-to-head it has
/// and leaving the curvature singular (thirty-second audit). Capped, a long
/// chain of clean sweeps still converges, in more steps.
const MAX_STEP: f64 = 8.0;

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
}

/// One player's fitted rating.
#[derive(Debug, Clone, Copy)]
pub struct Rated {
    pub rating: f64,
    /// Approximate standard error in Elo, from the inverse of the full Fisher
    /// information over the anchor's component, so a config's error includes
    /// the uncertainty of every link between it and the anchor, widened by the
    /// prior's pull on it (see `standard_errors`). Two approximations remain:
    /// - counting each unit of `games` as one Bernoulli trial makes it wider
    ///   when a unit is more than one game. The ratings feed one paired game
    ///   (two games, scored in quarters) per unit, whose variance is
    ///   `p(1-p)(1+ρ)/2` for a correlation ρ between the pair's two games, so
    ///   it reads `√(2/(1+ρ))` wide: √2 for independent games, more when
    ///   pairing works (ρ < 0), less when one config wins both halves (ρ > 0),
    ///   never below 1;
    /// - the prior's virtual games are in the fit but not in the information,
    ///   which widens it most for the barely-played.
    ///
    /// Infinite for a config with no games or outside the anchor's component.
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

/// Fits ratings by Newton's method on the log-strengths, holding `anchor` at
/// `anchor_rating`.
///
/// The objective is the log-likelihood plus the virtual games against the
/// pool's centre ([`PRIOR_GAMES`]), which is strictly concave, so it has one
/// answer; Newton with a bounded step (`MAX_STEP`) and a backtracking line
/// search reaches it, quadratically once near. Each step solves for every
/// config at once through the full curvature, which is what a group moving
/// together needs: minorization-maximization, which updated one config at a
/// time, crept toward such a group's answer and stopped at its iteration cap
/// short of it (KL-74). A step costs a Cholesky factorisation of an `n × n`
/// matrix, so a hundred-member pool fits in milliseconds.
pub fn fit(matrix: &Matrix, anchor: usize, anchor_rating: f64) -> Fit {
    let n = matrix.len();
    let component = components(matrix);
    let anchor_component = component.get(anchor).copied().unwrap_or(0);

    // The real games, each head-to-head with games in it once:
    // `(i, j, games, i's score)`.
    let played: Vec<(usize, usize, f64, f64)> = (0..n)
        .flat_map(|i| ((i + 1)..n).map(move |j| (i, j)))
        .filter_map(|(i, j)| {
            let cell = matrix.get(i, j);
            (cell.games > 0.0).then_some((i, j, cell.games, cell.score))
        })
        .collect();
    // Each config's virtual drawn games against the centre, the plain mean of
    // every config's strength, the anchor's included.
    let virtual_games: Vec<f64> = (0..n)
        .map(|i| {
            let faded = PRIOR_GAMES / (1.0 + matrix.games_played(i) / PRIOR_FADE_GAMES);
            faded.max(PRIOR_FLOOR_GAMES)
        })
        .collect();
    let slope = 1.0 / PRIOR_SCALE;
    let mean = |theta: &[f64]| theta.iter().sum::<f64>() / n as f64;

    // Unknowns are every config but the anchor. Log-strengths are relative to
    // the anchor's, which is 0: absolute ones (up to ±57 for a ±10,000 anchor)
    // cost precision in the objective for nothing.
    let free: Vec<usize> = (0..n).filter(|&i| i != anchor).collect();
    let m = free.len();
    let slot: Vec<Option<usize>> = {
        let mut slot = vec![None; n];
        for (k, &i) in free.iter().enumerate() {
            slot[i] = Some(k);
        }
        slot
    };
    let mut theta = vec![0.0; n];

    let objective = |theta: &[f64]| -> f64 {
        let real: f64 = played
            .iter()
            .map(|&(i, j, games, score)| {
                let d = theta[i] - theta[j];
                score * d - games * log_sum_exp(d, 0.0)
            })
            .sum();
        let centre = mean(theta);
        let prior: f64 = (0..n)
            .map(|i| {
                let d = slope * (theta[i] - centre);
                virtual_games[i] * (d / 2.0 - log_sum_exp(d, 0.0))
            })
            .sum();
        real + prior
    };
    // The gradient over the free strengths, and the negated Hessian: positive
    // definite, since with the anchor held the centre's pulls reach every
    // direction.
    let derivatives = |theta: &[f64]| -> (Vec<f64>, Vec<f64>) {
        let mut gradient = vec![0.0; m];
        let mut curvature = vec![0.0; m * m];
        for &(i, j, games, score) in &played {
            let p = win_probability(theta[i], theta[j]);
            let surplus = score - games * p;
            if let Some(a) = slot[i] {
                gradient[a] += surplus;
            }
            if let Some(b) = slot[j] {
                gradient[b] -= surplus;
            }
            add_pair(&mut curvature, m, slot[i], slot[j], games * p * (1.0 - p));
        }
        // A config's virtual games move with `θi − mean(θ)`: along
        // `v_i = e_i − 1/n` over the free strengths. Summed over configs,
        // `Σ c_i v_i v_iᵀ` is a diagonal and three constant terms, so this is
        // O(m²), not O(n·m²): `c_a δ_ab − (c_a + c_b)/n + Σc/n²`.
        let centre = mean(theta);
        let share = 1.0 / n as f64;
        let mut pulls = vec![0.0; n];
        let mut weights = vec![0.0; n];
        for i in 0..n {
            let p = win_probability(slope * theta[i], slope * centre);
            pulls[i] = slope * virtual_games[i] * (0.5 - p);
            weights[i] = slope * slope * virtual_games[i] * p * (1.0 - p);
        }
        let (pull_sum, weight_sum): (f64, f64) = (pulls.iter().sum(), weights.iter().sum());
        for a in 0..m {
            gradient[a] += pulls[free[a]] - share * pull_sum;
            for b in 0..m {
                let diagonal = if a == b { weights[free[a]] } else { 0.0 };
                curvature[a * m + b] += diagonal - share * (weights[free[a]] + weights[free[b]])
                    + share * share * weight_sum;
            }
        }
        (gradient, curvature)
    };

    let mut iterations = 0;
    let mut converged = free.is_empty();
    let mut current = objective(&theta);
    while !converged && iterations < MAX_ITERATIONS {
        iterations += 1;
        let (gradient, curvature) = derivatives(&theta);
        // Positive definite in exact arithmetic; if rounding says otherwise,
        // damped towards a gradient step until it factors.
        let Some(step) = damped_newton_step(&curvature, m, &gradient) else { break };
        let largest = step.iter().fold(0.0f64, |a, d| a.max(d.abs()));
        // Or the step promises nothing: `g · step` is twice the gain a full
        // step predicts. A config far out on a flat objective can keep a step
        // above `CONVERGENCE` that rounding will not let shrink, at the answer.
        let promised: f64 = gradient.iter().zip(&step).map(|(g, d)| g * d).sum();
        if largest < CONVERGENCE || promised < 1e-12 {
            converged = true;
            break;
        }
        let mut scale = if largest > MAX_STEP { MAX_STEP / largest } else { 1.0 };
        let gain: f64 = gradient.iter().zip(&step).map(|(g, d)| g * d).sum();

        // Backtrack until the step gains what its slope promises, except that
        // a small full step is taken as it is (`FULL_STEP_RADIUS`).
        let mut accepted = false;
        for _ in 0..60 {
            let mut trial = theta.clone();
            for (k, &i) in free.iter().enumerate() {
                trial[i] += scale * step[k];
            }
            let value = objective(&trial);
            let near = scale == 1.0 && largest < FULL_STEP_RADIUS;
            if value.is_finite() && (near || value >= current + 1e-4 * scale * gain) {
                theta = trial;
                current = value;
                accepted = true;
                break;
            }
            scale /= 2.0;
        }
        // No step along a Newton direction gains anything: the objective is
        // flat to rounding here. Converged only if the full step was already
        // negligible, measured in Elo rather than in the shrunken step.
        if !accepted {
            converged = largest * ELO_PER_LN < 1e-3;
            break;
        }
    }

    // The prior's pull on each strength at the answer: `slope · c_i (½ − p_i)`
    // along `e_i − 1/n`, as in `derivatives`.
    let prior_gradient: Vec<f64> = {
        let centre = mean(&theta);
        let pulls: Vec<f64> = (0..n)
            .map(|i| {
                let p = win_probability(slope * theta[i], slope * centre);
                slope * virtual_games[i] * (0.5 - p)
            })
            .collect();
        let average = pulls.iter().sum::<f64>() / n as f64;
        pulls.iter().map(|pull| pull - average).collect()
    };
    let stderr =
        standard_errors(matrix, anchor, &component, anchor_component, &theta, &prior_gradient);
    let ratings = (0..n)
        .map(|i| {
            // The anchor's rating is the input, not a round trip through its
            // strength: the pool's fixed point must be stored as stated.
            let rating = if i == anchor { anchor_rating } else { anchor_rating + theta[i] * ELO_PER_LN };
            Rated { rating, stderr: stderr[i], games: matrix.games_played(i), component: component[i] }
        })
        .collect();

    Fit { ratings, anchor_component, iterations, converged }
}

/// Adds `value` to the information between free configs `a` and `b` (`None`
/// for the anchor, whose row and column are not in the matrix): `+value` on
/// each diagonal, `-value` off it.
fn add_pair(matrix: &mut [f64], m: usize, a: Option<usize>, b: Option<usize>, value: f64) {
    if let Some(a) = a {
        matrix[a * m + a] += value;
    }
    if let Some(b) = b {
        matrix[b * m + b] += value;
    }
    if let (Some(a), Some(b)) = (a, b) {
        matrix[a * m + b] -= value;
        matrix[b * m + a] -= value;
    }
}

/// The Newton step `curvature⁻¹ · gradient`, adding `λ·I` to the curvature,
/// growing `λ` from nothing, until it factors (Levenberg–Marquardt damping).
/// `None` only if even a heavily damped matrix will not factor, which takes
/// non-finite input.
fn damped_newton_step(curvature: &[f64], m: usize, gradient: &[f64]) -> Option<Vec<f64>> {
    let scale = (0..m).map(|k| curvature[k * m + k].abs()).fold(1e-12, f64::max);
    let mut damping = 0.0;
    for _ in 0..40 {
        let mut damped = curvature.to_vec();
        for k in 0..m {
            damped[k * m + k] += damping;
        }
        if let Some(factor) = cholesky(&damped, m) {
            return Some(cholesky_solve(&factor, m, gradient));
        }
        damping = if damping == 0.0 { scale * 1e-12 } else { damping * 10.0 };
    }
    None
}

/// Standard errors in Elo from the inverse of the Fisher information over the
/// anchor's component — the full matrix, so a group's error includes the
/// uncertainty of every link between it and the anchor — widened by how far
/// the prior holds each config from where its games alone would put it.
///
/// That shift is one Newton step on the games alone from the answer:
/// `I⁻¹ · ∇prior`, since at the answer the games' gradient is the prior's,
/// negated. It is what the prior's pull costs, and the pulls add up across a
/// group joined to the rest thinly: two tiers of lightly played configs 600 Elo
/// apart and joined by one small job put the upper one nearly two errors low
/// with the games' error alone (the audit's adversarial check), and no weighting
/// of the prior removes that without moving it to another shape. Added to the
/// variance, `PULL_IN_ERROR` times over, it makes the error say when a number
/// is the prior's. A config outside the anchor's component, or with no games,
/// has no finite error.
fn standard_errors(
    matrix: &Matrix,
    anchor: usize,
    component: &[usize],
    anchor_component: usize,
    theta: &[f64],
    prior_gradient: &[f64],
) -> Vec<f64> {
    let n = matrix.len();
    let mut stderr = vec![f64::INFINITY; n];
    if anchor < n {
        stderr[anchor] = 0.0;
    }
    let members: Vec<usize> = (0..n)
        .filter(|&i| i != anchor && component[i] == anchor_component && matrix.games_played(i) > 0.0)
        .collect();
    let m = members.len();
    if m == 0 {
        return stderr;
    }
    let mut slot = vec![None; n];
    for (k, &i) in members.iter().enumerate() {
        slot[i] = Some(k);
    }
    let mut information = vec![0.0; m * m];
    for i in 0..n {
        for j in (i + 1)..n {
            let cell = matrix.get(i, j);
            if cell.games <= 0.0 {
                continue;
            }
            let p = win_probability(theta[i], theta[j]);
            let value = cell.games * p * (1.0 - p);
            if let Some(a) = slot[i] {
                information[a * m + a] += value;
            }
            if let Some(b) = slot[j] {
                information[b * m + b] += value;
            }
            if let (Some(a), Some(b)) = (slot[i], slot[j]) {
                information[a * m + b] -= value;
                information[b * m + a] -= value;
            }
        }
    }
    // Singular only if the information underflows (a mismatch of thousands of
    // Elo); the errors then stay infinite, which is what they are.
    if let Some(factor) = cholesky(&information, m) {
        let pull: Vec<f64> = members.iter().map(|&i| prior_gradient[i]).collect();
        let shift = cholesky_solve(&factor, m, &pull);
        for (k, &i) in members.iter().enumerate() {
            let mut unit = vec![0.0; m];
            unit[k] = 1.0;
            let pull = PULL_IN_ERROR * shift[k];
            let variance = cholesky_solve(&factor, m, &unit)[k] + pull * pull;
            if variance > 0.0 && variance.is_finite() {
                stderr[i] = ELO_PER_LN * variance.sqrt();
            }
        }
    }
    stderr
}

/// How many times the prior's one-step shift goes into a config's error. One
/// Newton step underestimates a pull that has saturated, the more so the
/// further apart a thinly joined group is: at 1, tiers of lightly played configs
/// 800 Elo apart had their 95% interval cover the truth 64 to 86% of the time;
/// at 1.5, 95 to 100%, at 600 Elo as at 800 (the audit's adversarial check).
/// Most errors grow a few percent; a thinly held config's, or a newcomer's
/// clean sweep's, by a fifth to a third.
const PULL_IN_ERROR: f64 = 1.5;

/// Probability that strength `a` beats strength `b`, in log-strength units.
fn win_probability(a: f64, b: f64) -> f64 {
    1.0 / (1.0 + (b - a).exp())
}

/// `ln(e^a + e^b)` without overflow.
fn log_sum_exp(a: f64, b: f64) -> f64 {
    let high = a.max(b);
    high + (-(a - b).abs()).exp().ln_1p()
}

/// Lower-triangular Cholesky factor of a symmetric `m × m` matrix, or `None`
/// if it is not positive definite.
fn cholesky(a: &[f64], m: usize) -> Option<Vec<f64>> {
    let mut l = vec![0.0; m * m];
    for i in 0..m {
        for j in 0..=i {
            let mut sum = a[i * m + j];
            for k in 0..j {
                sum -= l[i * m + k] * l[j * m + k];
            }
            if i == j {
                if sum.is_nan() || sum <= 0.0 || sum.is_infinite() {
                    return None;
                }
                l[i * m + i] = sum.sqrt();
            } else {
                l[i * m + j] = sum / l[j * m + j];
            }
        }
    }
    Some(l)
}

/// Solves `L Lᵀ x = b` given the Cholesky factor `L`.
fn cholesky_solve(l: &[f64], m: usize, b: &[f64]) -> Vec<f64> {
    let mut y = b.to_vec();
    for i in 0..m {
        for k in 0..i {
            y[i] -= l[i * m + k] * y[k];
        }
        y[i] /= l[i * m + i];
    }
    for i in (0..m).rev() {
        for k in (i + 1)..m {
            y[i] -= l[k * m + i] * y[k];
        }
        y[i] /= l[i * m + i];
    }
    y
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
        // 1.268655383138… Elo. The prior's virtual games move p by about
        // 1e-6, so this is held to 0.1%.
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

    /// Noiseless evidence: every head-to-head scores exactly what `truth`
    /// predicts, so a correct fit returns `truth` up to the prior's small pull.
    fn noiseless(truth: &[f64], pairs: &[(usize, usize, f64)]) -> Matrix {
        let mut m = matrix(truth.len());
        for &(i, j, games) in pairs {
            let p = 1.0 / (1.0 + 10f64.powf(-(truth[i] - truth[j]) / 400.0));
            m.add(i, j, games, games * p);
        }
        m
    }

    fn assert_near_truth(fit: &Fit, truth: &[f64], tolerance: f64) {
        assert!(fit.converged, "unconverged after {} iterations", fit.iterations);
        for (i, want) in truth.iter().enumerate() {
            let got = fit.ratings[i].rating;
            assert!((got - want).abs() < tolerance, "config {i}: fitted {got}, truth {want}");
        }
    }

    /// KL-74: a group joined to the anchor by one thin link. Twelve configs
    /// around anchor + 300, every pair of them played to 1,000 pairs, and one
    /// 300-pair job between one of them and the anchor. The fit used to store
    /// the group 42 Elo low with a shown error of ±1.9, unconverged; the link
    /// alone carries about ±28.
    #[test]
    fn a_thinly_linked_cluster_is_fitted_where_it_is_with_the_links_error() {
        let mut truth = vec![ANCHOR];
        truth.extend((0..12).map(|k| ANCHOR + 300.0 + 10.0 * k as f64));
        let mut pairs = vec![(0, 1, 300.0)];
        for i in 1..truth.len() {
            for j in (i + 1)..truth.len() {
                pairs.push((i, j, 1_000.0));
            }
        }
        let fit = fit(&noiseless(&truth, &pairs), 0, ANCHOR);
        assert_near_truth(&fit, &truth, 3.0);
        // The whole group moves with the link: every member's error is at
        // least the link's, (400 / ln 10) / √(300·p(1-p)) ≈ 28 Elo here.
        for rated in &fit.ratings[1..] {
            assert!(rated.stderr > 25.0, "stderr {} understates the link", rated.stderr);
        }
    }

    /// KL-74: a 20-config chain, each 50 Elo above the last and joined to it by
    /// 300 pairs. The top used to be fitted 36.5 Elo low.
    #[test]
    fn a_long_chain_reaches_its_top() {
        let truth: Vec<f64> = (0..20).map(|k| ANCHOR + 50.0 * k as f64).collect();
        let pairs: Vec<_> = (0..19).map(|k| (k, k + 1, 300.0)).collect();
        let fit = fit(&noiseless(&truth, &pairs), 0, ANCHOR);
        // The virtual games' pulls toward the centre add up along a chain:
        // up to about half the error there (KL-79).
        assert!(fit.converged);
        for (k, rated) in fit.ratings.iter().enumerate().skip(1) {
            assert!((rated.rating - truth[k]).abs() < 0.6 * rated.stderr, "config {k}: {rated:?}");
        }
        // Errors add up along the chain.
        for k in 1..19 {
            assert!(fit.ratings[k + 1].stderr > fit.ratings[k].stderr);
        }
    }

    /// KL-74: thirty configs at anchor + 400, all against each other, linked
    /// by one 300-pair job. The fit used to land about 200 Elo low.
    #[test]
    fn a_large_cluster_is_not_pulled_toward_the_anchor() {
        let mut truth = vec![ANCHOR];
        truth.extend((0..30).map(|k| ANCHOR + 400.0 + 5.0 * k as f64));
        let mut pairs = vec![(0, 1, 300.0)];
        for i in 1..truth.len() {
            for j in (i + 1)..truth.len() {
                pairs.push((i, j, 500.0));
            }
        }
        let fit = fit(&noiseless(&truth, &pairs), 0, ANCHOR);
        assert_near_truth(&fit, &truth, 3.0);
    }

    /// KL-74: a well-played island with no path to the anchor used to hold the
    /// fit at its iteration cap and mark the whole pool unconverged.
    #[test]
    fn an_island_does_not_stop_the_fit_converging() {
        let truth = [ANCHOR, ANCHOR + 100.0, ANCHOR - 50.0, ANCHOR + 500.0, ANCHOR + 700.0];
        let pairs = [(0, 1, 400.0), (1, 2, 400.0), (0, 2, 400.0), (3, 4, 5_000.0)];
        let fit = fit(&noiseless(&truth, &pairs), 0, ANCHOR);
        assert!(fit.converged, "unconverged after {} iterations", fit.iterations);
        for (rated, want) in fit.ratings.iter().zip(&truth).take(3) {
            assert!((rated.rating - want).abs() < 3.0);
        }
        assert!(!fit.is_rateable(3) && !fit.is_rateable(4));
        assert!(fit.ratings[3].stderr.is_infinite() && fit.ratings[4].stderr.is_infinite());
        // Within the island the gap is still measured, even if its level is not.
        let gap = fit.ratings[4].rating - fit.ratings[3].rating;
        assert!((gap - 200.0).abs() < 3.0, "island gap {gap}");
    }

    /// A config that won every pair of its only job gets a finite rating well
    /// above its opponent, and a wide error.
    #[test]
    fn a_clean_sweep_of_a_group_stays_finite() {
        let mut m = matrix(4);
        m.add(0, 1, 500.0, 250.0);
        m.add(1, 2, 500.0, 250.0);
        m.add(2, 3, 40.0, 0.0); // 3 won all forty
        let fit = fit(&m, 0, ANCHOR);
        assert!(fit.converged);
        assert!(fit.ratings[3].rating.is_finite() && fit.ratings[3].rating > ANCHOR + 400.0);
        assert!(fit.ratings[3].stderr > 100.0);
    }

    /// A fit of a hundred configs is fast enough to run inside a membership
    /// request.
    #[test]
    fn a_hundred_member_pool_fits_quickly() {
        let truth: Vec<f64> = (0..100).map(|k| ANCHOR + 7.0 * k as f64).collect();
        let mut pairs = Vec::new();
        for i in 0..100 {
            for j in (i + 1)..100 {
                if j == i + 1 || (i * 7 + j * 3) % 5 == 0 {
                    pairs.push((i, j, 200.0));
                }
            }
        }
        let m = noiseless(&truth, &pairs);
        let started = std::time::Instant::now();
        let fit = fit(&m, 0, ANCHOR);
        assert!(started.elapsed() < std::time::Duration::from_secs(1), "{:?}", started.elapsed());
        assert_near_truth(&fit, &truth, 3.0);
    }

    /// Clean sweeps of thousands of pairs that contradict the rest of the pool
    /// (the thirty-second audit's adversarial check, minimised). An unbounded
    /// Newton step threw one config some 18,000 Elo away while the others'
    /// gains paid for it, saturating its every head-to-head: the fit gave up
    /// unconverged at 7 iterations with ratings from -8,168 to 18,130 and
    /// every error infinite.
    #[test]
    fn contradicting_clean_sweeps_do_not_throw_the_fit_into_saturation() {
        let mut m = matrix(16);
        for (i, j, games, score) in [
            (0, 2, 50.0, 0.0),
            (0, 5, 500.0, 500.0),
            (1, 3, 5.0, 2.5),
            (1, 9, 50.0, 0.0),
            (2, 12, 500.0, 500.0),
            (2, 15, 5000.0, 0.0),
            (3, 10, 20000.0, 119.75),
            (4, 6, 20000.0, 20000.0),
            (4, 14, 5000.0, 6.0),
            (5, 13, 500.0, 64.5),
            (5, 14, 5000.0, 0.0),
            (6, 10, 500.0, 488.5),
            (6, 11, 5.0, 0.0),
            (7, 9, 50.0, 0.0),
            (7, 13, 5000.0, 0.0),
            (7, 15, 20000.0, 0.0),
            (8, 12, 5000.0, 5000.0),
        ] {
            m.add(i, j, games, score);
        }
        let fit = fit(&m, 11, ANCHOR);
        assert!(fit.converged, "unconverged after {} iterations", fit.iterations);
        for (i, rated) in fit.ratings.iter().enumerate() {
            // Config 15 swept 25,000 pairs, and config 7 lost 25,050 without
            // scoring: thousands of Elo from the rest, but finite and
            // converged, where saturation reached -8,168 and 18,130.
            assert!((-4_000.0..8_000.0).contains(&rated.rating), "config {i} at {}", rated.rating);
            if fit.is_rateable(i) {
                assert!(rated.stderr.is_finite(), "config {i}'s error");
            }
        }
    }

    /// A newcomer's clean sweep of its only job is finite, and rates higher the
    /// more pairs it swept.
    #[test]
    fn a_newcomers_clean_sweep_rates_higher_the_more_it_swept() {
        let rating = |pairs: f64| {
            let mut star = matrix(17);
            for leaf in 1..16 {
                star.add(0, leaf, 20.0, 10.0);
            }
            star.add(0, 16, pairs, 0.0);
            let fit = fit(&star, 0, ANCHOR);
            assert!(fit.converged);
            fit.ratings[16].rating
        };
        let [few, more, most] = [5.0, 20.0, 100.0].map(rating);
        assert!(ANCHOR < few && few < more && more < most && most.is_finite(), "{few} {more} {most}");
    }

    /// A well-played head-to-head is its maximum likelihood, the prior's pull
    /// a fraction of an Elo: 75% over 100 pairs is 400·log10(3) above.
    #[test]
    fn a_well_played_head_to_head_is_its_maximum_likelihood() {
        let mut m = matrix(2);
        m.add(0, 1, 100.0, 25.0);
        let got = fit(&m, 0, ANCHOR).ratings[1].rating;
        let want = ANCHOR + 400.0 * 3f64.log10();
        assert!((got - want).abs() < 1.0, "{got} against {want}");
    }

    /// KL-74, again: a strong config rated by a gauntlet of lightly played
    /// opponents. Twenty configs level with the anchor over 100 pairs each,
    /// and a config 600 Elo above them playing five pairs against each. A
    /// prior of virtual draws on every head-to-head it played — whatever the
    /// split — held it 2 to 4 standard errors low.
    #[test]
    fn a_gauntlet_of_lightly_played_opponents_does_not_hold_a_config_back() {
        let mut truth = vec![ANCHOR; 21];
        truth.push(ANCHOR + 600.0);
        let mut pairs: Vec<_> = (1..=20).map(|k| (0, k, 100.0)).collect();
        pairs.extend((1..=20).map(|k| (k, 21, 5.0)));
        let fit = fit(&noiseless(&truth, &pairs), 0, ANCHOR);
        assert!(fit.converged);
        let hub = fit.ratings[21];
        // The prior's pull on 100 pairs at 97%, well inside the error; the
        // priors placed on its head-to-heads held it 2 to 4 errors low.
        assert!((hub.rating - truth[21]).abs() < 0.5 * hub.stderr, "{} ± {}", hub.rating, hub.stderr);
    }

    /// With head-to-heads of a million pairs, rounding in the objective made
    /// the line search accept 1/512 of a step that never shrank, and a fit at
    /// the answer ran out of iterations marked unconverged (the thirty-second
    /// audit's adversarial check, a 13-config case of its fuzzing).
    #[test]
    fn a_fit_at_the_answer_with_huge_head_to_heads_says_it_converged() {
        let mut m = matrix(13);
        for (i, j, games, score) in [
            (1, 2, 500.0, 497.75),
            (1, 10, 1.0, 0.0),
            (3, 10, 5000.0, 3051.25),
            (4, 9, 1_000_000.0, 617_813.25),
            (4, 10, 50.0, 38.25),
            (4, 12, 20000.0, 5051.0),
            (5, 7, 500.0, 15.75),
            (7, 10, 5.0, 5.0),
            (7, 12, 1_000_000.0, 934_014.25),
            (8, 12, 20000.0, 1227.25),
            (9, 11, 2.0, 0.25),
        ] {
            m.add(i, j, games, score);
        }
        let fit = fit(&m, 3, 0.0);
        assert!(fit.converged, "unconverged after {} iterations", fit.iterations);
    }

    /// Conceding a quarter of a point must not raise a rating. A prior switched
    /// on for a clean sweep and off for anything less rated a 19.75-of-20
    /// newcomer 230 Elo above a 20-of-20 one.
    #[test]
    fn a_clean_sweep_rates_at_least_a_near_sweep() {
        for pairs in [3.0, 20.0, 100.0] {
            let mut m = matrix(9);
            for i in 1..6 {
                m.add(0, i, 50.0, 25.0);
            }
            m.add(0, 6, pairs, 0.0);
            m.add(0, 7, pairs, 0.25);
            m.add(0, 8, pairs, 0.5);
            let fit = fit(&m, 0, ANCHOR);
            let [sweep, near, nearer] = [6, 7, 8].map(|i| fit.ratings[i].rating);
            assert!(sweep > near && near > nearer, "{pairs} pairs: {sweep} {near} {nearer}");
        }
    }

    /// Configs that every rated config swept say little about how the rated
    /// ones compare: twenty baselines each losing 10 of 10 pairs to both the
    /// anchor and a config 400 above it must not tie that config to the anchor.
    #[test]
    fn shared_swept_baselines_do_not_pull_a_config_toward_the_anchor() {
        let mut m = matrix(22);
        let p = 1.0 / (1.0 + 10f64.powf(-1.0));
        m.add(0, 1, 50.0, 50.0 * (1.0 - p));
        for i in 2..22 {
            m.add(0, i, 10.0, 10.0);
            m.add(1, i, 10.0, 10.0);
        }
        let fit = fit(&m, 0, ANCHOR);
        let hub = fit.ratings[1];
        assert!(fit.converged);
        assert!((hub.rating - 2400.0).abs() < hub.stderr, "{} ± {}", hub.rating, hub.stderr);
    }

    /// A ladder, each config played only against the next: twelve rungs 100
    /// Elo apart over 100 pairs each. Every config's pull toward the centre
    /// adds up along it; the old prior, at the real games' scale and toward the
    /// anchor, held the top some 390 Elo low in the audit's Monte Carlo, three
    /// times its error.
    #[test]
    fn a_ladder_is_not_compressed_past_its_errors() {
        let truth: Vec<f64> = (0..12).map(|k| ANCHOR + 100.0 * k as f64).collect();
        let pairs: Vec<_> = (0..11).map(|k| (k, k + 1, 100.0)).collect();
        let fit = fit(&noiseless(&truth, &pairs), 0, ANCHOR);
        assert!(fit.converged);
        // Some tens of Elo low, well inside the top's error of ±120: the
        // cost of the prior, recorded in KL-79.
        let top = fit.ratings[11];
        assert!((top.rating - truth[11]).abs() < 0.6 * top.stderr, "{top:?}");
    }

    /// Two tiers: the anchor and twenty configs within 150 Elo of it, all
    /// against each other over 100 pairs, and twelve strong configs 1,000 Elo
    /// above, against each other over 1,000, joined by one 300-pair job. A
    /// pull toward the pool's centre left whole on well played configs held
    /// the strong tier 345 Elo low, nearly four errors (the audit's
    /// adversarial check); faded with their games, it is well inside one.
    #[test]
    fn a_strong_tier_joined_thinly_is_not_pulled_to_the_centre() {
        let mut truth = vec![ANCHOR];
        truth.extend((0..20).map(|k| ANCHOR - 150.0 + 15.0 * k as f64));
        truth.extend((0..12).map(|k| ANCHOR + 1_000.0 + 10.0 * k as f64));
        let mut pairs = vec![(0, 21, 300.0)];
        for i in 0..21 {
            for j in (i + 1)..21 {
                pairs.push((i, j, 100.0));
            }
        }
        for i in 21..33 {
            for j in (i + 1)..33 {
                pairs.push((i, j, 1_000.0));
            }
        }
        let fit = fit(&noiseless(&truth, &pairs), 0, ANCHOR);
        assert!(fit.converged);
        for (k, rated) in fit.ratings.iter().enumerate().skip(21) {
            assert!((rated.rating - truth[k]).abs() < 0.5 * rated.stderr, "config {k}: {rated:?}");
        }
    }

    /// A newcomer's clean sweep is shrunk the same however much the rest of
    /// the pool has played. A centre fitted from the configs' faded virtual
    /// games followed the newcomer once the others had played thousands of
    /// pairs, and its 3-pair sweep rose 460 Elo with its own evidence
    /// unchanged (the audit's adversarial check).
    #[test]
    fn a_newcomer_is_shrunk_the_same_in_a_young_pool_as_a_mature_one() {
        let newcomer = |established: f64| {
            let mut m = matrix(11);
            for i in 0..10 {
                for j in (i + 1)..10 {
                    m.add(i, j, established, established / 2.0);
                }
            }
            m.add(1, 10, 3.0, 0.0);
            let fit = fit(&m, 0, ANCHOR);
            assert!(fit.converged);
            fit.ratings[10].rating
        };
        let (young, mature) = (newcomer(10.0), newcomer(5_000.0));
        assert!((young - mature).abs() < 20.0, "young pool {young}, mature {mature}");
    }

    /// An anchor swept 5,000–0 by a config of an established field is held to
    /// that field only by virtual games. With too few, the field floated on the
    /// pulls of unrelated young configs: six scoring a twentieth against it
    /// moved it 1,600 Elo, with every error infinite (the audit's adversarial
    /// check).
    #[test]
    fn a_field_that_swept_the_anchor_does_not_float_on_young_configs() {
        let field = |young: usize| {
            let mut m = matrix(11 + young);
            m.add(0, 1, 5_000.0, 0.0);
            for i in 1..11 {
                for j in (i + 1)..11 {
                    m.add(i, j, 1_000.0, 500.0);
                }
            }
            for k in 0..young {
                m.add(2, 11 + k, 5.0, 4.75);
            }
            let fit = fit(&m, 0, ANCHOR);
            assert!(fit.converged);
            fit.ratings[1]
        };
        // A 5,000–0 sweep says only that the field is far above: its level is
        // the prior's, with an error of some ±700 to ±1,300 Elo, and the young
        // configs move it by a fraction of that, where they moved it 1,600
        // with an error of ±137,763.
        let (alone, with_young) = (field(0), field(6));
        assert!(alone.stderr < 2_000.0 && with_young.stderr < 2_000.0, "{alone:?} {with_young:?}");
        assert!((alone.rating - with_young.rating).abs() < 0.5 * alone.stderr, "{alone:?} {with_young:?}");
    }

    /// Two tiers of lightly played configs 600 or 800 Elo apart — twenty
    /// each, five pairs a head-to-head within a tier, one 20-pair job between
    /// them. The prior's pulls add up across the upper tier and hold it 260 to
    /// 430 Elo low, nearly two of the games' errors — a 95% interval that
    /// missed the truth a third of the time, and at 800 most of it — and no
    /// weighting of the prior removes that (the audit's adversarial check).
    /// The error shown includes the pull, so the interval covers the truth.
    #[test]
    fn the_error_shown_covers_what_the_prior_pulls_a_thin_tier() {
        for gap in [600.0, 800.0] {
            let mut truth = vec![ANCHOR];
            truth.extend((0..20).map(|k| ANCHOR - 100.0 + 10.0 * k as f64));
            truth.extend((0..20).map(|k| ANCHOR + gap - 100.0 + 10.0 * k as f64));
            let mut pairs = vec![(0, 21, 20.0)];
            for (low, high) in [(0, 21), (21, 41)] {
                for i in low..high {
                    for j in (i + 1)..high {
                        pairs.push((i, j, 5.0));
                    }
                }
            }
            let fit = fit(&noiseless(&truth, &pairs), 0, ANCHOR);
            assert!(fit.converged);
            for (k, rated) in fit.ratings.iter().enumerate().skip(21) {
                let off = (rated.rating - truth[k]).abs() / rated.stderr;
                assert!(off < 1.5, "gap {gap}, config {k}: {off:.2} errors off, {rated:?}");
            }
        }
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
