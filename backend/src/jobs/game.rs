use super::handler::*;
use crate::error::AppResult;
use crate::models::job::GameConfig;
use sqlx::{PgConnection, Row};
use uuid::Uuid;

pub struct GameHandler;

impl JobHandler for GameHandler {
    type Request = GameRequest;
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
        super::load_game_request(conn, task_id, false).await
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

/// On-demand task creation for a `games` job.
///
/// MAGPIE plays seeds S..S+N-1 for a batch of N starting at S, so consecutive
/// tasks are spaced `games_per_batch` apart and the seed space tiles with
/// neither gaps nor overlaps. Two workers racing here both compute the same
/// next seed; the `(job_id, seed)` unique index makes one of them lose, and the
/// loser retries.
pub async fn next_request(
    conn: &mut PgConnection,
    job_id: Uuid,
    config: &GameConfig,
) -> AppResult<(i64, GameRequest)> {
    let next_seed = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT MAX(seed) FROM tasks WHERE job_id = $1",
    )
    .bind(job_id)
    .fetch_one(&mut *conn)
    .await?
    .map(|max| max + config.games_per_batch as i64)
    .unwrap_or(1);

    let player1 = super::load_player_spec(conn, config.player1_config_id).await?;
    let player2 = super::load_player_spec(conn, config.player2_config_id).await?;

    Ok((
        next_seed,
        GameRequest {
            lexicon: config.lexicon.clone(),
            variant: config.variant.clone(),
            seed: next_seed as u64,
            num_games: config.games_per_batch,
            game_pairs: false,
            player1,
            player2,
        },
    ))
}

pub(super) async fn load_game_request_row(
    conn: &mut PgConnection,
    task_id: Uuid,
) -> AppResult<sqlx::postgres::PgRow> {
    Ok(sqlx::query(
        "SELECT lexicon, variant, seed, num_games, player1_config_id, player2_config_id
         FROM game_requests WHERE task_id = $1",
    )
    .bind(task_id)
    .fetch_one(conn)
    .await?)
}

pub(super) fn seed_from_row(row: &sqlx::postgres::PgRow) -> u64 {
    row.get::<i64, _>("seed") as u64
}
