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

#[derive(Serialize)]
struct TaskAssignment {
    claim_token: Uuid,
    job_id: Uuid,
    task_request: TaskRequest,
    min_magpie_version: Option<String>,
    /// Present only when the request carried no identity at all and the
    /// server just minted one. The client persists this and sends it as
    /// `X-Worker-UUID` on every later request.
    #[serde(skip_serializing_if = "Option::is_none")]
    worker_uuid: Option<Uuid>,
}

/// The task claim: a minimal message saying "I am ready for work". Everything
/// about the assignment is the server's decision.
async fn claim_task(State(state): State<AppState>, identity: WorkerIdentity) -> AppResult<Response> {
    ratelimit::check(&state.limits.worker, &identity.rate_key())?;

    match scheduler::claim(&state, &identity).await? {
        // 204 rather than an error: "no work right now" is the normal state of a
        // quiet server, and the client just sleeps and asks again. A brand new
        // anonymous identity minted for this request is not reported here --
        // there is no body to carry it in, and a client that finds no work has
        // nothing to persist yet. It tries again with no identity next time,
        // and gets one for keeps once a task is actually available.
        None => Ok(StatusCode::NO_CONTENT.into_response()),
        Some(outcome) => Ok(Json(TaskAssignment {
            claim_token: outcome.claim_token,
            job_id: outcome.job_id,
            task_request: outcome.request,
            min_magpie_version: outcome.min_magpie_version,
            worker_uuid: identity.newly_assigned_uuid(),
        })
        .into_response()),
    }
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
            JobType::OpeningRackAnalysis => {
                stats.tasks_total > 0 && stats.tasks_completed >= stats.tasks_total
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
