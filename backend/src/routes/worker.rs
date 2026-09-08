use crate::audit;
use crate::auth::WorkerIdentity;
use crate::error::{AppError, AppResult};
use crate::jobs::handler::TaskRequest;
use crate::jobstats;
use crate::models::job::{GamePairConfig, Job, JobType};
use crate::ratelimit;
use crate::ratings;
use crate::scheduler;
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/client-version", get(client_version))
        .route("/task", post(claim_task))
        .route("/decline", post(decline_task))
        .route("/heartbeat", post(heartbeat))
        .route("/result", post(submit_result))
        .route("/artifact", get(artifact))
}

#[derive(Serialize)]
struct ClientVersion {
    /// The oldest MAGPIE a client may contribute with. Individual jobs can
    /// require newer still, via `min_magpie_version`.
    min_magpie_version: String,
    /// Where to get MAGPIE. The client cannot update itself -- it is a compiled
    /// binary, and an auto-updating executable is a far larger security
    /// proposition than a script re-execing itself -- so this is for humans.
    download_url: String,
}

/// Version negotiation, replacing the self-update the Python client used.
async fn client_version(State(state): State<AppState>) -> Json<ClientVersion> {
    Json(ClientVersion {
        min_magpie_version: state.cfg.min_magpie_version.clone(),
        download_url: state.cfg.magpie_download_url.clone(),
    })
}

#[derive(Deserialize)]
struct ArtifactQuery {
    key: String,
}

/// Workers fetch previous-generation leave files through the server rather than
/// straight from S3, so a contributor never needs AWS credentials.
async fn artifact(
    State(state): State<AppState>,
    identity: WorkerIdentity,
    Query(query): Query<ArtifactQuery>,
) -> AppResult<Response> {
    ratelimit::check(&state.limits.worker, &identity.rate_key())?;

    // Only keys the server itself minted are reachable; an arbitrary key would
    // turn this into a read primitive for the whole bucket.
    let known = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM leave_generation_artifacts WHERE artifact_key = $1)",
    )
    .bind(&query.key)
    .fetch_one(&state.pool)
    .await?;
    if !known {
        return Err(AppError::not_found("no such artifact"));
    }

    let body = state.artifacts.get(&query.key).await?;
    Ok((
        [(axum::http::header::CONTENT_TYPE, "application/octet-stream")],
        body,
    )
        .into_response())
}

/// What the worker says about itself. Required, not optional: the version
/// drives the per-job floor filter, and a server that has to assume a version
/// is giving a wrong answer dressed as a safe one.
#[derive(Deserialize)]
struct ClaimBody {
    magpie_version: String,
    /// Jobs this worker has already found it cannot run, for any reason.
    /// Attacker-controlled, so it is capped and bound as an array rather than
    /// interpolated.
    #[serde(default)]
    unsupported_jobs: Vec<Uuid>,
}

/// Far above any honest client: the list is bounded by the jobs a worker has
/// actually been offered. Past the cap it is truncated rather than rejected --
/// a truncated list costs at most a wasted claim.
const MAX_UNSUPPORTED_JOBS: usize = 200;

#[derive(Serialize)]
struct TaskAssignment {
    claim_token: Uuid,
    job_id: Uuid,
    task_request: TaskRequest,
    min_magpie_version: String,
    /// Every file this task will load, with the digest the worker must be able
    /// to reproduce from its own copy.
    expected_data: ExpectedData,
    /// Present only when the request carried no identity at all and the
    /// server just minted one. The client persists this and sends it as
    /// `X-Worker-UUID` on every later request.
    #[serde(skip_serializing_if = "Option::is_none")]
    worker_uuid: Option<Uuid>,
}

#[derive(Serialize)]
struct ExpectedData {
    /// Named so a client that does not recognise the algorithm can say so and
    /// run unverified rather than refusing work: an algorithm change should not
    /// be a fleet-wide outage, and `min_magpie_version` is the lever for that.
    algorithm: &'static str,
    files: Vec<crate::jobs::ExpectedFile>,
}

#[derive(Serialize)]
struct ShutdownResponse {
    shutdown: scheduler::ShutdownDirective,
}

/// The task claim: "I am ready for work, here is what I am and what I cannot
/// do". Everything about the assignment itself is the server's decision.
async fn claim_task(
    State(state): State<AppState>,
    identity: WorkerIdentity,
    Json(body): Json<ClaimBody>,
) -> AppResult<Response> {
    ratelimit::check(&state.limits.worker, &identity.rate_key())?;

    let mut unsupported_jobs = body.unsupported_jobs;
    unsupported_jobs.truncate(MAX_UNSUPPORTED_JOBS);
    let caps = scheduler::WorkerCapabilities {
        magpie_version: crate::version::Version::parse_or_zero(&body.magpie_version),
        unsupported_jobs,
    };

    match scheduler::claim(&state, &identity, &caps).await? {
        // 204 rather than an error: "no work right now" is the normal state of
        // a quiet server, and the client sleeps and asks again. This is not the
        // same as a shutdown, which says "you will never be useful until
        // something on your end changes" -- conflating them either spins a
        // doomed client forever or tells a contributor their data is stale
        // because the server happened to be idle.
        //
        // A brand new anonymous identity minted for this request is not
        // reported here: there is no body to carry it in, and a client that
        // finds no work has nothing to persist yet.
        scheduler::ClaimOutcome::Idle | scheduler::ClaimOutcome::NoWorkExists => {
            Ok(StatusCode::NO_CONTENT.into_response())
        }
        scheduler::ClaimOutcome::Shutdown(directive) => {
            Ok(Json(ShutdownResponse { shutdown: directive }).into_response())
        }
        scheduler::ClaimOutcome::Task(claim) => Ok(Json(TaskAssignment {
            claim_token: claim.claim_token,
            job_id: claim.job_id,
            task_request: claim.request,
            min_magpie_version: claim.min_magpie_version,
            expected_data: ExpectedData { algorithm: "sha256", files: claim.expected_data },
            worker_uuid: identity.newly_assigned_uuid(),
        })
        .into_response()),
    }
}

/// A file the worker could not produce the expected digest for.
#[derive(Deserialize)]
struct MissingFile {
    role: String,
    name: String,
    expected: String,
    /// `None` means the file was not found at all; a hex string means it was
    /// found with different content.
    #[serde(default)]
    actual: Option<String>,
}

#[derive(Deserialize)]
struct DeclineBody {
    claim_token: Uuid,
    /// `missing_data`, `magpie_version` or `unknown_job_type`.
    reason: String,
    #[serde(default)]
    missing: Vec<MissingFile>,
}

/// "I claimed this and cannot do it." Releases the claim immediately rather
/// than waiting out the heartbeat timeout, and records what was missing.
///
/// The gaps are recorded for humans, not for routing: the client resends its
/// own unsupported set with every claim, which is what makes that state
/// self-correcting when a contributor updates their data.
async fn decline_task(
    State(state): State<AppState>,
    identity: WorkerIdentity,
    Json(body): Json<DeclineBody>,
) -> AppResult<StatusCode> {
    ratelimit::check(&state.limits.worker, &identity.rate_key())?;

    if !matches!(
        body.reason.as_str(),
        "missing_data" | "magpie_version" | "unknown_job_type"
    ) {
        return Err(AppError::bad_request(
            "reason must be 'missing_data', 'magpie_version' or 'unknown_job_type'",
        ));
    }

    let mut tx = state.pool.begin().await?;
    let row = sqlx::query(
        "SELECT c.id, t.job_id
         FROM task_claims c
         JOIN tasks t ON t.id = c.task_id
         WHERE c.claim_token = $1 AND c.state = 'claimed'",
    )
    .bind(body.claim_token)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else {
        // Already released, already submitted, or never existed: nothing the
        // client can do about it either way.
        tx.rollback().await?;
        return Err(AppError::not_found("no open claim with that token"));
    };
    let claim_id: Uuid = row.get("id");
    let job_id: Uuid = row.get("job_id");

    scheduler::release_claim(&mut tx, claim_id, "declined").await?;

    for file in &body.missing {
        sqlx::query(
            "INSERT INTO worker_data_gaps (job_id, claim_id, role, name, expected, actual)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(job_id)
        .bind(claim_id)
        .bind(&file.role)
        .bind(&file.name)
        .bind(&file.expected)
        .bind(&file.actual)
        .execute(&mut *tx)
        .await?;
    }

    audit::log(
        &mut tx,
        "task.declined",
        identity.user_id(),
        identity.anon_uuid(),
        Some("claim"),
        Some(claim_id.to_string()),
        Some(job_id),
    )
    .await?;
    tx.commit().await?;

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct HeartbeatBody {
    claim_token: Uuid,
}

/// Pure liveness ping. Leave-gen progress is derived server-side from accepted
/// results, so a heartbeat carries no payload.
async fn heartbeat(
    State(state): State<AppState>,
    identity: WorkerIdentity,
    Json(body): Json<HeartbeatBody>,
) -> AppResult<StatusCode> {
    ratelimit::check(&state.limits.worker, &identity.rate_key())?;

    sqlx::query(
        "UPDATE task_claims SET last_heartbeat_at = now()
         WHERE claim_token = $1 AND state = 'claimed'",
    )
    .bind(body.claim_token)
    .execute(&state.pool)
    .await?;

    // A heartbeat for a claim that already timed out is not an error the client
    // can do anything with; it will find out when it submits.
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct ResultBody {
    claim_token: Uuid,
    result: serde_json::Value,
}

#[derive(Serialize)]
struct ResultAck {
    accepted: bool,
}

async fn submit_result(
    State(state): State<AppState>,
    identity: WorkerIdentity,
    Json(body): Json<ResultBody>,
) -> AppResult<Json<ResultAck>> {
    ratelimit::check(&state.limits.worker, &identity.rate_key())?;

    let claim = sqlx::query(
        "SELECT c.id, c.task_id, t.job_id
         FROM task_claims c JOIN tasks t ON t.id = c.task_id
         WHERE c.claim_token = $1 AND c.state = 'claimed'",
    )
    .bind(body.claim_token)
    .fetch_optional(&state.pool)
    .await?;

    // A stale token means the claim timed out and was reclaimed. The plan calls
    // for silently ignoring it: the work is already reassigned, and the worker
    // has nothing useful to do with an error.
    let Some(claim) = claim else {
        tracing::debug!(claim_token = %body.claim_token, "ignoring result for stale claim");
        return Ok(Json(ResultAck { accepted: false }));
    };

    let claim_id: Uuid = claim.get("id");
    let task_id: Uuid = claim.get("task_id");
    let job_id: Uuid = claim.get("job_id");

    let job = jobstats::load_job(&state.pool, job_id).await?;

    let mut tx = state.pool.begin().await?;

    crate::jobs::registry::store_result(&mut tx, &job, task_id, claim_id, body.result).await?;

    sqlx::query(
        "UPDATE task_claims SET state = 'completed', completed_at = now() WHERE id = $1",
    )
    .bind(claim_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "UPDATE tasks t
         SET accepted_count = t.accepted_count + 1,
             active_claim_count = GREATEST(t.active_claim_count - 1, 0),
             state = CASE
                 WHEN t.accepted_count + 1 >= j.redundancy THEN 'completed'::task_state
                 WHEN t.accepted_count + 1 + GREATEST(t.active_claim_count - 1, 0) >= j.redundancy
                     THEN 'claimed'::task_state
                 ELSE 'available'::task_state
             END,
             completed_at = CASE
                 WHEN t.accepted_count + 1 >= j.redundancy THEN now()
                 ELSE t.completed_at
             END
         FROM jobs j
         WHERE t.id = $1 AND j.id = t.job_id",
    )
    .bind(task_id)
    .execute(&mut *tx)
    .await?;

    if job.job_type == JobType::GamePairs {
        let config = sqlx::query_as::<_, GamePairConfig>(
            "SELECT * FROM job_game_pair_config WHERE job_id = $1",
        )
        .bind(job_id)
        .fetch_one(&mut *tx)
        .await?;
        ratings::apply_claim(&mut tx, job_id, claim_id, &config).await?;
    }

    audit::log(
        &mut tx,
        "result.submitted",
        identity.user_id(),
        identity.anon_uuid(),
        Some("task"),
        Some(task_id.to_string()),
        Some(job_id),
    )
    .await?;

    tx.commit().await?;

    // SPRT and the finish conditions are evaluated inline on every submission —
    // there is no background sweep.
    let stats = jobstats::compute(&state.pool, &job).await?;
    finish_if_done(&state, &job, &stats).await?;

    if let Ok(payload) = serde_json::to_string(&stats) {
        state.sse.publish(job_id, payload);
    }

    Ok(Json(ResultAck { accepted: true }))
}

/// Auto-complete on either finish condition: SPRT significance (only after
/// `min_units`) or the hard cap.
async fn finish_if_done(state: &AppState, job: &Job, stats: &jobstats::JobStats) -> AppResult<()> {
    let should_finish = match &stats.games {
        Some(games) => games.sprt.status.is_finished(),
        None => match job.job_type {
            JobType::OpeningRack => {
                // Tasks are generated on demand, so "all tasks complete" is not
                // enough -- it is trivially true before anything is dispatched.
                // The job is done once the rack space is exhausted as well.
                let exhausted = sqlx::query_scalar::<_, bool>(
                    "SELECT COALESCE(MAX(t.seed) + c.racks_per_batch, 0) >= c.total_racks
                     FROM job_opening_rack_config c
                     LEFT JOIN tasks t ON t.job_id = c.job_id
                     WHERE c.job_id = $1
                     GROUP BY c.racks_per_batch, c.total_racks",
                )
                .bind(job.id)
                .fetch_optional(&state.pool)
                .await?
                .unwrap_or(false);
                exhausted
                    && stats.tasks_total > 0
                    && stats.tasks_completed >= stats.tasks_total
            }
            // Leave generation completes in `run_transition` once the final
            // generation is aggregated.
            _ => false,
        },
    };

    if should_finish {
        let updated = sqlx::query(
            "UPDATE jobs SET status = 'completed' WHERE id = $1 AND status = 'active'",
        )
        .bind(job.id)
        .execute(&state.pool)
        .await?;
        if updated.rows_affected() > 0 {
            tracing::info!(job_id = %job.id, "job auto-completed");
        }
    }
    Ok(())
}
