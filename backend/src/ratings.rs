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

/// Apply every pair contained in one accepted submission, oldest pair first.
pub async fn apply_claim(
    conn: &mut PgConnection,
    job_id: Uuid,
    claim_id: Uuid,
    config: &GamePairConfig,
) -> AppResult<()> {
    // Player 1's score in each pair: their result in the first ordering plus
    // their result in the second, where the two players have swapped seats.
    let pairs = sqlx::query(
        "SELECT game_index / 2 AS pair_index,
                SUM(CASE
                    WHEN game_index % 2 = 0 THEN
                        CASE winner WHEN 1 THEN 1.0 WHEN 2 THEN 0.0 ELSE 0.5 END
                    ELSE
                        CASE winner WHEN 2 THEN 1.0 WHEN 1 THEN 0.0 ELSE 0.5 END
                END)::float8 AS score,
                COUNT(*) AS games
         FROM game_records
         WHERE task_claim_id = $1
         GROUP BY 1
         HAVING COUNT(*) = 2
         ORDER BY 1",
    )
    .bind(claim_id)
    .fetch_all(&mut *conn)
    .await?;

    if pairs.is_empty() {
        return Ok(());
    }

    let mut r1 = load_rating(conn, job_id, config.player1_config_id).await?;
    let mut r2 = load_rating(conn, job_id, config.player2_config_id).await?;

    for row in &pairs {
        // A pair is worth 2 points; >1 is a pair win for player 1.
        let raw: f64 = row.get("score");
        let score1 = if raw > 1.0 {
            1.0
        } else if raw < 1.0 {
            0.0
        } else {
            0.5
        };
        let (next1, next2) = (
            glicko::update(r1, r2, score1),
            glicko::update(r2, r1, 1.0 - score1),
        );
        r1 = next1;
        r2 = next2;
    }

    let count = pairs.len() as i32;
    store_rating(conn, job_id, config.player1_config_id, r1, count).await?;
    store_rating(conn, job_id, config.player2_config_id, r2, count).await?;
    Ok(())
}
