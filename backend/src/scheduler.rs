//! Job selection and task claiming — the sequence that runs every time a worker
//! asks for work.

use crate::auth::WorkerIdentity;
use crate::error::{AppError, AppResult};
use crate::jobs::handler::TaskRequest;
use crate::jobs::leave_gen;
use crate::jobs::registry::{self, Acquired};
use crate::jobs::{expected_data, ExpectedFile};
use crate::models::job::{Job, JobType, LeaveConfig};
use crate::state::AppState;
use crate::version::Version;
use serde::Serialize;
use sqlx::{PgPool, Row};
use uuid::Uuid;

/// What a worker says about itself when it asks for work. Both fields are
/// load-bearing, which is why the claim body is required rather than optional:
/// without the version the server would have to assume one, and an assumed
/// version is a wrong answer dressed as a safe one.
pub struct WorkerCapabilities {
    pub magpie_version: Version,
    /// Jobs this worker has already found it cannot run, for any reason. The
    /// client keeps this set in memory and resends it; the server does not
    /// route on the gaps it has recorded, so a contributor who fixes their
    /// data is unblocked by their own next claim.
    pub unsupported_jobs: Vec<Uuid>,
}

pub struct TaskClaim {
    pub job_id: Uuid,
    pub claim_token: Uuid,
    pub request: TaskRequest,
    pub min_magpie_version: String,
    pub expected_data: Vec<ExpectedFile>,
}

/// The four answers a claim can get. One decision, not four checks: `204` and
/// `shutdown` mean opposite things -- "sleep and retry" versus "you will never
/// be useful until something on your end changes" -- and conflating them either
/// spins a doomed client forever or tells a contributor their data is stale
/// because the server happened to be idle. Computing all four in one place lets
/// the compiler insist every branch is considered.
pub enum ClaimOutcome {
    Task(Box<TaskClaim>),
    /// Work this worker could run exists, but none is available right now.
    Idle,
    /// No active jobs at all. A quiet server is not the worker's fault.
    NoWorkExists,
    /// Every active job is ruled out for this worker.
    Shutdown(ShutdownDirective),
}

#[derive(Debug, Clone, Serialize)]
pub struct ShutdownDirective {
    /// `data_out_of_date`, `magpie_too_old`, or `both`.
    pub reason: String,
    pub message: String,
    pub required_tarball_dates: Vec<String>,
    pub required_magpie_version: Option<String>,
    pub download_url: Option<String>,
}

/// Active jobs in the top priority tier, ordered by how far behind their
/// allocation share they are.
///
/// `tasks_dispatched` counts every claim ever issued, abandoned ones included:
/// a claim consumed dispatch capacity the moment it was inserted, so the count
/// only ever goes up. Filtering out abandoned claims would let a job with flaky
/// workers quietly accumulate more than its share and would make the deficit
/// non-monotonic, which is the opposite of what this scheduler needs.
/// Both capability filters live in the `eligible_jobs` CTE and the priority is
/// computed over its output, so "filter before MIN(priority)" is structural
/// rather than remembered. Applying either filter afterwards would pick the top
/// tier from jobs the worker cannot run and then hand back nothing, leaving a
/// worker locked out of tier 0 blind to doable work in tier 1.
async fn candidate_jobs(pool: &PgPool, caps: &WorkerCapabilities) -> AppResult<Vec<Job>> {
    Ok(sqlx::query_as::<_, Job>(
        "WITH eligible_jobs AS (
             SELECT j.*
             FROM jobs j
             WHERE j.status = 'active'
               AND (j.min_magpie_major, j.min_magpie_minor, j.min_magpie_patch)
                   <= ($1, $2, $3)
               AND j.id <> ALL($4)
         )
         SELECT e.*
         FROM eligible_jobs e
         WHERE e.priority = (SELECT MIN(priority) FROM eligible_jobs)
         ORDER BY
           (SELECT COUNT(*) FROM task_claims tc
            JOIN tasks t ON t.id = tc.task_id
            WHERE t.job_id = e.id)::float
           / NULLIF(e.allocation, 0) ASC,
           e.created_at ASC",
    )
    .bind(caps.magpie_version.major)
    .bind(caps.magpie_version.minor)
    .bind(caps.magpie_version.patch)
    .bind(&caps.unsupported_jobs)
    .fetch_all(pool)
    .await?)
}

/// Why a worker that can run nothing can run nothing, and what to tell it.
///
/// Only reached once `candidate_jobs` came back empty, so the question is
/// whether any active job exists at all and, if so, which axis ruled them out.
/// "Both" leads with the MAGPIE version: a release bumps `DATA_VERSION` and the
/// contributor runs `download_data.sh` as part of updating, so telling them to
/// fix their data first sends them on a trip they would have made anyway.
async fn shutdown_or_idle(
    state: &AppState,
    caps: &WorkerCapabilities,
) -> AppResult<ClaimOutcome> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS total,
                COUNT(*) FILTER (
                    WHERE (min_magpie_major, min_magpie_minor, min_magpie_patch) > ($1, $2, $3)
                ) AS too_new,
                MIN(format('%s.%s.%s', min_magpie_major, min_magpie_minor, min_magpie_patch))
                    FILTER (
                        WHERE (min_magpie_major, min_magpie_minor, min_magpie_patch) > ($1, $2, $3)
                    ) AS lowest_floor
         FROM jobs WHERE status = 'active'",
    )
    .bind(caps.magpie_version.major)
    .bind(caps.magpie_version.minor)
    .bind(caps.magpie_version.patch)
    .fetch_one(&state.pool)
    .await?;

    let total: i64 = row.get("total");
    if total == 0 {
        return Ok(ClaimOutcome::NoWorkExists);
    }
    let too_new: i64 = row.get("too_new");
    let version_blocked = too_new > 0;
    let data_blocked = !caps.unsupported_jobs.is_empty();
    if !version_blocked && !data_blocked {
        // Active jobs exist and nothing rules them out; they simply had no
        // task to hand out this instant.
        return Ok(ClaimOutcome::Idle);
    }

    let required_magpie_version: Option<String> = row.get("lowest_floor");
    let tarball_dates = if data_blocked {
        sqlx::query_scalar::<_, String>(
            "SELECT DISTINCT d.tarball_date
             FROM jobs j
             JOIN input_data d ON d.id IN (j.letterdist_id, j.layout_id)
             WHERE j.id = ANY($1)
             ORDER BY d.tarball_date DESC",
        )
        .bind(&caps.unsupported_jobs)
        .fetch_all(&state.pool)
        .await?
    } else {
        Vec::new()
    };

    let (reason, message) = match (version_blocked, data_blocked) {
        (true, true) => (
            "both",
            format!(
                "Every active job requires MAGPIE {} or newer, or input data you \
                 do not have; you are running {}.",
                required_magpie_version.clone().unwrap_or_default(),
                caps.magpie_version
            ),
        ),
        (true, false) => (
            "magpie_too_old",
            format!(
                "Every active job requires MAGPIE {} or newer; you are running {}.",
                required_magpie_version.clone().unwrap_or_default(),
                caps.magpie_version
            ),
        ),
        (false, true) => (
            "data_out_of_date",
            "Every active job needs input data you do not have.".to_string(),
        ),
        (false, false) => unreachable!("handled above"),
    };

    Ok(ClaimOutcome::Shutdown(ShutdownDirective {
        reason: reason.to_string(),
        message,
        required_tarball_dates: tarball_dates,
        download_url: version_blocked.then(|| state.cfg.magpie_download_url.clone()),
        required_magpie_version: version_blocked.then_some(required_magpie_version).flatten(),
    }))
}

/// Lazy timeout reclamation, run at claim time rather than by a background
/// process. Each timed-out claim flips to `abandoned`, the task's
/// `active_claim_count` drops, and a task that was at capacity reopens.
pub async fn reclaim_expired(pool: &PgPool, job_id: Uuid, timeout_secs: f64) -> AppResult<u64> {
    let result = sqlx::query(
        "WITH expired AS (
             UPDATE task_claims c
             SET state = 'abandoned'
             FROM tasks t
             WHERE c.task_id = t.id
               AND t.job_id = $1
               AND c.state = 'claimed'
               AND COALESCE(c.last_heartbeat_at, c.claimed_at) < now() - make_interval(secs => $2)
             RETURNING c.task_id
         ),
         counts AS (
             SELECT task_id, COUNT(*)::int AS n FROM expired GROUP BY task_id
         )
         UPDATE tasks t
         SET active_claim_count = GREATEST(t.active_claim_count - counts.n, 0),
             state = CASE
                 WHEN t.accepted_count >= $3 THEN 'completed'::task_state
                 WHEN t.accepted_count + GREATEST(t.active_claim_count - counts.n, 0) >= $3
                     THEN 'claimed'::task_state
                 ELSE 'available'::task_state
             END
         FROM counts
         WHERE t.id = counts.task_id",
    )
    .bind(job_id)
    .bind(timeout_secs)
    .bind(job_redundancy(pool, job_id).await?)
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}

async fn job_redundancy(pool: &PgPool, job_id: Uuid) -> AppResult<i32> {
    Ok(sqlx::query_scalar::<_, i32>("SELECT redundancy FROM jobs WHERE id = $1")
        .bind(job_id)
        .fetch_one(pool)
        .await?)
}

/// Walk the priority tier in deficit order and hand out the first available unit
/// of work. Returns `None` when no active job has anything to dispatch.
pub async fn claim(
    state: &AppState,
    identity: &WorkerIdentity,
    caps: &WorkerCapabilities,
) -> AppResult<ClaimOutcome> {
    let timeout_secs = state.cfg.heartbeat_timeout.as_secs_f64();

    // The global floor short-circuits everything: a client below it cannot run
    // any job that could ever exist, so no job needs consulting.
    let floor = Version::parse_or_zero(&state.cfg.min_magpie_version);
    if caps.magpie_version < floor {
        return Ok(ClaimOutcome::Shutdown(ShutdownDirective {
            reason: "magpie_too_old".to_string(),
            message: format!(
                "This server requires MAGPIE {floor} or newer; you are running {}.",
                caps.magpie_version
            ),
            required_tarball_dates: Vec::new(),
            required_magpie_version: Some(floor.to_string()),
            download_url: Some(state.cfg.magpie_download_url.clone()),
        }));
    }

    // The outer retry exists for two cases: a leave-gen generation transition
    // (which creates new work mid-request) and a lost race on the `(job_id,
    // seed)` unique index when two workers generate the same on-demand task.
    for _attempt in 0..3 {
        let jobs = candidate_jobs(&state.pool, caps).await?;
        if jobs.is_empty() {
            return shutdown_or_idle(state, caps).await;
        }

        let mut retry_outer = false;
        for job in &jobs {
            reclaim_expired(&state.pool, job.id, timeout_secs).await?;

            match try_claim_from_job(state, identity, job, caps).await {
                Ok(Some(outcome)) => return Ok(ClaimOutcome::Task(Box::new(outcome))),
                Ok(None) => continue,
                Err(JobClaimError::Retry) => {
                    retry_outer = true;
                    break;
                }
                Err(JobClaimError::Fatal(err)) => return Err(err),
            }
        }

        if !retry_outer {
            // Jobs this worker can run exist; none had a task to hand out.
            return Ok(ClaimOutcome::Idle);
        }
    }

    Ok(ClaimOutcome::Idle)
}

enum JobClaimError {
    /// Something changed underneath us; re-run job selection.
    Retry,
    Fatal(AppError),
}

async fn try_claim_from_job(
    state: &AppState,
    identity: &WorkerIdentity,
    job: &Job,
    caps: &WorkerCapabilities,
) -> Result<Option<TaskClaim>, JobClaimError> {
    let mut tx = state.pool.begin().await.map_err(|e| JobClaimError::Fatal(e.into()))?;

    let acquired = match registry::acquire(&mut tx, job).await {
        Ok(acquired) => acquired,
        Err(err) => {
            let _ = tx.rollback().await;
            return Err(if is_unique_violation(&err) {
                JobClaimError::Retry
            } else {
                JobClaimError::Fatal(err)
            });
        }
    };

    match acquired {
        Acquired::NoWork => {
            let _ = tx.rollback().await;
            Ok(None)
        }
        Acquired::JobFinished => {
            let _ = tx.rollback().await;
            sqlx::query("UPDATE jobs SET status = 'completed' WHERE id = $1")
                .bind(job.id)
                .execute(&state.pool)
                .await
                .map_err(|e| JobClaimError::Fatal(e.into()))?;
            Ok(None)
        }
        Acquired::NeedsGenerationTransition { generation } => {
            // Uploads to S3 and shells out to MAGPIE, so it must not hold the
            // claim transaction open.
            let _ = tx.rollback().await;
            run_leave_generation_transition(state, job, generation)
                .await
                .map_err(JobClaimError::Fatal)?;
            Err(JobClaimError::Retry)
        }
        Acquired::Task { task_id, request } => {
            let claim_token = Uuid::new_v4();
            let insert = sqlx::query(
                "INSERT INTO task_claims
                     (task_id, claim_token, claimed_by_user_id, claimed_by_anon_uuid,
                      magpie_version)
                 VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(task_id)
            .bind(claim_token)
            .bind(identity.user_id())
            .bind(identity.anon_uuid())
            .bind(caps.magpie_version.to_string())
            .execute(&mut *tx)
            .await;

            if let Err(err) = insert {
                let _ = tx.rollback().await;
                let err: AppError = err.into();
                // The per-identity partial unique index rejects a second slot on
                // the same task. That is not a failure — this worker already
                // holds a slot here, so re-run selection and land somewhere else.
                return if is_unique_violation(&err) {
                    Err(JobClaimError::Retry)
                } else {
                    Err(JobClaimError::Fatal(err))
                };
            }

            sqlx::query(
                "UPDATE tasks t
                 SET active_claim_count = t.active_claim_count + 1,
                     state = CASE
                         WHEN t.accepted_count + t.active_claim_count + 1 >= j.redundancy
                             THEN 'claimed'::task_state
                         ELSE 'available'::task_state
                     END
                 FROM jobs j
                 WHERE t.id = $1 AND j.id = t.job_id",
            )
            .bind(task_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| JobClaimError::Fatal(e.into()))?;

            crate::audit::log(
                &mut tx,
                "task.claimed",
                identity.user_id(),
                identity.anon_uuid(),
                Some("task"),
                Some(task_id.to_string()),
                Some(job.id),
            )
            .await
            .map_err(JobClaimError::Fatal)?;

            let expected = expected_data(&mut tx, job)
                .await
                .map_err(JobClaimError::Fatal)?;

            tx.commit().await.map_err(|e| JobClaimError::Fatal(e.into()))?;

            Ok(Some(TaskClaim {
                job_id: job.id,
                claim_token,
                request,
                min_magpie_version: job.min_magpie_version().to_string(),
                expected_data: expected,
            }))
        }
    }
}

/// Release a claim that ended in something other than a submission.
///
/// Decline and expiry are the same operation -- mark the claim terminal, drop
/// the job's live claim count, recompute the task against `redundancy` -- and
/// writing it twice is how the counter drifts. A drifting counter makes the
/// scheduler believe a job is saturated and dispatch quietly stops, days later
/// and nowhere near the cause.
pub async fn release_claim(
    tx: &mut sqlx::PgTransaction<'_>,
    claim_id: Uuid,
    terminal_state: &str,
) -> AppResult<()> {
    sqlx::query("UPDATE task_claims SET state = $2::claim_state WHERE id = $1")
        .bind(claim_id)
        .bind(terminal_state)
        .execute(&mut **tx)
        .await?;

    sqlx::query(
        "UPDATE tasks t
         SET active_claim_count = GREATEST(t.active_claim_count - 1, 0),
             state = CASE
                 WHEN t.accepted_count >= j.redundancy THEN 'completed'::task_state
                 WHEN t.accepted_count + GREATEST(t.active_claim_count - 1, 0) >= j.redundancy
                     THEN 'claimed'::task_state
                 ELSE 'available'::task_state
             END
         FROM jobs j, task_claims c
         WHERE c.id = $1 AND t.id = c.task_id AND j.id = t.job_id",
    )
    .bind(claim_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

async fn run_leave_generation_transition(
    state: &AppState,
    job: &Job,
    generation: i32,
) -> AppResult<()> {
    if job.job_type != JobType::LeaveGeneration {
        return Ok(());
    }
    let config = sqlx::query_as::<_, LeaveConfig>("SELECT * FROM job_leave_config WHERE job_id = $1")
        .bind(job.id)
        .fetch_one(&state.pool)
        .await?;

    let mut conn = state.pool.acquire().await?;
    let job_data = crate::jobs::load_job_data(&mut conn, job.id).await?;
    drop(conn);

    let key = leave_gen::run_transition(
        &state.pool,
        &state.artifacts,
        job.id,
        generation,
        &config,
        &job_data.letterdist,
    )
    .await?;
    tracing::info!(job_id = %job.id, generation, artifact_key = %key, "leave generation complete");
    Ok(())
}

fn is_unique_violation(err: &AppError) -> bool {
    err.message.contains("duplicate key value violates unique constraint")
}
