use super::handler::*;
use crate::error::{AppError, AppResult};
use crate::models::job::GamePairConfig;
use sqlx::PgConnection;
use std::path::Path;
use uuid::Uuid;

pub struct GamePairHandler;

impl JobHandler for GamePairHandler {
    type Request = GameRequest;
    /// Identical to `games`: a pair is two plain per-game results, one per
    /// ordering. Pair-level outcome is derived at read time, never stored.
    type Response = GameResultsResponse;
    type Record = GameResultsRecord;



    async fn load_request(conn: &mut PgConnection, task_id: Uuid,
                          _data_path: &Path) -> AppResult<Self::Request> {
        super::load_game_request(conn, task_id, true).await
    }

    fn process_response(response: Self::Response) -> AppResult<Self::Record> {
        super::game::validate_aggregate(&response.all_games, "all_games")?;

        // Every pair is two games, so an odd total means the worker ran
        // something other than what was asked for.
        if response.all_games.games == 0 || response.all_games.games % 2 != 0 {
            return Err(AppError::bad_request(
                "a game_pairs result must contain an even, non-zero number of games (two per pair)",
            ));
        }

        // The divergent subset is where a pairs job's signal lives, so a result
        // without it cannot be evaluated.
        let divergent = response.divergent_games.ok_or_else(|| {
            AppError::bad_request("a game_pairs result must report divergent_games")
        })?;
        super::game::validate_aggregate(&divergent, "divergent_games")?;
        if divergent.games % 2 != 0 || divergent.games > response.all_games.games {
            return Err(AppError::bad_request(
                "divergent_games must be even and no larger than the total games played",
            ));
        }

        let positions =
            super::game::validate_positions(response.positions, response.all_games.games)?;
        Ok(GameResultsRecord {
            all_games: response.all_games,
            divergent_games: Some(divergent),
            positions,
        })
    }

    async fn insert_record(
        conn: &mut PgConnection,
        task_id: Uuid,
        claim_id: Uuid,
        record: &Self::Record,
    ) -> AppResult<()> {
        super::insert_game_results(conn, task_id, claim_id, record).await
    }
}

/// Same seed-tiling scheme as `games`, with `pairs_per_batch` as the stride.
pub async fn next_request(
    conn: &mut PgConnection,
    job_id: Uuid,
    config: &GamePairConfig,
) -> AppResult<(i64, GameRequest)> {
    let next_seed = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT MAX(seed) FROM tasks WHERE job_id = $1",
    )
    .bind(job_id)
    .fetch_one(&mut *conn)
    .await?
    .map(|max| max + config.pairs_per_batch as i64)
    .unwrap_or(1);

    let player1 = super::load_player_spec(conn, config.player1_config_id).await?;
    let player2 = super::load_player_spec(conn, config.player2_config_id).await?;

    Ok((
        next_seed,
        GameRequest {
            lexicon: config.lexicon.clone(),
            variant: config.variant.clone(),
            seed: next_seed as u64,
            num_games: config.pairs_per_batch,
            game_pairs: true,
            capture_positions: config.capture_positions,
            capture_top_moves: config.capture_top_moves,
            player1,
            player2,
        },
    ))
}
