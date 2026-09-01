//! Glicko-2 bookkeeping for game-pair jobs. Ratings are keyed by
//! `(player_config_id, job_id)`: each job is an independent experiment, and
//! pooling ratings across jobs would conflate different conditions.

use crate::error::AppResult;
use crate::models::job::GamePairConfig;
use crate::stats::glicko::{self, Rating};
use sqlx::{PgConnection, Row};
use uuid::Uuid;

/// A player config with no simulation parameters is the static bot, which the
/// plan seeds at 2000; everything else starts at the Glicko default.
async fn seed_rating(conn: &mut PgConnection, player_config_id: Uuid) -> AppResult<Rating> {
    let is_static = sqlx::query_scalar::<_, bool>(
        "SELECT max_iterations IS NULL FROM player_configs WHERE id = $1",
    )
    .bind(player_config_id)
    .fetch_one(&mut *conn)
    .await?;

    Ok(Rating { rating: if is_static { 2000.0 } else { 1500.0 }, ..Rating::default() })
}

async fn load_rating(
    conn: &mut PgConnection,
    job_id: Uuid,
    player_config_id: Uuid,
) -> AppResult<Rating> {
    let existing = sqlx::query(
        "SELECT rating, rating_deviation, volatility
         FROM player_config_ratings WHERE job_id = $1 AND player_config_id = $2",
    )
    .bind(job_id)
    .bind(player_config_id)
    .fetch_optional(&mut *conn)
    .await?;

    Ok(match existing {
        Some(row) => Rating {
            rating: row.get("rating"),
            deviation: row.get("rating_deviation"),
            volatility: row.get("volatility"),
        },
        None => seed_rating(conn, player_config_id).await?,
    })
}

async fn store_rating(
    conn: &mut PgConnection,
    job_id: Uuid,
    player_config_id: Uuid,
    rating: Rating,
    pairs: i32,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO player_config_ratings
             (player_config_id, job_id, rating, rating_deviation, volatility, games_played)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (player_config_id, job_id) DO UPDATE SET
             rating = excluded.rating,
             rating_deviation = excluded.rating_deviation,
             volatility = excluded.volatility,
             games_played = player_config_ratings.games_played + excluded.games_played,
             updated_at = now()",
    )
    .bind(player_config_id)
    .bind(job_id)
    .bind(rating.rating)
    .bind(rating.deviation)
    .bind(rating.volatility)
    .bind(pairs)
    .execute(conn)
    .await?;
    Ok(())
}

/// Apply one accepted submission's divergent games to both players' ratings.
///
/// The divergent subset is the whole signal: pairs whose two games played
/// identically are guaranteed ties and move nobody's rating. Applied as a
/// single Glicko-2 rating period rather than game by game, which is the
/// algorithm's native form.
pub async fn apply_claim(
    conn: &mut PgConnection,
    job_id: Uuid,
    claim_id: Uuid,
    config: &GamePairConfig,
) -> AppResult<()> {
    let row = sqlx::query(
        "SELECT divergent_games, divergent_wins, divergent_ties
         FROM game_results WHERE task_claim_id = $1",
    )
    .bind(claim_id)
    .fetch_optional(&mut *conn)
    .await?;

    let Some(row) = row else { return Ok(()) };
    let games: Option<i32> = row.get("divergent_games");
    let (Some(games), Some(wins), Some(ties)) = (
        games,
        row.get::<Option<i32>, _>("divergent_wins"),
        row.get::<Option<i32>, _>("divergent_ties"),
    ) else {
        return Ok(());
    };
    if games <= 0 {
        return Ok(());
    }

    let r1 = load_rating(conn, job_id, config.player1_config_id).await?;
    let r2 = load_rating(conn, job_id, config.player2_config_id).await?;

    let games = games as f64;
    let score1 = wins as f64 + 0.5 * ties as f64;
    let next1 = glicko::update(r1, r2, score1, games);
    let next2 = glicko::update(r2, r1, games - score1, games);

    // Ratings are counted in pairs, matching how the dashboard and the job's
    // min/max thresholds talk about progress.
    let pairs = (games / 2.0).round() as i32;
    store_rating(conn, job_id, config.player1_config_id, next1, pairs).await?;
    store_rating(conn, job_id, config.player2_config_id, next2, pairs).await?;
    Ok(())
}
