use super::handler::*;
use crate::error::AppResult;
use crate::models::job::GamePairConfig;
use sqlx::PgConnection;
use uuid::Uuid;

pub struct GamePairHandler;

impl JobHandler for GamePairHandler {
    type Request = GameRequest;
    /// Identical to `games`: a pair is two plain per-game results, one per
    /// ordering. Pair-level outcome is derived at read time, never stored.
    type Response = GameResultsResponse;
    type Record = GameResultsRecord;

    fn creation_strategy() -> CreationStrategy {
        CreationStrategy::OnDemand
    }

    async fn insert_request(
        conn: &mut PgConnection,
        task_id: Uuid,
        req: &Self::Request,
    ) -> AppResult<()> {
        super::insert_game_request(conn, task_id, req).await
    }

    async fn load_request(conn: &mut PgConnection, task_id: Uuid) -> AppResult<Self::Request> {
        super::load_game_request(conn, task_id, true).await
    }

    fn process_response(response: Self::Response) -> AppResult<Self::Record> {
        Ok(GameResultsRecord { games: response.games })
    }

    async fn insert_record(
        conn: &mut PgConnection,
        task_id: Uuid,
        claim_id: Uuid,
        record: &Self::Record,
    ) -> AppResult<()> {
        super::insert_game_records(conn, task_id, claim_id, record).await
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
            player1,
            player2,
        },
    ))
}
