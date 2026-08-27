//! Glicko-2 rating updates, applied one game (here, one game *pair*) at a time.
//!
//! Glickman's algorithm is defined over a rating period containing many games.
//! We run it with a single result per period, which is the standard way to get
//! continuously-updating ratings out of it and is what the dashboard wants.

const SCALE: f64 = 173.717_557_87;
const TAU: f64 = 0.5;
const CONVERGENCE: f64 = 1e-6;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rating {
    pub rating: f64,
    pub deviation: f64,
    pub volatility: f64,
}

impl Default for Rating {
    fn default() -> Self {
        Self { rating: 1500.0, deviation: 350.0, volatility: 0.06 }
    }
}

fn g(phi: f64) -> f64 {
    1.0 / (1.0 + 3.0 * phi * phi / (std::f64::consts::PI * std::f64::consts::PI)).sqrt()
}

fn e(mu: f64, mu_j: f64, phi_j: f64) -> f64 {
    1.0 / (1.0 + (-g(phi_j) * (mu - mu_j)).exp())
}

/// Returns the player's updated rating after `num_games` games against one
/// opponent, where `score_sum` is their total score across them (a win counting
/// 1, a draw 0.5).
///
/// Glicko-2 is defined over a rating period containing many games, so this is
/// the algorithm's native form — applying the single-game update repeatedly
/// would shrink the rating deviation once per game and overstate confidence.
pub fn update(player: Rating, opponent: Rating, score_sum: f64, num_games: f64) -> Rating {
    if num_games <= 0.0 {
        return player;
    }
    let mu = (player.rating - 1500.0) / SCALE;
    let phi = player.deviation / SCALE;
    let sigma = player.volatility;
    let mu_j = (opponent.rating - 1500.0) / SCALE;
    let phi_j = opponent.deviation / SCALE;

    let g_j = g(phi_j);
    let e_j = e(mu, mu_j, phi_j);

    // Every game in the period is against the same opponent, so the sums in
    // Glickman's step 3 and 4 collapse to a factor of `num_games`.
    let v = 1.0 / (num_games * g_j * g_j * e_j * (1.0 - e_j));
    let delta = v * g_j * (score_sum - num_games * e_j);

    // Illinois-method root find for the new volatility (Glickman step 5).
    let a = (sigma * sigma).ln();
    let f = |x: f64| {
        let ex = x.exp();
        let num = ex * (delta * delta - phi * phi - v - ex);
        let den = 2.0 * (phi * phi + v + ex).powi(2);
        num / den - (x - a) / (TAU * TAU)
    };

    let mut big_a = a;
    let mut big_b = if delta * delta > phi * phi + v {
        (delta * delta - phi * phi - v).ln()
    } else {
        let mut k = 1.0;
        while f(a - k * TAU) < 0.0 {
            k += 1.0;
        }
        a - k * TAU
    };

    let mut f_a = f(big_a);
    let mut f_b = f(big_b);
    while (big_b - big_a).abs() > CONVERGENCE {
        let c = big_a + (big_a - big_b) * f_a / (f_b - f_a);
        let f_c = f(c);
        if f_c * f_b <= 0.0 {
            big_a = big_b;
            f_a = f_b;
        } else {
            f_a /= 2.0;
        }
        big_b = c;
        f_b = f_c;
    }

    let new_sigma = (big_a / 2.0).exp();
    let phi_star = (phi * phi + new_sigma * new_sigma).sqrt();
    let new_phi = 1.0 / (1.0 / (phi_star * phi_star) + 1.0 / v).sqrt();
    let new_mu = mu + new_phi * new_phi * g_j * (score_sum - num_games * e_j);

    Rating {
        rating: SCALE * new_mu + 1500.0,
        deviation: SCALE * new_phi,
        volatility: new_sigma,
    }
}
