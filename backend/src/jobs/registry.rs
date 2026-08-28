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

pub async fn acquire(conn: &mut PgConnection, job: &Job,
                     data_path: &Path) -> AppResult<Acquired> {
    // A task whose claim timed out drops back to `available` regardless of the
    // job's creation strategy, so re-dispatching those comes first. For games
    // this is what keeps the seed space covered: an abandoned batch is replayed
    // rather than skipped, since nothing else would ever revisit those seeds.
    if let Some(task_id) = next_available(conn, job.id).await? {
        let request = load_request(conn, job.job_type, task_id, data_path).await?;
        return Ok(Acquired::Task { task_id, request });
    }

    // Every job type generates its tasks at claim time; there is no
    // pre-populated strategy any more.
    match job.job_type {
        JobType::OpeningRackAnalysis => generate_opening_rack(conn, job, data_path).await,
        JobType::Games => generate_games(conn, job).await,
        JobType::GamePairs => generate_game_pairs(conn, job).await,
        JobType::LeaveGeneration => generate_leave_gen(conn, job).await,
    }
}

async fn generate_opening_rack(conn: &mut PgConnection, job: &Job,
                               data_path: &Path) -> AppResult<Acquired> {
    let config = sqlx::query_as::<_, OpeningRackConfig>(
        "SELECT * FROM job_opening_rack_config WHERE job_id = $1",
    )
    .bind(job.id)
    .fetch_one(&mut *conn)
    .await?;

    let Some((start, request)) =
        opening_rack::next_request(conn, job.id, &config, data_path).await?
    else {
        // The rack space is exhausted; nothing left to hand out.
        return Ok(Acquired::NoWork);
    };

    let task_id = insert_on_demand_task(conn, job.id, Some(start)).await?;
    opening_rack::insert_range(conn, task_id, &config, start, request.racks.len()).await?;
    Ok(Acquired::Task { task_id, request: TaskRequest::OpeningRackAnalysis(request) })
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
    data_path: &Path,
) -> AppResult<TaskRequest> {
    Ok(match job_type {
        JobType::OpeningRackAnalysis => TaskRequest::OpeningRackAnalysis(
            opening_rack::OpeningRackHandler::load_request(conn, task_id, data_path).await?,
        ),
        JobType::Games => {
            TaskRequest::Games(game::GameHandler::load_request(conn, task_id, data_path).await?)
        }
        JobType::GamePairs => {
            TaskRequest::GamePairs(
            game_pair::GamePairHandler::load_request(conn, task_id, data_path).await?,
        )
        }
        JobType::LeaveGeneration => TaskRequest::LeaveGeneration(
            leave_gen::LeaveGenHandler::load_request(conn, task_id, data_path).await?,
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
    super::insert_game_request(conn, task_id, &request).await?;
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
    super::insert_game_request(conn, task_id, &request).await?;
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
            leave_gen::insert_request(conn, task_id, &request).await?;
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

/// State a job needs in place before it can dispatch anything.
///
/// No job type pre-populates *tasks* any more -- every one generates them at
/// claim time. This is only leave generation's rack universe, which claim-time
/// rack selection orders by.
pub async fn initialize_job_state(conn: &mut PgConnection, job: &Job,
                                  data_path: &Path) -> AppResult<i64> {
    match job.job_type {
        JobType::LeaveGeneration => {
            let config =
                sqlx::query_as::<_, LeaveConfig>("SELECT * FROM job_leave_config WHERE job_id = $1")
                    .bind(job.id)
                    .fetch_one(&mut *conn)
                    .await?;
            leave_gen::seed_generation(conn, job.id, 1, &config, data_path).await
        }
        JobType::OpeningRackAnalysis | JobType::Games | JobType::GamePairs => Ok(0),
    }
}
