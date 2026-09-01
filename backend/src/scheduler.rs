//! Job selection and task claiming — the sequence that runs every time a worker
//! asks for work.

use crate::auth::WorkerIdentity;
use crate::error::{AppError, AppResult};
use crate::jobs::handler::TaskRequest;
use crate::jobs::leave_gen;
use crate::jobs::registry::{self, Acquired};
use crate::models::job::{Job, JobType, LeaveConfig};
use crate::state::AppState;
use sqlx::PgPool;
use uuid::Uuid;

pub struct ClaimOutcome {
    pub job_id: Uuid,
    pub claim_token: Uuid,
    pub request: TaskRequest,
    pub min_magpie_version: Option<String>,
}

/// Active jobs in the top priority tier, ordered by how far behind their
/// allocation share they are.
///
/// `tasks_dispatched` counts every claim ever issued, abandoned ones included:
/// a claim consumed dispatch capacity the moment it was inserted, so the count
/// only ever goes up. Filtering out abandoned claims would let a job with flaky
/// workers quietly accumulate more than its share and would make the deficit
/// non-monotonic, which is the opposite of what this scheduler needs.
async fn candidate_jobs(pool: &PgPool) -> AppResult<Vec<Job>> {
    let min_priority =
        sqlx::query_scalar::<_, Option<i32>>("SELECT MIN(priority) FROM jobs WHERE status = 'active'")
            .fetch_one(pool)
            .await?;
    let Some(min_priority) = min_priority else {
        return Ok(Vec::new());
    };

    Ok(sqlx::query_as::<_, Job>(
        "SELECT j.*
         FROM jobs j
         WHERE j.status = 'active' AND j.priority = $1
         ORDER BY
           (SELECT COUNT(*) FROM task_claims tc
            JOIN tasks t ON t.id = tc.task_id
            WHERE t.job_id = j.id)::float
           / NULLIF(j.allocation, 0) ASC,
           j.created_at ASC",
    )
    .bind(min_priority)
    .fetch_all(pool)
    .await?)
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
pub async fn claim(state: &AppState, identity: &WorkerIdentity) -> AppResult<Option<ClaimOutcome>> {
    let timeout_secs = state.cfg.heartbeat_timeout.as_secs_f64();

    // The outer retry exists for two cases: a leave-gen generation transition
    // (which creates new work mid-request) and a lost race on the `(job_id,
    // seed)` unique index when two workers generate the same on-demand task.
    for _attempt in 0..3 {
        let jobs = candidate_jobs(&state.pool).await?;
        if jobs.is_empty() {
            return Ok(None);
        }

        let mut retry_outer = false;
        for job in &jobs {
            reclaim_expired(&state.pool, job.id, timeout_secs).await?;

            match try_claim_from_job(state, identity, job).await {
                Ok(Some(outcome)) => return Ok(Some(outcome)),
                Ok(None) => continue,
                Err(JobClaimError::Retry) => {
                    retry_outer = true;
                    break;
                }
                Err(JobClaimError::Fatal(err)) => return Err(err),
            }
        }

        if !retry_outer {
            return Ok(None);
        }
    }

    Ok(None)
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
) -> Result<Option<ClaimOutcome>, JobClaimError> {
    let mut tx = state.pool.begin().await.map_err(|e| JobClaimError::Fatal(e.into()))?;

    let acquired = match registry::acquire(&mut tx, job, &state.cfg.data_path).await {
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
                     (task_id, claim_token, claimed_by_user_id, claimed_by_anon_uuid)
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(task_id)
            .bind(claim_token)
            .bind(identity.user_id())
            .bind(identity.anon_uuid())
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

            tx.commit().await.map_err(|e| JobClaimError::Fatal(e.into()))?;

            Ok(Some(ClaimOutcome {
                job_id: job.id,
                claim_token,
                request,
                min_magpie_version: job.min_magpie_version.clone(),
            }))
        }
    }
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

    let key = leave_gen::run_transition(
        &state.pool,
        &state.artifacts,
        &state.cfg.data_path,
        job.id,
        generation,
        &config,
    )
    .await?;
    tracing::info!(job_id = %job.id, generation, artifact_key = %key, "leave generation complete");
    Ok(())
}

fn is_unique_violation(err: &AppError) -> bool {
    err.message.contains("duplicate key value violates unique constraint")
}
