//! Dispatch from `JobType` to the concrete handler. Every function here matches
//! exhaustively on `JobType`, so adding a variant fails to compile until all
//! four components of the new job type exist.

use super::handler::*;
use super::{game, game_pair, leave_gen, opening_rack};
use crate::error::{AppError, AppResult};
use crate::models::job::*;
use sqlx::PgConnection;
use std::path::Path;
use uuid::Uuid;

/// The outcome of trying to get one unit of work out of a job.
pub enum Acquired {
    Task { task_id: Uuid, request: TaskRequest },
    /// This job has nothing to hand out right now; try the next one.
    NoWork,
    /// Leave generation only: the current generation is finished and must be
    /// aggregated before more tasks exist. Handled outside the transaction.
    NeedsGenerationTransition { generation: i32 },
    /// Leave generation only: all configured generations are done.
    JobFinished,
}

pub fn creation_strategy(job_type: JobType) -> CreationStrategy {
    match job_type {
        JobType::OpeningRackAnalysis => opening_rack::OpeningRackHandler::creation_strategy(),
        JobType::Games => game::GameHandler::creation_strategy(),
        JobType::GamePairs => game_pair::GamePairHandler::creation_strategy(),
        JobType::LeaveGeneration => leave_gen::LeaveGenHandler::creation_strategy(),
    }
}

pub async fn acquire(conn: &mut PgConnection, job: &Job) -> AppResult<Acquired> {
    // A task whose claim timed out drops back to `available` regardless of the
    // job's creation strategy, so re-dispatching those comes first. For games
    // this is what keeps the seed space covered: an abandoned batch is replayed
    // rather than skipped, since nothing else would ever revisit those seeds.
    if let Some(task_id) = next_available(conn, job.id).await? {
        let request = load_request(conn, job.job_type, task_id).await?;
        return Ok(Acquired::Task { task_id, request });
    }

    match creation_strategy(job.job_type) {
        CreationStrategy::PrePopulated => Ok(Acquired::NoWork),
        CreationStrategy::OnDemand => match job.job_type {
            JobType::Games => generate_games(conn, job).await,
            JobType::GamePairs => generate_game_pairs(conn, job).await,
            JobType::LeaveGeneration => generate_leave_gen(conn, job).await,
            JobType::OpeningRackAnalysis => Ok(Acquired::NoWork),
        },
    }
}

/// Pre-populated jobs draw from the pool with `FOR UPDATE SKIP LOCKED`, which is
/// what lets many workers claim concurrently without serializing on a lock.
async fn next_available(conn: &mut PgConnection, job_id: Uuid) -> AppResult<Option<Uuid>> {
    Ok(sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM tasks
         WHERE job_id = $1 AND state = 'available'
         ORDER BY created_at
         FOR UPDATE SKIP LOCKED
         LIMIT 1",
    )
    .bind(job_id)
    .fetch_optional(conn)
    .await?)
}

pub async fn load_request(
    conn: &mut PgConnection,
    job_type: JobType,
    task_id: Uuid,
) -> AppResult<TaskRequest> {
    Ok(match job_type {
        JobType::OpeningRackAnalysis => TaskRequest::OpeningRackAnalysis(
            opening_rack::OpeningRackHandler::load_request(conn, task_id).await?,
        ),
        JobType::Games => {
            TaskRequest::Games(game::GameHandler::load_request(conn, task_id).await?)
        }
        JobType::GamePairs => {
            TaskRequest::GamePairs(game_pair::GamePairHandler::load_request(conn, task_id).await?)
        }
        JobType::LeaveGeneration => TaskRequest::LeaveGeneration(
            leave_gen::LeaveGenHandler::load_request(conn, task_id).await?,
        ),
    })
}

async fn generate_games(conn: &mut PgConnection, job: &Job) -> AppResult<Acquired> {
    let config = sqlx::query_as::<_, GameConfig>("SELECT * FROM job_game_config WHERE job_id = $1")
        .bind(job.id)
        .fetch_one(&mut *conn)
        .await?;

    let (seed, request) = game::next_request(conn, job.id, &config).await?;
    let task_id = insert_on_demand_task(conn, job.id, Some(seed)).await?;
    game::GameHandler::insert_request(conn, task_id, &request).await?;
    Ok(Acquired::Task { task_id, request: TaskRequest::Games(request) })
}

async fn generate_game_pairs(conn: &mut PgConnection, job: &Job) -> AppResult<Acquired> {
    let config =
        sqlx::query_as::<_, GamePairConfig>("SELECT * FROM job_game_pair_config WHERE job_id = $1")
            .bind(job.id)
            .fetch_one(&mut *conn)
            .await?;

    let (seed, request) = game_pair::next_request(conn, job.id, &config).await?;
    let task_id = insert_on_demand_task(conn, job.id, Some(seed)).await?;
    game_pair::GamePairHandler::insert_request(conn, task_id, &request).await?;
    Ok(Acquired::Task { task_id, request: TaskRequest::GamePairs(request) })
}

async fn generate_leave_gen(conn: &mut PgConnection, job: &Job) -> AppResult<Acquired> {
    let config =
        sqlx::query_as::<_, LeaveConfig>("SELECT * FROM job_leave_config WHERE job_id = $1")
            .bind(job.id)
            .fetch_one(&mut *conn)
            .await?;

    match leave_gen::next_step(conn, job.id, &config).await? {
        leave_gen::LeaveGenStep::Dispatch(request) => {
            let task_id = insert_on_demand_task(conn, job.id, None).await?;
            leave_gen::LeaveGenHandler::insert_request(conn, task_id, &request).await?;
            Ok(Acquired::Task { task_id, request: TaskRequest::LeaveGeneration(request) })
        }
        leave_gen::LeaveGenStep::Transition { generation } => {
            Ok(Acquired::NeedsGenerationTransition { generation })
        }
        leave_gen::LeaveGenStep::Finished => Ok(Acquired::JobFinished),
        leave_gen::LeaveGenStep::NoWorkYet => Ok(Acquired::NoWork),
    }
}

/// On-demand tasks are inserted `available` and immediately claimed by the
/// caller in the same transaction, so the counter bookkeeping is identical on
/// every path.
async fn insert_on_demand_task(
    conn: &mut PgConnection,
    job_id: Uuid,
    seed: Option<i64>,
) -> AppResult<Uuid> {
    Ok(sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO tasks (job_id, seed, state) VALUES ($1, $2, 'available') RETURNING id",
    )
    .bind(job_id)
    .bind(seed)
    .fetch_one(conn)
    .await?)
}

/// Validate, normalize and store a worker submission.
pub async fn store_result(
    conn: &mut PgConnection,
    job: &Job,
    task_id: Uuid,
    claim_id: Uuid,
    payload: serde_json::Value,
) -> AppResult<()> {
    fn decode<T: serde::de::DeserializeOwned>(payload: serde_json::Value) -> AppResult<T> {
        serde_json::from_value(payload)
            .map_err(|e| AppError::bad_request(format!("malformed task response: {e}")))
    }

    match job.job_type {
        JobType::OpeningRackAnalysis => {
            let record = opening_rack::OpeningRackHandler::process_response(decode(payload)?)?;
            opening_rack::OpeningRackHandler::insert_record(conn, task_id, claim_id, &record).await
        }
        JobType::Games => {
            let record = game::GameHandler::process_response(decode(payload)?)?;
            game::GameHandler::insert_record(conn, task_id, claim_id, &record).await
        }
        JobType::GamePairs => {
            let record = game_pair::GamePairHandler::process_response(decode(payload)?)?;
            game_pair::GamePairHandler::insert_record(conn, task_id, claim_id, &record).await
        }
        JobType::LeaveGeneration => {
            let record = leave_gen::LeaveGenHandler::process_response(decode(payload)?)?;
            leave_gen::LeaveGenHandler::insert_record(conn, task_id, claim_id, &record).await
        }
    }
}

/// Called once at job creation for pre-populated job types. On-demand types do
/// nothing here — their tasks appear at claim time.
pub async fn prepopulate(conn: &mut PgConnection, job: &Job, data_path: &Path) -> AppResult<i64> {
    match job.job_type {
        JobType::OpeningRackAnalysis => {
            let config = sqlx::query_as::<_, OpeningRackConfig>(
                "SELECT * FROM job_opening_rack_config WHERE job_id = $1",
            )
            .bind(job.id)
            .fetch_one(&mut *conn)
            .await?;
            opening_rack::prepopulate(conn, job.id, &config, data_path).await
        }
        JobType::LeaveGeneration => {
            let config =
                sqlx::query_as::<_, LeaveConfig>("SELECT * FROM job_leave_config WHERE job_id = $1")
                    .bind(job.id)
                    .fetch_one(&mut *conn)
                    .await?;
            // Not tasks, but the rack universe generation 1 will be measured
            // against — without it there is nothing for claim-time selection to
            // order by.
            leave_gen::seed_generation(conn, job.id, 1, &config, data_path).await
        }
        JobType::Games | JobType::GamePairs => Ok(0),
    }
}
