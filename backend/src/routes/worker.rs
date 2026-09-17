use crate::audit;
use crate::auth::WorkerIdentity;
use crate::error::{AppError, AppResult};
use crate::jobs::handler::TaskRequest;
use crate::jobstats;
use crate::models::job::{Job, JobStatus, JobType};
use crate::scheduler;
use crate::state::AppState;
use axum::extract::{DefaultBodyLimit, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

/// The largest result body `/api/worker/result` accepts.
///
/// Axum's default is 2 MB, which a `games_per_batch` of a few hundred with
/// `capture_positions` on already exceeds: every position of every game is
/// captured, at a few KB each. A body over the limit is refused before it is
/// parsed, so this is also the bound on how much memory one submission can
/// make the server allocate. Batch size is the admin's lever for staying under
/// it; see PLAN.md, "What bounds a submission?".
pub const MAX_RESULT_BYTES: usize = 64 * 1024 * 1024;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/client-version", get(client_version))
        .route("/task", post(claim_task))
        .route("/decline", post(decline_task))
        .route("/heartbeat", post(heartbeat))
        .route(
            "/result",
            post(submit_result).layer(DefaultBodyLimit::max(MAX_RESULT_BYTES)),
        )
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

/// Version negotiation. MAGPIE is the only production client and cannot
/// replace itself, so this reports a floor rather than offering an update.
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
    identity.check_rate_limit(&state)?;
    identity.require_registered()?;

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
    files: std::sync::Arc<Vec<crate::jobs::ExpectedFile>>,
    /// The files the worker builds for itself -- a wordmap, a rack info table
    /// -- with the SHA-256 the server's own pinned MAGPIE got from the same
    /// inputs. Neither can be shipped (179 MB and 1.9 GB for CSW24), so the
    /// hash is what travels instead: the worker builds its own copy and uses
    /// it only if the bytes agree.
    ///
    /// Always serialized, empty included. A missing key would be read by a
    /// client as an older server that checks nothing, which is precisely the
    /// state this replaces; `[]` says "this job needs no derived file".
    derived: std::sync::Arc<Vec<crate::derived::ExpectedDerived>>,
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
    identity.check_rate_limit(&state)?;

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
        // A request that arrived with no identity is not assigned one here:
        // there is no body to carry it in, and nothing was written for it. It
        // is issued one with its first task.
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
            expected_data: ExpectedData {
                algorithm: "sha256",
                files: claim.expected_data,
                derived: claim.derived_data,
            },
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
    /// `missing_data`, `magpie_version`, `unknown_job_type`, `derived_mismatch`
    /// or `task_failed`.
    reason: String,
    #[serde(default)]
    missing: Vec<MissingFile>,
}

/// A task loads at most a handful of files, so an honest decline names a
/// handful. Past this the list is truncated: every entry is a row in
/// `worker_data_gaps`, and an unbounded list is an unbounded write.
const MAX_MISSING_FILES: usize = 32;
/// Role, name and digest are all short; anything longer is not one of them.
const MAX_GAP_FIELD_CHARS: usize = 128;

fn bounded(text: &str) -> String {
    text.chars().take(MAX_GAP_FIELD_CHARS).collect()
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
    identity.check_rate_limit(&state)?;
    identity.require_registered()?;

    // `task_failed` is a worker that ran the task and could not produce a
    // result the server accepted. Declining hands the slot straight back, where
    // stopping the heartbeat alone held it for the whole heartbeat timeout.
    // `derived_mismatch` is a worker that built the wordmap or rack info table
    // this job pins and got different bytes. Distinct from `missing_data`,
    // which is a file the contributor was supposed to have downloaded: nothing
    // the contributor can do fixes this one, and the two hashes it carries are
    // the evidence that the fleet's builders disagree -- which is exactly what
    // should be visible in the admin view rather than worked around silently.
    if !matches!(
        body.reason.as_str(),
        "missing_data"
            | "magpie_version"
            | "unknown_job_type"
            | "derived_mismatch"
            | "task_failed"
    ) {
        return Err(AppError::bad_request(
            "reason must be 'missing_data', 'magpie_version', 'unknown_job_type', \
             'derived_mismatch' or 'task_failed'",
        ));
    }

    let mut tx = state.pool.begin().await?;
    // Locked, so a timeout reclaiming this claim concurrently cannot release
    // it a second time.
    let row = sqlx::query(
        "SELECT c.id, t.job_id
         FROM task_claims c
         JOIN tasks t ON t.id = c.task_id
         WHERE c.claim_token = $1 AND c.state = 'claimed'
           AND c.claimed_by_user_id IS NOT DISTINCT FROM $2
           AND c.claimed_by_anon_uuid IS NOT DISTINCT FROM $3
         FOR UPDATE OF c",
    )
    .bind(body.claim_token)
    .bind(identity.user_id())
    .bind(identity.anon_uuid())
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else {
        // Already released, already submitted, never existed, or claimed by
        // a different identity: nothing the client can do about it either way.
        tx.rollback().await?;
        return Err(AppError::not_found("no open claim with that token"));
    };
    let claim_id: Uuid = row.get("id");
    let job_id: Uuid = row.get("job_id");

    scheduler::release_claim(&mut tx, claim_id, "declined").await?;

    for file in body.missing.iter().take(MAX_MISSING_FILES) {
        sqlx::query(
            "INSERT INTO worker_data_gaps (job_id, claim_id, role, name, expected, actual)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(job_id)
        .bind(claim_id)
        .bind(bounded(&file.role))
        .bind(bounded(&file.name))
        .bind(bounded(&file.expected))
        .bind(file.actual.as_deref().map(bounded))
        .execute(&mut *tx)
        .await?;
    }

    // The reason is recorded, not just validated. `worker_data_gaps` says what
    // was missing when a decline named files, but nothing said *why* a claim
    // came back, so a worker that declined and a worker that vanished were
    // indistinguishable afterwards -- and a client failing a task locally is
    // invisible to the server entirely. It goes on the audit row rather than on
    // `task_claims` because it describes an event rather than state.
    audit::log_worker_detail(
        &mut tx,
        "task.declined",
        identity.user_id(),
        identity.anon_uuid(),
        "claim",
        claim_id.to_string(),
        Some(job_id),
        body.reason.clone(),
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
    identity.check_rate_limit(&state)?;
    identity.require_registered()?;

    sqlx::query(
        "UPDATE task_claims SET last_heartbeat_at = now()
         WHERE claim_token = $1 AND state = 'claimed'
           AND claimed_by_user_id IS NOT DISTINCT FROM $2
           AND claimed_by_anon_uuid IS NOT DISTINCT FROM $3",
    )
    .bind(body.claim_token)
    .bind(identity.user_id())
    .bind(identity.anon_uuid())
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
    identity.check_rate_limit(&state)?;
    identity.require_registered()?;

    let mut tx = state.pool.begin().await?;

    // The claim is looked up and locked inside the transaction that completes
    // it. Looked up outside it, a timeout could reclaim the claim between the
    // lookup and the write -- the claim would then be marked both abandoned and
    // completed, the task's live-claim counter decremented twice, and a task
    // another worker had just been handed would read as unclaimed. The same
    // lock is what makes a retried submission of an already-accepted result a
    // clean `accepted: false` instead of a duplicate-key error.
    let claim = sqlx::query(
        "SELECT c.id, c.task_id, t.job_id
         FROM task_claims c JOIN tasks t ON t.id = c.task_id
         WHERE c.claim_token = $1 AND c.state = 'claimed'
           AND c.claimed_by_user_id IS NOT DISTINCT FROM $2
           AND c.claimed_by_anon_uuid IS NOT DISTINCT FROM $3
         FOR UPDATE OF c",
    )
    .bind(body.claim_token)
    .bind(identity.user_id())
    .bind(identity.anon_uuid())
    .fetch_optional(&mut *tx)
    .await?;

    // A token is bound to the identity it was issued to (the checks above), so
    // a ban or an audit row means what it says: a token handed to another
    // identity is treated exactly like an unknown one.
    //
    // A stale token means the claim timed out and was reclaimed, or this
    // result was already accepted. The work is reassigned or done, and the
    // worker has nothing useful to do with an error.
    let Some(claim) = claim else {
        tx.rollback().await?;
        tracing::debug!(claim_token = %body.claim_token, "ignoring result for stale claim");
        return Ok(Json(ResultAck { accepted: false }));
    };

    let claim_id: Uuid = claim.get("id");
    let task_id: Uuid = claim.get("task_id");
    let job_id: Uuid = claim.get("job_id");

    // Submissions for the same task serialize here, on the task row, before
    // anything is stored. Redundant claims of one task hold different claim
    // rows, so the lock above does not order them, and what a submission
    // stores depends on what the task's earlier submissions stored: only the
    // first accepted result adds to the job's running progress totals
    // (`registry::store_result`). Without this, two submissions arriving
    // together each saw only their own uncommitted rows and both counted. The
    // task row is locked by the update below anyway; taking it first keeps the
    // order every path uses -- claim, then task, then job.
    //
    // The lock is also what makes `accepted_count` trustworthy here: every
    // accepted result increments it in the transaction that stores the result,
    // and that transaction holds this lock, so a zero read under it means no
    // result for the task has been accepted before this one. That is what
    // "first accepted result" means everywhere it is used, and reading it is
    // one row where counting the stored results was up to 10,000 of them for
    // an opening-rack batch.
    let prior_accepted: i32 =
        sqlx::query_scalar("SELECT accepted_count FROM tasks WHERE id = $1 FOR UPDATE")
            .bind(task_id)
            .fetch_one(&mut *tx)
            .await?;

    let job = sqlx::query_as::<_, Job>("SELECT * FROM jobs WHERE id = $1")
        .bind(job_id)
        .fetch_one(&mut *tx)
        .await?;

    // The job's immutable half: its batch size, its players' reporting caps,
    // its rack space. Read once per process; a hit costs no round trip inside
    // the locks held here.
    let template = state.templates.get_or_load(&mut tx, &job).await?;

    crate::jobs::registry::store_result(
        &mut tx,
        &template,
        &job,
        task_id,
        claim_id,
        prior_accepted == 0,
        body.result,
    )
    .await?;

    sqlx::query(
        "UPDATE task_claims SET state = 'completed', completed_at = now() WHERE id = $1",
    )
    .bind(claim_id)
    .execute(&mut *tx)
    .await?;

    // `RETURNING` the new state is what tells the job's `tasks_completed`
    // counter that a task has actually *reached* completed. A task makes that
    // transition exactly once -- once `accepted_count` meets `redundancy` the
    // task stops being dispatched, and a claim that lapsed before then is
    // abandoned, so its late submission is refused above -- which is what makes
    // counting on the transition safe rather than approximate.
    let task_completed = sqlx::query_scalar::<_, bool>(
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
         WHERE t.id = $1 AND j.id = t.job_id
         RETURNING t.state = 'completed'",
    )
    .bind(task_id)
    .fetch_one(&mut *tx)
    .await?;

    if task_completed {
        sqlx::query("UPDATE jobs SET tasks_completed = tasks_completed + 1 WHERE id = $1")
            .bind(job_id)
            .execute(&mut *tx)
            .await?;
    }

    // The contributor's own running total, which is what the leaderboards read
    // instead of counting this identity's claims. One statement, on the row the
    // identity already owns. Deliberately not rolled back by account deletion:
    // the account is anonymized in place and keeps its claims, so no donated
    // compute is lost. `purge_job` and `delete_job` *do* decrement it, because
    // unlike the counters on `jobs` this one spans every job the identity ever
    // worked on.
    match (identity.user_id(), identity.anon_uuid()) {
        (Some(user_id), _) => {
            sqlx::query(
                "UPDATE users SET tasks_completed = tasks_completed + 1,
                                  last_completed_at = now()
                 WHERE id = $1",
            )
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        }
        (None, Some(uuid)) => {
            sqlx::query(
                "UPDATE anonymous_workers SET tasks_completed = tasks_completed + 1,
                                              last_completed_at = now()
                 WHERE uuid = $1",
            )
            .bind(uuid)
            .execute(&mut *tx)
            .await?;
        }
        (None, None) => {}
    }

    // Ratings are deliberately not touched here. A fit is global to a rating
    // pool and nothing in the submission path depends on it, so it runs on a
    // periodic sweep (see ratings::recompute_stale) rather than inside every
    // result transaction.
    //
    // No audit row either: the claim, now `completed` with its
    // `completed_at`, and the stored result say everything a
    // `result.submitted` row said, and the row was a write per submission on
    // the path a worker waits on.

    tx.commit().await?;

    // The result is committed; nothing below can un-accept it. Failing the
    // request now would tell the worker to retry a submission that already
    // landed, so a failure here is logged and the next submission's check
    // picks the job up.
    //
    // `job` is the row read inside the transaction above, before this result
    // was stored and before the finish check reads any result -- which is the
    // order `complete_unless_purged`'s witness needs -- so it is reused rather
    // than read again on the path the worker waits on.
    if let Err(err) = after_submission(&state, &job).await {
        tracing::error!(job_id = %job_id, error = %err.message, "post-submission bookkeeping failed");
    }

    Ok(Json(ResultAck { accepted: true }))
}

/// Everything that has to happen after a result lands, split by whether the
/// submitting worker has to wait for it.
///
/// **Inline:** the finish conditions. SPRT gates whether the job keeps
/// dispatching, so it is evaluated on every submission and the aggregates it
/// needs are read once, here.
///
/// **Spawned:** the live stats payload. It is display-only -- nothing in the
/// claim path reads a statistic -- and it is the most expensive thing in this
/// path, several aggregates over the job's whole history. Building it before
/// answering the worker made the next claim wait on a dashboard nobody may
/// have open. It is coalesced per job (`sse::begin_push`), so a busy job
/// builds one payload at a time rather than one per submission, and they stay
/// ordered because one task issues them.
async fn after_submission(state: &AppState, job: &Job) -> AppResult<()> {
    let job_id = job.id;

    // Leave generation finishes in its own transition and has no finish
    // condition here, so it skips the in-flight query `should_check_finish`
    // would otherwise run on seven submissions out of eight.
    //
    // `job.status` is as of the submit transaction. A job deactivated or
    // completed since is guarded by `complete_unless_purged`'s own predicate,
    // so a stale `active` here costs a check and never a wrong write.
    if job.status == JobStatus::Active
        && job.job_type != JobType::LeaveGeneration
        && should_check_finish(state, job_id).await?
        && finish_condition_met(state, job).await?
    {
        // `job` was loaded before the results were read, which is what lets
        // its `claims_issued` tell a purge in between from no purge at all.
        if crate::jobs::complete_unless_purged(&state.pool, job.id, job.claims_issued).await? {
            tracing::info!(job_id = %job.id, "job auto-completed");
            // Completion is final, so this job will never need checking again.
            state.finish_checks.forget(job_id);
        }
    }

    // Checked here so a job nobody is watching costs nothing at all; the
    // payload itself is built off this request.
    if state.sse.has_subscribers(job_id) && state.sse.begin_push(job_id) {
        let state = state.clone();
        tokio::spawn(async move { push_stats_until_idle(&state, job_id).await });
    }
    Ok(())
}

/// Build and publish the job's stats, repeating while submissions asked for
/// another round while the last was building. Owned by one task per job, so
/// pushes never overtake each other.
async fn push_stats_until_idle(state: &AppState, job_id: Uuid) {
    loop {
        // Reloaded each round rather than carried in: the status may have
        // changed since the submission that asked for this, and a payload
        // saying `active` for a job that just completed is exactly the
        // staleness the dashboard would notice.
        // On the display pool: this is a dashboard payload, and must not take
        // a connection from the pool the submission that asked for it used.
        match jobstats::load_job(&state.read_pool, job_id).await {
            Ok(job) => match jobstats::compute(&state.read_pool, &job).await {
                Ok(stats) => {
                    if let Ok(payload) = serde_json::to_string(&stats) {
                        state.sse.publish(job_id, payload);
                    }
                }
                Err(err) => tracing::warn!(
                    job_id = %job_id, error = %err.message, "building live job stats failed"
                ),
            },
            Err(err) => tracing::warn!(
                job_id = %job_id, error = %err.message, "loading a job for its live stats failed"
            ),
        }
        if !state.sse.end_push(job_id) {
            return;
        }
        // Another round was asked for while this one built. On a busy job that
        // is every round, so without a pause the loop rebuilt the payload back
        // to back -- several aggregates over the job's history, one after
        // another, for as long as a dashboard stayed open, on a pool of twenty
        // connections the claim and submit paths share. Submissions arriving
        // during the pause still coalesce into the one round that follows it.
        tokio::time::sleep(MIN_STATS_PUSH_INTERVAL).await;
    }
}

/// The shortest gap between two live stats pushes for one job. The dashboard
/// lags a busy job by at most this much, which a human watching it cannot tell
/// from live.
const MIN_STATS_PUSH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

/// Whether this submission is the one that evaluates the job's finish
/// conditions.
///
/// Every `SPRT_CHECK_EVERY`th, which bounds how much work a job can do past its
/// stopping point at `SPRT_CHECK_EVERY - 1` tasks — see the constant for why
/// the first several of those cost nothing.
///
/// **Plus, unconditionally, when this job has nothing left in flight.** The
/// check is triggered *by* submissions, so a job whose contributors all stop
/// between checks would not be evaluated again until work resumed — which for a
/// job that has already reached its stopping point means never, leaving it
/// `active` and holding its allocation. The `EXISTS` below is
/// bounded by the number of claims open across the fleet, not by anything that
/// grows with the job, and it is only reached when the debounce would otherwise
/// skip.
async fn should_check_finish(state: &AppState, job_id: Uuid) -> AppResult<bool> {
    if state.finish_checks.should_check(job_id) {
        return Ok(true);
    }
    Ok(sqlx::query_scalar::<_, bool>(
        "SELECT NOT EXISTS (
             SELECT 1 FROM task_claims c JOIN tasks t ON t.id = c.task_id
             WHERE t.job_id = $1 AND c.state = 'claimed'
         )",
    )
    .bind(job_id)
    .fetch_one(&state.pool)
    .await?)
}

/// Either finish condition: SPRT significance (only after `min_units`) or the
/// hard cap for game jobs; an exhausted and fully completed rack space for
/// opening racks.
async fn finish_condition_met(state: &AppState, job: &Job) -> AppResult<bool> {
    Ok(match job.job_type {
        JobType::Games | JobType::GamePairs => jobstats::game_stats(&state.pool, job)
            .await?
            .is_some_and(|games| games.sprt.status.is_finished()),
        JobType::OpeningRack => {
            // Tasks are generated on demand, so "all tasks complete" is not
            // enough -- it is trivially true before anything is dispatched.
            // The job is done once the rack space is exhausted as well.
            sqlx::query_scalar::<_, bool>(
                "SELECT COALESCE(MAX(t.seed) + c.racks_per_batch, 0) >= c.total_racks
                        AND COUNT(t.id) > 0
                        AND COUNT(t.id) FILTER (WHERE t.state <> 'completed') = 0
                 FROM job_opening_rack_config c
                 LEFT JOIN tasks t ON t.job_id = c.job_id
                 WHERE c.job_id = $1
                 GROUP BY c.racks_per_batch, c.total_racks",
            )
            .bind(job.id)
            .fetch_optional(&state.pool)
            .await?
            .unwrap_or(false)
        }
        // Leave generation completes in `run_transition` once the final
        // generation is aggregated.
        JobType::LeaveGeneration => false,
    })
}

// ---------------------------------------------------------------------------

/// The worker API is a cross-repo boundary: birdtest serves it and MAGPIE's
/// `contribute` command speaks it, released independently. `contract-fixtures/`
/// holds one committed example of each message either side has to produce or
/// read; these tests are what make those files load-bearing on this side, so a
/// field renamed here fails a test rather than a contributor's run.
///
/// Client → server messages are checked by parsing the fixture into the type
/// that actually handles the request. Server → client messages are checked by
/// key structure rather than byte equality: fields are still free to move
/// before the first release, so pinning exact bytes would make every additive
/// change a fixture edit, but a *renamed* or *dropped* field is exactly what
/// this needs to catch.
#[cfg(test)]
mod contract_fixtures {
    use super::*;
    use serde_json::{json, Value};

    /// The set of keys at every level, as a comparable tree. Values are
    /// ignored; only shape matters.
    fn shape(v: &Value) -> Value {
        match v {
            Value::Object(map) => Value::Object(
                map.iter().map(|(k, v)| (k.clone(), shape(v))).collect(),
            ),
            // An array's elements must agree, so the first one stands for
            // all. An array of scalars pins nothing whether it is empty or
            // not, since a scalar carries no field names -- and that
            // distinction is load-bearing here: a `magpie_too_old` shutdown
            // legitimately has an empty `required_tarball_dates` where a
            // `data_out_of_date` one does not.
            Value::Array(items) => match items.first() {
                Some(first) if first.is_object() || first.is_array() => json!([shape(first)]),
                _ => json!([]),
            },
            _ => Value::Null,
        }
    }

    fn assert_same_shape(fixture: &Value, produced: &Value, what: &str) {
        assert_eq!(
            shape(fixture),
            shape(produced),
            "{what}: contract-fixtures/ and the wire type disagree on field names.\n\
             Update both repositories together, or the fixture is now a lie."
        );
    }

    #[test]
    fn claim_request_parses_as_a_claim_body() {
        let body: ClaimBody =
            serde_json::from_str(include_str!("../../../contract-fixtures/claim-request.json"))
                .expect("claim-request.json no longer parses as ClaimBody");
        assert_eq!(body.magpie_version, "1.4.0");
        assert_eq!(body.unsupported_jobs.len(), 1);
    }

    #[test]
    fn decline_parses_as_a_decline_body() {
        let body: DeclineBody = serde_json::from_str(include_str!(
            "../../../contract-fixtures/decline-missing-data.json"
        ))
        .expect("decline-missing-data.json no longer parses as DeclineBody");
        assert_eq!(body.reason, "missing_data");
        // One file absent entirely and one present with the wrong bytes: the
        // two cases `actual` exists to tell apart.
        assert_eq!(body.missing.len(), 2);
        assert!(body.missing.iter().any(|m| m.actual.is_none()));
        assert!(body.missing.iter().any(|m| m.actual.is_some()));
    }

    #[test]
    fn assignments_carry_a_task_request_this_build_understands() {
        for (name, fixture) in [
            ("games", include_str!("../../../contract-fixtures/assignment-games.json")),
            (
                "opening-rack",
                include_str!("../../../contract-fixtures/assignment-opening-rack.json"),
            ),
            (
                "leave-generation",
                include_str!("../../../contract-fixtures/assignment-leave-generation.json"),
            ),
        ] {
            let value: Value = serde_json::from_str(fixture).unwrap();
            let request: TaskRequest =
                serde_json::from_value(value["task_request"].clone())
                    .unwrap_or_else(|e| panic!("assignment-{name}.json task_request: {e}"));
            // Round-trips: the client reads what the server would have written.
            assert_same_shape(
                &value["task_request"],
                &serde_json::to_value(&request).unwrap(),
                &format!("assignment-{name} task_request"),
            );
        }
    }

    #[test]
    fn the_assignment_envelope_matches_what_the_server_sends() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../../contract-fixtures/assignment-games.json"))
                .unwrap();
        let request: TaskRequest =
            serde_json::from_value(fixture["task_request"].clone()).unwrap();

        let produced = serde_json::to_value(TaskAssignment {
            claim_token: Uuid::nil(),
            job_id: Uuid::nil(),
            task_request: request,
            min_magpie_version: "1.4.0".into(),
            expected_data: ExpectedData {
                algorithm: "sha256",
                files: std::sync::Arc::new(vec![crate::jobs::ExpectedFile {
                    role: "kwg".into(),
                    name: "NWL23".into(),
                    path: "lexica/NWL23.kwg".into(),
                    sha256: "3e74af98".into(),
                    bytes: 4_719_596,
                    tarball_date: "20251004".into(),
                }]),
                derived: std::sync::Arc::new(vec![crate::derived::ExpectedDerived {
                    role: "wmp".into(),
                    name: "NWL23".into(),
                    sha256: "214a46d7".into(),
                    bytes: 104_857_600,
                    builder: "wmp-1".into(),
                    build_target: "nehalem".into(),
                }]),
            },
            // Absent from the fixture: it is sent only to a worker that
            // arrived with no identity at all, which this one did not.
            worker_uuid: None,
        })
        .unwrap();

        assert_same_shape(&fixture, &produced, "assignment envelope");
    }

    #[test]
    fn every_shutdown_reason_matches_what_the_server_sends() {
        for (reason, fixture) in [
            (
                "data_out_of_date",
                include_str!("../../../contract-fixtures/shutdown-data-out-of-date.json"),
            ),
            (
                "magpie_too_old",
                include_str!("../../../contract-fixtures/shutdown-magpie-too-old.json"),
            ),
            ("both", include_str!("../../../contract-fixtures/shutdown-both.json")),
        ] {
            let value: Value = serde_json::from_str(fixture).unwrap();
            assert_eq!(
                value["shutdown"]["reason"], reason,
                "shutdown fixture for {reason} names a different reason"
            );

            let produced = serde_json::to_value(ShutdownResponse {
                shutdown: scheduler::ShutdownDirective {
                    reason: reason.into(),
                    message: "…".into(),
                    required_tarball_dates: vec!["20260101".into()],
                    required_magpie_version: Some("1.6.0".into()),
                    download_url: Some("https://github.com/jvc56/MAGPIE".into()),
                },
            })
            .unwrap();
            assert_same_shape(&value, &produced, &format!("shutdown ({reason})"));
        }

        // "both" leads with the MAGPIE version, because updating MAGPIE is the
        // remedy that fixes both: a release bumps DATA_VERSION and the
        // contributor runs download_data.sh as part of updating.
        let both: Value =
            serde_json::from_str(include_str!("../../../contract-fixtures/shutdown-both.json"))
                .unwrap();
        assert!(
            !both["shutdown"]["required_magpie_version"].is_null(),
            "the `both` shutdown must name a MAGPIE version to lead with"
        );
    }

    #[test]
    fn decline_gap_fields_are_bounded() {
        let long = "x".repeat(10_000);
        assert_eq!(bounded(&long).chars().count(), MAX_GAP_FIELD_CHARS);
        assert_eq!(bounded("kwg"), "kwg");
    }
}
