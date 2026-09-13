//! Dispatch from `JobType` to the concrete handler. Every function here matches
//! exhaustively on `JobType`, so adding a variant fails to compile until all
//! four components of the new job type exist.

use super::handler::*;
use super::{game, game_pair, leave_gen, load_job_data, opening_rack};
use crate::artifacts::ArtifactStore;
use crate::auth::WorkerIdentity;
use crate::error::{AppError, AppResult};
use crate::models::job::*;
use sqlx::PgConnection;
use uuid::Uuid;

/// The outcome of trying to get one unit of work out of a job.
// One value per claim attempt, moved straight to the caller and never stored,
// so the size of the `Task` variant costs nothing worth a box.
#[allow(clippy::large_enum_variant)]
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

pub async fn acquire(
    conn: &mut PgConnection,
    job: &Job,
    identity: &WorkerIdentity,
) -> AppResult<Acquired> {
    // A task whose claim timed out drops back to `available`, and so does a
    // task with redundancy left to fill, so re-dispatching those comes first.
    // For games this is what keeps the seed space covered: an abandoned batch
    // is replayed rather than skipped, since nothing else would ever revisit
    // those seeds.
    //
    // Leave generation does this itself, under its claim lock and only for the
    // current generation (see `generate_leave_gen`).
    if !matches!(job.job_type, JobType::LeaveGeneration) {
        if let Some(task_id) = next_available(conn, job.id, identity, None).await? {
            let request = load_request(conn, job.job_type, task_id).await?;
            return Ok(Acquired::Task { task_id, request });
        }
    }

    // Every job type generates its tasks at claim time; there is no
    // pre-populated strategy any more.
    match job.job_type {
        JobType::OpeningRack => generate_opening_rack(conn, job).await,
        JobType::Games => generate_games(conn, job).await,
        JobType::GamePairs => generate_game_pairs(conn, job).await,
        JobType::LeaveGeneration => generate_leave_gen(conn, job, identity).await,
    }
}

async fn generate_opening_rack(conn: &mut PgConnection, job: &Job) -> AppResult<Acquired> {
    let config = sqlx::query_as::<_, OpeningRackConfig>(
        "SELECT * FROM job_opening_rack_config WHERE job_id = $1",
    )
    .bind(job.id)
    .fetch_one(&mut *conn)
    .await?;

    let job_data = load_job_data(&mut *conn, job.id).await?;
    let Some((start, request)) =
        opening_rack::next_request(conn, job.id, &config, &job_data).await?
    else {
        // The rack space is exhausted; nothing left to hand out.
        return Ok(Acquired::NoWork);
    };

    let task_id = insert_on_demand_task(conn, job.id, Some(start)).await?;
    opening_rack::insert_range(conn, task_id, &config, &job_data, start, request.racks.len())
        .await?;
    Ok(Acquired::Task { task_id, request: TaskRequest::OpeningRack(request) })
}

/// An available task this worker may take, locked with `FOR UPDATE SKIP LOCKED`
/// so concurrent claimers never serialize on one row.
///
/// Excludes tasks this identity already holds a slot on, live or completed.
/// With redundancy above 1 a task stays `available` after its first claim, and
/// the per-identity unique index refuses a second slot for the same worker --
/// correctly, since redundancy means *independent* workers. Without this
/// filter the oldest such task would be selected again on every attempt, the
/// insert would fail every time, and the worker would get nothing at all until
/// someone else filled the slot.
///
/// `leave_generation`, when set, restricts it to a leave job's tasks for that
/// generation.
async fn next_available(
    conn: &mut PgConnection,
    job_id: Uuid,
    identity: &WorkerIdentity,
    leave_generation: Option<i32>,
) -> AppResult<Option<Uuid>> {
    Ok(sqlx::query_scalar::<_, Uuid>(
        "SELECT t.id FROM tasks t
         WHERE t.job_id = $1 AND t.state = 'available'
           AND NOT EXISTS (
               SELECT 1 FROM task_claims c
               WHERE c.task_id = t.id
                 AND c.state NOT IN ('abandoned', 'declined')
                 AND (c.claimed_by_user_id = $2 OR c.claimed_by_anon_uuid = $3)
           )
           AND ($4::int IS NULL OR EXISTS (
               SELECT 1 FROM leave_requests r
               WHERE r.task_id = t.id AND r.generation = $4
           ))
         ORDER BY t.created_at
         FOR UPDATE OF t SKIP LOCKED
         LIMIT 1",
    )
    .bind(job_id)
    .bind(identity.user_id())
    .bind(identity.anon_uuid())
    .bind(leave_generation)
    .fetch_optional(conn)
    .await?)
}

pub async fn load_request(
    conn: &mut PgConnection,
    job_type: JobType,
    task_id: Uuid,
) -> AppResult<TaskRequest> {
    Ok(match job_type {
        JobType::OpeningRack => TaskRequest::OpeningRack(
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

    let job_data = load_job_data(&mut *conn, job.id).await?;
    let (seed, request) = game::next_request(conn, job.id, &config, &job_data).await?;
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

    let job_data = load_job_data(&mut *conn, job.id).await?;
    let (seed, request) = game_pair::next_request(conn, job.id, &config, &job_data).await?;
    let task_id = insert_on_demand_task(conn, job.id, Some(seed)).await?;
    super::insert_game_request(conn, task_id, &request).await?;
    Ok(Acquired::Task { task_id, request: TaskRequest::GamePairs(request) })
}

async fn generate_leave_gen(
    conn: &mut PgConnection,
    job: &Job,
    identity: &WorkerIdentity,
) -> AppResult<Acquired> {
    let config =
        sqlx::query_as::<_, LeaveConfig>("SELECT * FROM job_leave_config WHERE job_id = $1")
            .bind(job.id)
            .fetch_one(&mut *conn)
            .await?;

    // Held for the rest of this transaction, before anything is read: what to
    // hand out, and whether the generation can be closed, are decisions that
    // must not be made from a view of the job that another claim is in the
    // middle of changing. See PLAN.md's leave-generation claim steps.
    leave_gen::lock_claim_decisions(conn, job.id).await?;

    // A reopened task -- its claim timed out -- is reissued before a new one is
    // generated, as for every job type, but only here: after the lock, so the
    // reissued claim is visible to the in-flight check of every claim that
    // follows it, and only for the current generation. A task from a
    // generation that has since closed is left alone; its racks would be
    // played with a KLV that is no longer current, and its result discarded.
    let Some(generation) = leave_gen::current_generation(&mut *conn, job.id, &config).await? else {
        return Ok(Acquired::JobFinished);
    };
    if let Some(task_id) = next_available(&mut *conn, job.id, identity, Some(generation)).await? {
        let request = load_request(conn, job.job_type, task_id).await?;
        return Ok(Acquired::Task { task_id, request });
    }

    let job_data = load_job_data(&mut *conn, job.id).await?;
    match leave_gen::next_step(conn, job.id, &config, &job_data).await? {
        leave_gen::LeaveGenStep::Dispatch(request) => {
            let task_id = insert_on_demand_task(conn, job.id, None).await?;
            leave_gen::insert_request(conn, task_id, &request).await?;
            Ok(Acquired::Task { task_id, request: TaskRequest::LeaveGeneration(request) })
        }
        leave_gen::LeaveGenStep::Transition { generation } => {
            Ok(Acquired::NeedsGenerationTransition { generation })
        }
        leave_gen::LeaveGenStep::Finished => Ok(Acquired::JobFinished),
        // Another worker's request is aggregating the generation. This one has
        // nothing to do until that finishes, and the next job in the
        // scheduler's order may well have work.
        leave_gen::LeaveGenStep::TransitionInProgress { .. } => Ok(Acquired::NoWork),
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

/// Checks a batch against the size the task was dispatched with.
///
/// Lives here rather than in `process_response` because it needs the request,
/// which the pure validation step does not have. It is the one submission-time
/// check that can catch a worker reporting work it did not do: the batch size
/// was fixed when the task was handed out, so a result of any other size is
/// answering a question nobody asked.
async fn check_batch_size(
    conn: &mut PgConnection,
    job: &Job,
    task_id: Uuid,
    reported_games: i32,
) -> AppResult<()> {
    super::plausibility::check_against_task(
        conn,
        job,
        task_id,
        &super::plausibility::Reported { games: Some(reported_games) },
    )
    .await
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
        JobType::OpeningRack => {
            let record = opening_rack::OpeningRackHandler::process_response(decode(payload)?)?;
            opening_rack::OpeningRackHandler::insert_record(conn, task_id, claim_id, &record)
                .await?;
            // One row per rack, and the unique index on (task_claim_id, rack)
            // means the insert above would have failed on a duplicate, so the
            // submission's length is its distinct-rack count. Racks never
            // repeat across tasks either: each task analyses its own slice of
            // the enumerated space.
            count_first_result(
                conn,
                job,
                task_id,
                "racks_analyzed",
                record.positions.len() as i64,
            )
            .await
        }
        JobType::Games => {
            let record = game::GameHandler::process_response(decode(payload)?)?;
            check_batch_size(conn, job, task_id, record.all_games.games).await?;
            game::GameHandler::insert_record(conn, task_id, claim_id, &record).await?;
            count_first_result(conn, job, task_id, "games_completed", record.all_games.games as i64)
                .await
        }
        JobType::GamePairs => {
            let record = game_pair::GamePairHandler::process_response(decode(payload)?)?;
            check_batch_size(conn, job, task_id, record.all_games.games).await?;
            game_pair::GamePairHandler::insert_record(conn, task_id, claim_id, &record).await?;
            // Games, not pairs, for both job types: the pairs count is half of
            // it and is derived where it is displayed.
            count_first_result(conn, job, task_id, "games_completed", record.all_games.games as i64)
                .await
        }
        JobType::LeaveGeneration => {
            let record = leave_gen::LeaveGenHandler::process_response(decode(payload)?)?;
            leave_gen::LeaveGenHandler::insert_record(conn, task_id, claim_id, &record).await
        }
    }
}

/// Add this submission's work to one of the job's running progress totals, but
/// only if it is the first accepted result for its task.
///
/// The reads these totals replace both selected one result per task -- the
/// aggregates they summed describe the same deterministic work on every
/// redundant claim, so counting all of them would multiply the total by the
/// job's redundancy (PLAN.md, "What these reads cost"). "First" is decided from
/// the rows just written rather than from the task's `accepted_count`, which
/// the submit path has not incremented yet. That is sound only because
/// `submit_result` locks the task row before storing anything, so this count
/// sees every earlier submission for the task as committed. Without the lock,
/// two submissions arriving together each counted only their own uncommitted
/// rows and both added to the total. Count and update share the submission's
/// transaction, so a submission that later fails contributes neither.
///
/// The update takes a row lock on `jobs`, so two submissions for the same job
/// serialize here for as long as the lock is held. A task is minutes of work, so
/// that is a lock every few seconds at most on a busy job, and the alternative
/// -- the count these totals exist to avoid -- was seconds of CPU per page view.
async fn count_first_result(
    conn: &mut PgConnection,
    job: &Job,
    task_id: Uuid,
    column: &str,
    amount: i64,
) -> AppResult<()> {
    let results_for_task = match job.job_type {
        JobType::OpeningRack => {
            "SELECT COUNT(DISTINCT task_claim_id) FROM position_analysis_records WHERE task_id = $1"
        }
        JobType::Games | JobType::GamePairs => {
            "SELECT COUNT(*) FROM game_results WHERE task_id = $1"
        }
        // Leave generation has no such total: its progress is
        // `leave_rack_progress`, which is already one indexed row per rack.
        JobType::LeaveGeneration => return Ok(()),
    };
    let results: i64 = sqlx::query_scalar(results_for_task)
        .bind(task_id)
        .fetch_one(&mut *conn)
        .await?;
    if results != 1 {
        return Ok(());
    }

    // `column` is one of two literals chosen in this file, never worker input.
    sqlx::query(&format!("UPDATE jobs SET {column} = {column} + $2 WHERE id = $1"))
        .bind(job.id)
        .bind(amount)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// State a job needs in place before it can dispatch anything.
///
/// No job type pre-populates *tasks* any more -- every one generates them at
/// claim time. This is only leave generation's rack universe, which claim-time
/// rack selection orders by.
pub async fn initialize_job_state(conn: &mut PgConnection, job: &Job) -> AppResult<i64> {
    match job.job_type {
        JobType::LeaveGeneration => {
            let job_data = load_job_data(&mut *conn, job.id).await?;
            leave_gen::seed_generation(conn, job.id, 1, &job_data.letterdist).await
        }
        JobType::OpeningRack | JobType::Games | JobType::GamePairs => Ok(0),
    }
}

/// The part of job initialization that cannot run inside the creating
/// transaction: generation 1's zeroed KLV is a multi-megabyte build and an
/// object-store write. Called after the transaction commits, and again at
/// activation if it has not happened yet. Idempotent.
pub async fn initialize_job_artifacts(
    pool: &sqlx::PgPool,
    artifacts: &ArtifactStore,
    job: &Job,
) -> AppResult<()> {
    if job.job_type != JobType::LeaveGeneration {
        return Ok(());
    }
    let mut conn = pool.acquire().await?;
    let job_data = load_job_data(&mut conn, job.id).await?;
    drop(conn);
    leave_gen::seed_zero_generation(pool, artifacts, job.id, &job_data.letterdist).await?;
    Ok(())
}

/// Whether a job has everything `initialize_job_artifacts` writes. Only leave
/// generation writes anything.
pub async fn job_artifacts_ready(pool: &sqlx::PgPool, job: &Job) -> AppResult<bool> {
    if job.job_type != JobType::LeaveGeneration {
        return Ok(true);
    }
    Ok(sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM leave_generation_artifacts
                        WHERE job_id = $1 AND generation = 0)",
    )
    .bind(job.id)
    .fetch_one(pool)
    .await?)
}
