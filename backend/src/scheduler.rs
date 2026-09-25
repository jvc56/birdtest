//! Job selection and task claiming — the sequence that runs every time a worker
//! asks for work.

use crate::auth::WorkerIdentity;
use crate::error::{AppError, AppResult};
use crate::jobs::handler::TaskRequest;
use crate::jobs::leave_gen;
use crate::jobs::registry::{self, Acquired};
use crate::jobs::ExpectedFile;
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
    /// Every file the task loads, with the digest the job pins. Shared with
    /// the job's template rather than queried per claim: the set is fixed when
    /// the job is created.
    pub expected_data: std::sync::Arc<Vec<ExpectedFile>>,
    /// The wordmap and rack info table hashes this job's tasks must reproduce.
    /// Empty for a job whose players ask for neither.
    pub derived_data: std::sync::Arc<Vec<crate::derived::ExpectedDerived>>,
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
    /// No job is offering work at all: none is active, or every active one is
    /// parked at 0%. A quiet server is not the worker's fault.
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

/// Active jobs with a share of the fleet, ordered by how far behind that share
/// they are.
///
/// The deficit is `(jobs.claims_issued - jobs.claims_baseline) / allocation`:
/// the claims a job has been issued *since it last joined the jobs on offer*,
/// against its share (see [`join_at_parity`] for the baseline). The counter
/// counts every
/// claim ever issued, abandoned and declined ones included: a claim consumed
/// dispatch capacity the moment it was inserted, so the count only ever goes up.
/// Filtering out abandoned claims would let a job with flaky workers quietly
/// accumulate more than its share and would make the deficit non-monotonic,
/// which is the opposite of what this scheduler needs. It is a counter rather
/// than a `COUNT(*)` over `task_claims` because this query runs on every claim
/// request, and a count grows with the whole history of every candidate job.
///
/// There is no priority. Every active job with an allocation above zero is a
/// candidate, and the worker takes from the one furthest behind its share; a
/// job at 0% is offered to nobody, which is what `inactive` means too, so an
/// admin parks a job with either. The two capability filters -- the worker's
/// MAGPIE version and its unsupported set -- are applied here, so a worker that
/// cannot run one job is offered the next.
async fn candidate_jobs(pool: &PgPool, caps: &WorkerCapabilities) -> AppResult<Vec<Job>> {
    Ok(sqlx::query_as::<_, Job>(
        "SELECT j.*
         FROM jobs j
         WHERE j.status = 'active'
           AND j.allocation > 0
           AND (j.min_magpie_major, j.min_magpie_minor, j.min_magpie_patch)
               <= ($1, $2, $3)
           AND j.id <> ALL($4)
         ORDER BY
           (j.claims_issued - j.claims_baseline)::float / j.allocation ASC,
           j.created_at ASC",
    )
    .bind(caps.magpie_version.major)
    .bind(caps.magpie_version.minor)
    .bind(caps.magpie_version.patch)
    .bind(&caps.unsupported_jobs)
    .fetch_all(pool)
    .await?)
}

/// Put `job_id` level with the jobs already **being served**: set its
/// `claims_baseline` so that its deficit ratio equals the lowest ratio among
/// the other active jobs above 0% that issued a claim within `served_within`
/// -- or, when none has, among all of them; or zero when there are none.
///
/// The deficit the scheduler orders on is a ratio of claims issued to share.
/// Measured over a job's whole life, that made every change to the set of jobs
/// a takeover. A job activated beside one that had issued two million claims
/// had a ratio of zero, so it was first in every candidate list until it had
/// issued two million of its own -- and the older job, at the same 50%, got
/// nothing for as long as that took. A purge zeroes `claims_issued`, so a
/// purged job did the same; so did a job reactivated after a week switched
/// off; and raising an allocation from 10% to 50% cut a job's ratio to a
/// fifth, with the same effect. PLAN.md promised "no starvation of any job
/// above 0%", and the formula it gave did not have that property.
///
/// This is start-time fair queuing's rule: a flow that (re)joins starts at the
/// system's current virtual time, not at zero. Called wherever a job's
/// standing changes -- activation (which is also how an allocation is changed)
/// and purge -- inside that operation's transaction, after it has written the
/// job's new allocation and counters. From then on the job is simply one more
/// candidate: selection stays deterministic, stays one statement, and still
/// converges on the configured shares, because every job's numerator counts
/// from the same moment in the fleet's history.
///
/// **Why "being served", and not merely "offering work".** Virtual time is the
/// position of the flows in service. A job can be active and above 0% and
/// still not be served: its derived files are building (or failed), it is
/// pinned to data the fleet does not have yet, its MAGPIE floor is above what
/// the workers run, its generation is mid-transition. Its ratio stands still
/// while the others climb -- and a newcomer put level with *that* job was, for
/// every worker that could not run the lagging one, first in the list until it
/// had caught up with the jobs that were running: the takeover this function
/// exists to prevent, by another door. (Demonstrated before this was changed:
/// a veteran at 100,000 claims, a job nobody could run at zero, a newcomer --
/// twelve of the next twelve claims went to the newcomer.) `last_claimed_at`
/// rides the `UPDATE jobs` every claim already makes, and `served_within` is
/// the heartbeat timeout: a job that has not issued a claim for that long is
/// not being served in any sense the fleet would notice. With no job served
/// that recently -- a quiet server, the first activation in a while -- every
/// job on offer counts, as before.
///
/// What this does not cover, deliberately: a job only a *minority* of the
/// fleet can run is served, recently, and still lags, so a newcomer joining
/// level with it is ahead of the rest for the majority. No single ratio
/// describes a fleet that is really two queues; PLAN.md records it as a limit.
///
/// The baseline may go negative (a job with no claims joining a busy fleet is
/// credited the claims that put it level); ratios never do, since the minimum
/// it copies is itself a ratio of a count that only grows.
pub async fn join_at_parity(
    conn: &mut sqlx::PgConnection,
    job_id: Uuid,
    served_within: std::time::Duration,
) -> AppResult<()> {
    sqlx::query(
        "WITH others AS (
             SELECT (o.claims_issued - o.claims_baseline)::float8 / o.allocation AS ratio,
                    COALESCE(o.last_claimed_at > now() - make_interval(secs => $2), FALSE) AS served
             FROM jobs o
             WHERE o.status = 'active' AND o.allocation > 0 AND o.id <> $1
         )
         UPDATE jobs j
         SET claims_baseline = j.claims_issued - floor(
                 COALESCE((SELECT MIN(ratio) FROM others WHERE served),
                          (SELECT MIN(ratio) FROM others), 0)
                 * COALESCE(j.allocation, 0)
             )::bigint
         WHERE j.id = $1",
    )
    .bind(job_id)
    .bind(served_within.as_secs_f64())
    .execute(conn)
    .await?;
    Ok(())
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
    // An axis counts as blocking only for jobs that are *offering work* --
    // active and above 0% -- and that it actually rules out. The unsupported
    // set is client-supplied and may name jobs that have since completed or
    // been deactivated; its mere non-emptiness says nothing about why the
    // jobs on offer are out of reach.
    //
    // A job parked at 0% is offered to nobody, which is what `inactive` means,
    // so it is left out exactly as an inactive job is. Counted, a parked job
    // this worker could not run told the worker to shut down -- "every active
    // job requires MAGPIE 0.2.0" -- over a job that was handing out nothing to
    // anyone, where the same job switched to `inactive` answered `204`. A
    // contributor who exits on that does not come back when the admin raises
    // the allocation of a job it could have run all along.
    let row = sqlx::query(
        "SELECT COUNT(*) AS total,
                COUNT(*) FILTER (
                    WHERE (min_magpie_major, min_magpie_minor, min_magpie_patch) > ($1, $2, $3)
                ) AS too_new,
                COUNT(*) FILTER (
                    WHERE (min_magpie_major, min_magpie_minor, min_magpie_patch) <= ($1, $2, $3)
                      AND id = ANY($4)
                ) AS data_blocked
         FROM jobs WHERE status = 'active' AND allocation > 0",
    )
    .bind(caps.magpie_version.major)
    .bind(caps.magpie_version.minor)
    .bind(caps.magpie_version.patch)
    .bind(&caps.unsupported_jobs)
    .fetch_one(&state.pool)
    .await?;

    let total: i64 = row.get("total");
    if total == 0 {
        return Ok(ClaimOutcome::NoWorkExists);
    }
    let version_blocked = row.get::<i64, _>("too_new") > 0;
    let data_blocked = row.get::<i64, _>("data_blocked") > 0;
    if !version_blocked && !data_blocked {
        // Jobs are on offer and nothing rules them out; they simply had no
        // task to hand out this instant.
        return Ok(ClaimOutcome::Idle);
    }

    // The smallest upgrade that would unblock anything, ordered numerically:
    // a MIN over the formatted text would put "0.10.0" before "0.9.0".
    let required_magpie_version: Option<String> = if version_blocked {
        sqlx::query_scalar::<_, String>(
            "SELECT format('%s.%s.%s', min_magpie_major, min_magpie_minor, min_magpie_patch)
             FROM jobs
             WHERE status = 'active' AND allocation > 0
               AND (min_magpie_major, min_magpie_minor, min_magpie_patch) > ($1, $2, $3)
             ORDER BY min_magpie_major, min_magpie_minor, min_magpie_patch
             LIMIT 1",
        )
        .bind(caps.magpie_version.major)
        .bind(caps.magpie_version.minor)
        .bind(caps.magpie_version.patch)
        .fetch_optional(&state.pool)
        .await?
    } else {
        None
    };

    let tarball_dates = if data_blocked {
        sqlx::query_scalar::<_, String>(
            "SELECT DISTINCT d.tarball_date
             FROM jobs j
             JOIN input_data d ON d.id IN (j.letterdist_id, j.layout_id)
             WHERE j.id = ANY($1) AND j.status = 'active' AND j.allocation > 0
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
        required_magpie_version,
    }))
}

/// Lazy timeout reclamation, run at claim time rather than by a background
/// process. Each timed-out claim flips to `abandoned`, the task's
/// `active_claim_count` drops, and a task that was at capacity reopens.
///
/// Safe against a submission for the same claim racing it: the submit path
/// holds the claim row locked from its lookup to its commit, and this
/// statement skips a locked claim rather than waiting on it, so a claim is
/// either abandoned here or completed there -- never both, which is what would
/// decrement the task's counter twice.
///
/// Skipped, not waited on: a claim somebody holds locked is being submitted,
/// declined, purged or deleted, and is not lapsed in any sense that matters.
/// Waiting was what let one purge stall the fleet -- it holds every open claim
/// of its job for as long as its deletes run, and a reclaim run by *any* claim
/// request, for any job, blocked on the first of them once it lapsed, holding
/// a pool connection -- and two reclaims locking overlapping claims in
/// different orders could deadlock. A claim skipped now is reclaimed by the
/// next claim request after its lock is released, if it still needs to be.
pub async fn reclaim_expired(pool: &PgPool, job_id: Uuid, timeout_secs: f64) -> AppResult<u64> {
    reclaim_expired_for(pool, &[job_id], timeout_secs).await
}

/// [`reclaim_expired`] over every candidate job in one statement.
///
/// The scan is the same either way: the planner reaches the expired claims
/// through the partial index on open claims -- one entry per claim currently in
/// flight across the fleet -- and filters by job afterwards, because
/// `task_claims` has no job column to narrow on. Run per job, a claim request
/// therefore paid that scan once per candidate, for a set of rows that does not
/// depend on the job at all. Run once over all of them it is a single pass and
/// a single round trip.
pub async fn reclaim_expired_for(
    pool: &PgPool,
    job_ids: &[Uuid],
    timeout_secs: f64,
) -> AppResult<u64> {
    let result = sqlx::query(
        "WITH lapsed AS (
             SELECT c.id
             FROM task_claims c
             JOIN tasks t ON t.id = c.task_id
             WHERE t.job_id = ANY($1)
               AND c.state = 'claimed'
               AND COALESCE(c.last_heartbeat_at, c.claimed_at) < now() - make_interval(secs => $2)
             FOR UPDATE OF c SKIP LOCKED
         ),
         expired AS (
             UPDATE task_claims c
             SET state = 'abandoned'
             FROM lapsed
             WHERE c.id = lapsed.id
             RETURNING c.task_id
         ),
         counts AS (
             SELECT task_id, COUNT(*)::int AS n FROM expired GROUP BY task_id
         )
         UPDATE tasks t
         SET active_claim_count = GREATEST(t.active_claim_count - counts.n, 0),
             state = CASE
                 WHEN t.accepted_count >= j.redundancy THEN 'completed'::task_state
                 WHEN t.accepted_count + GREATEST(t.active_claim_count - counts.n, 0) >= j.redundancy
                     THEN 'claimed'::task_state
                 ELSE 'available'::task_state
             END
         FROM counts, jobs j
         WHERE t.id = counts.task_id AND j.id = t.job_id",
    )
    .bind(job_ids)
    .bind(timeout_secs)
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}

/// [`reclaim_expired_for`], as the running server calls it: not before this
/// process has been up for the heartbeat timeout.
///
/// A claim lapses when no heartbeat has arrived for the timeout -- and a
/// heartbeat can only arrive at a server that is there to receive it. After an
/// outage longer than the timeout (a deployment that went badly, a database
/// maintenance window, a host that would not come back) every open claim in
/// the fleet has a `last_heartbeat_at` that old, however alive its worker: the
/// workers went on sending heartbeats, to nothing. The first claim request
/// after the restart then abandoned all of them in one statement. Every task
/// in flight was handed out again, and every result the fleet had been
/// computing through the outage -- hours of it, at a task each -- came back to
/// `accepted: false`.
///
/// So the clock starts when the process does. A worker that is alive
/// heartbeats every thirty seconds and refreshes its claim within the first
/// minute; a claim still silent a full timeout after startup has had the same
/// chance to speak as any other and is reclaimed as before. What this costs is
/// that a claim whose worker really did die during the outage is handed out
/// again up to one timeout later than it might have been.
pub async fn reclaim_lapsed(state: &AppState, job_ids: &[Uuid]) -> AppResult<u64> {
    if std::time::Instant::now() < state.reclaim_from {
        return Ok(0);
    }
    // Nor the claims of a job a purge or delete held and then let go without
    // committing: their heartbeats were skipped while it held them, not missed
    // (`jobs::DispatchHolds`).
    let job_ids = state.dispatch_holds.reclaimable(job_ids);
    if job_ids.is_empty() {
        return Ok(0);
    }
    reclaim_expired_for(&state.pool, &job_ids, state.cfg.heartbeat_timeout.as_secs_f64()).await
}

/// Walk the candidate jobs in deficit order and hand out the first available
/// unit of work.
pub async fn claim(
    state: &AppState,
    identity: &WorkerIdentity,
    caps: &WorkerCapabilities,
) -> AppResult<ClaimOutcome> {
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

        // Once for every candidate, before anything is handed out: a task
        // whose claim lapsed has to be back to `available` before the loop
        // below looks for one. Per job this was a round trip per candidate for
        // a scan that does not depend on the job; one statement covers them.
        // A failure here is not fatal -- nothing is reclaimed this time round,
        // so a lapsed task waits for the next claim -- and must not take the
        // whole request down with it.
        let job_ids: Vec<Uuid> = jobs.iter().map(|job| job.id).collect();
        if let Err(err) = reclaim_lapsed(state, &job_ids).await {
            tracing::error!(error = %err.message, "reclaiming expired claims failed");
        }

        let mut retry_outer = false;
        for job in &jobs {
            // One job that cannot dispatch -- a leave-generation job whose
            // generation-0 artifact never got written, a config row a bad
            // restore left out -- must not take every other job down with it.
            // Failing the whole claim here would answer every worker with a
            // 500 for as long as that job sits at the head of the list, and
            // every client retries 500s. It is logged loudly and skipped.
            match try_claim_from_job(state, identity, job, caps).await {
                Ok(Some(outcome)) => return Ok(ClaimOutcome::Task(Box::new(outcome))),
                Ok(None) => continue,
                Err(JobClaimError::Retry) => {
                    retry_outer = true;
                    break;
                }
                Err(JobClaimError::Fatal(err)) => {
                    tracing::error!(job_id = %job.id, error = %err.message, "claiming from job failed; skipping job");
                    continue;
                }
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
    // Its dispatch lock is held for a long time -- seeding, a purge -- and the
    // wait for it would be spent holding a pool connection to learn that there
    // is nothing here right now. See `jobs::DispatchHolds`.
    if state.dispatch_holds.is_held(job.id) {
        return Ok(None);
    }
    // Before the dispatch lock, because a job with a table still building has
    // nothing to hand out and taking the lock would only make every other
    // claim for it wait. The hashes travel with the claim, so a task issued
    // before they exist would either carry no `derived` entry -- which a
    // worker reads as "this server does not check derived files", falling back
    // to whatever is on its disk -- or carry an empty one. Waiting is the only
    // answer that cannot be mistaken for success.
    //
    // Answered from memory once a job has been found dispatchable: this runs
    // for every candidate job on every claim, and the answer for a
    // dispatchable job cannot change for the life of the process (see
    // `derived::DerivedCache`). A job still waiting is asked about each time.
    //
    // The job's template -- its config, players, letter distribution and
    // `expected_data` -- is remembered the same way (`dispatch::JobTemplates`),
    // so the claim transaction below reads only what changes from claim to
    // claim. Only a miss on either takes a pool connection: the common case is
    // a hit, and a connection held for a lookup the cache answers is one a
    // worker's claim or submission is waiting for.
    let (derived, template) = match (state.derived_ready.get(job.id), state.templates.get(job.id)) {
        (Some(derived), Some(template)) => (derived, template),
        (derived, template) => {
            let mut conn =
                state.pool.acquire().await.map_err(|e| JobClaimError::Fatal(e.into()))?;
            let derived = match derived {
                Some(derived) => derived,
                None => {
                    let ready = crate::derived::ready_for_job(
                        &mut conn,
                        job.id,
                        &state.builders,
                        &state.derived_ready,
                    )
                    .await
                    .map_err(JobClaimError::Fatal)?;
                    match ready {
                        Some(ready) => ready,
                        None => return Ok(None),
                    }
                }
            };
            let template = match template {
                Some(template) => template,
                None => state
                    .templates
                    .get_or_load(&mut conn, job)
                    .await
                    .map_err(JobClaimError::Fatal)?,
            };
            (derived, template)
        }
    };

    let mut tx = state.pool.begin().await.map_err(|e| JobClaimError::Fatal(e.into()))?;

    let acquired = match registry::acquire(&mut tx, job, identity, &template).await {
        Ok(acquired) => acquired,
        Err(err) => {
            let _ = tx.rollback().await;
            return Err(if err.is_unique_violation() {
                JobClaimError::Retry
            } else {
                JobClaimError::Fatal(err)
            });
        }
    };

    match acquired {
        Acquired::NoWork => {
            // Committed rather than rolled back, for the one thing a claim that
            // hands out nothing may have written: a leave job's sweep deleting
            // the cursor of a lap it found finished (`leave_gen::sweep`).
            // Rolled back, the next claim would find the lap finished again,
            // by the same read to the end of the universe. Nothing else writes
            // before answering `NoWork`, and a transaction the dispatch lock's
            // timeout aborted commits as the rollback it already is.
            let _ = tx.commit().await;
            Ok(None)
        }
        Acquired::JobFinished => {
            // Written inside this transaction, under the dispatch lock
            // `acquire` took, rather than after rolling it back. A purge takes
            // the same lock; released first, a purge could empty the job
            // between the decision and this update, which then completed the
            // job the purge had just restarted -- for good, since a completed
            // job cannot be reactivated. Guarded on `active` as well: an admin
            // may have deactivated the job between selection and here, and
            // that decision stands.
            sqlx::query("UPDATE jobs SET status = 'completed' WHERE id = $1 AND status = 'active'")
                .bind(job.id)
                .execute(&mut *tx)
                .await
                .map_err(|e| JobClaimError::Fatal(e.into()))?;
            tx.commit().await.map_err(|e| JobClaimError::Fatal(e.into()))?;
            // No submission is coming to push this to open pages.
            crate::routes::worker::push_after_change(state, job.id);
            Ok(None)
        }
        Acquired::NeedsUniverse { generation } => {
            // Nothing was written, and rolling back releases the job's lock so
            // the seeding can take it.
            let _ = tx.rollback().await;
            // Seeded on its own task rather than in this request, for the
            // reason the transition is: it is millions of rows and tens of
            // seconds, and a request can be dropped part-way. MAGPIE gives up
            // on a request after 120 seconds, and a dropped request rolls the
            // seeding back -- so on a database slow enough to take longer than
            // that, every claim started the seeding again and none finished,
            // and the job never left the generation it had just closed.
            let (spawn_state, spawn_job) = (state.clone(), job.clone());
            tokio::spawn(async move {
                if let Err(err) = seed_leave_universe(&spawn_state, &spawn_job, generation).await {
                    tracing::error!(
                        job_id = %spawn_job.id, generation, error = %err.message,
                        "seeding a leave generation's rack universe failed"
                    );
                }
            });
            Ok(None)
        }
        Acquired::NeedsZeroGeneration => {
            let _ = tx.rollback().await;
            // One build per job at a time: every claim finds the KLV missing
            // until the first build lands, and each would otherwise start one.
            static BUILDING: std::sync::Mutex<Vec<Uuid>> = std::sync::Mutex::new(Vec::new());
            {
                let mut building = BUILDING.lock().expect("zero-generation builds poisoned");
                if building.contains(&job.id) {
                    return Ok(None);
                }
                building.push(job.id);
            }
            // Cleared however the build ends -- a panic included, which left
            // the job marked as building until the process restarted.
            struct Built(Uuid);
            impl Drop for Built {
                fn drop(&mut self) {
                    if let Ok(mut building) = BUILDING.lock() {
                        building.retain(|id| *id != self.0);
                    }
                }
            }
            let (spawn_state, spawn_job) = (state.clone(), job.clone());
            tokio::spawn(async move {
                let _built = Built(spawn_job.id);
                let built =
                    crate::jobs::registry::initialize_job_artifacts(&spawn_state, &spawn_job).await;
                if let Err(err) = built {
                    tracing::error!(
                        job_id = %spawn_job.id, error = %err.message,
                        "building a leave job's generation-0 KLV failed"
                    );
                }
            });
            Ok(None)
        }
        Acquired::NeedsLeaveMerge { generation } => {
            // Committed, like `NoWork` and for the same reason: the one thing
            // this transaction may have written is a sweep deleting the cursor
            // of a lap it found finished, on its way to finding that lap's
            // results staged. Rolled back, every claim until the merge landed
            // read from the cursor to the end of the universe to find that out
            // again -- the read `NoWork` commits to avoid. Committing also
            // releases the job's lock before the merge starts.
            //
            // The merge runs on its own task -- a full-size one is the best
            // part of a minute -- and gives up at once if another is already
            // running, so a fleet asking together starts one merge rather
            // than parking a connection each behind it. This job has nothing
            // to hand out until it lands.
            let _ = tx.commit().await;
            let (pool, job_id) = (state.pool.clone(), job.id);
            tokio::spawn(async move {
                if let Err(err) = leave_gen::merge_staged(&pool, job_id, generation, false).await {
                    tracing::error!(
                        %job_id, generation, error = %err.message,
                        "merging staged leave results failed"
                    );
                }
            });
            Ok(None)
        }
        Acquired::NeedsGenerationTransition { generation } => {
            // Committed, not rolled back, and this is load-bearing: the only
            // thing this transaction wrote is the
            // `leave_generation_transitions` row saying this request owns the
            // transition, and a rollback would throw that away -- leaving every
            // claim that arrives while the transition runs free to start
            // another one. Committing also releases the
            // job's advisory lock, which the transition must not hold: it
            // builds a multi-megabyte KLV and uploads it to the object store,
            // and no claim transaction may stay open across that.
            tx.commit().await.map_err(|e| JobClaimError::Fatal(e.into()))?;
            // On its own task, and **not awaited**. A transition takes tens of
            // seconds; awaiting it held this worker's claim request open for
            // all of it, so the one worker unlucky enough to find the
            // generation complete paid the whole aggregation before it could
            // ask for work again. Nothing in this request needs the answer:
            // the generation it would open has no tasks until the transition
            // commits, so this worker's next move is the same either way.
            // Spawning is also what keeps the transition alive when the worker
            // or a load balancer gives up on the request and the handler
            // future is dropped -- run inline it would be abandoned part-way
            // every time, and a generation whose transition outlasts the
            // timeout would never close.
            //
            // Failure is logged rather than returned for the same reason: the
            // transition hands ownership back (`started_at` backdated), so the
            // next claim picks it up.
            let (spawn_state, spawn_job) = (state.clone(), job.clone());
            tokio::spawn(async move {
                if let Err(err) =
                    run_leave_generation_transition(&spawn_state, &spawn_job, generation).await
                {
                    tracing::error!(
                        job_id = %spawn_job.id, generation, error = %err.message,
                        "leave generation transition failed"
                    );
                }
            });
            // This job has nothing to hand out until that finishes; the next
            // candidate may well have work.
            Ok(None)
        }
        Acquired::Task { task_id, request, created } => {
            match issue_claim(&mut tx, identity, job, caps, task_id, created).await {
                // The job stopped being active between its selection and this
                // claim reaching its row: nothing is handed out.
                Ok(None) => {
                    let _ = tx.rollback().await;
                    Ok(None)
                }
                Ok(Some(claim_token)) => {
                    tx.commit().await.map_err(|e| JobClaimError::Fatal(e.into()))?;
                    request_tail_merge(state, job, &template, &request, created);
                    Ok(Some(TaskClaim {
                        job_id: job.id,
                        claim_token,
                        request,
                        min_magpie_version: job.min_magpie_version().to_string(),
                        // Fixed at job creation, so it is the template's copy
                        // rather than a union over six tables inside the
                        // dispatch lock and the job's row lock on every claim.
                        expected_data: template.expected.clone(),
                        derived_data: derived,
                    }))
                }
                Err(err) => {
                    let _ = tx.rollback().await;
                    // The per-identity partial unique index rejects a second
                    // slot on the same task. That is not a failure -- this
                    // worker already holds a slot there -- so re-run selection.
                    Err(if err.is_unique_violation() {
                        JobClaimError::Retry
                    } else {
                        JobClaimError::Fatal(err)
                    })
                }
            }
        }
    }
}

/// A leave claim that was handed fewer racks than a task holds has found its
/// generation nearly done -- everything else below target is out with another
/// claim or waiting in a staged result -- and that is when stale per-rack
/// counts cost most: tasks go out forcing racks that may already be at target,
/// and the generation cannot close until a merge shows that they are. Such a
/// claim asks for a merge, off the request, at most once a minute per job
/// (`leave_gen::TAIL_MERGE_INTERVAL`). Mid-generation the half-hourly sweep is
/// enough, and this never fires.
fn request_tail_merge(
    state: &AppState,
    job: &Job,
    template: &crate::jobs::dispatch::JobTemplate,
    request: &TaskRequest,
    created: bool,
) {
    let (TaskRequest::LeaveGeneration(request), crate::jobs::dispatch::JobKind::LeaveGeneration { config, .. }) =
        (request, &template.kind)
    else {
        return;
    };
    // A re-dispatched task's racks were chosen when it was created, and say
    // nothing about the generation now.
    if !created || request.forced_racks.len() >= config.racks_per_task as usize {
        return;
    }
    if !state.leave_merges.due(job.id) {
        return;
    }
    let (pool, job_id, generation) = (state.pool.clone(), job.id, request.generation);
    tokio::spawn(async move {
        if let Err(err) = leave_gen::merge_staged(&pool, job_id, generation, false).await {
            tracing::error!(
                %job_id, generation, error = %err.message,
                "merging staged leave results near a generation's end failed"
            );
        }
    });
}

/// Everything a claim writes, inside the claim transaction.
///
/// `None` when the job is no longer active by the time the claim reaches its
/// row; the caller rolls everything back.
async fn issue_claim(
    tx: &mut sqlx::PgTransaction<'_>,
    identity: &WorkerIdentity,
    job: &Job,
    caps: &WorkerCapabilities,
    task_id: Uuid,
    task_created: bool,
) -> AppResult<Option<Uuid>> {
    // A worker that arrived with no identity becomes a real one only now,
    // when there is a task to attach it to and a response body to return its
    // UUID in.
    if let Some(uuid) = identity.newly_assigned_uuid() {
        sqlx::query("INSERT INTO anonymous_workers (uuid) VALUES ($1) ON CONFLICT DO NOTHING")
            .bind(uuid)
            .execute(&mut **tx)
            .await?;
    }

    let claim_token = Uuid::new_v4();
    sqlx::query(
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
    .execute(&mut **tx)
    .await?;

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
    .execute(&mut **tx)
    .await?;

    // No audit row. The claim row just inserted records who claimed which task
    // and when, so a `task.claimed` audit row said nothing it did not -- while
    // costing a write per claim on the path a worker waits on, and most of
    // `audit_log`'s growth.

    // Last, because it locks the job row: claims against the same job
    // serialize on it until commit, so it should be held for as little of the
    // transaction as possible. `tasks_total` rides along for the same reason --
    // a second statement to count a created task would take the same lock
    // earlier and buy nothing.
    //
    // Guarded on `active`. The job was selected as active before this
    // transaction held any lock on it, and completing it -- the stopping rule
    // or an admin -- and deactivating it both update this row. Waiting on that
    // update and then updating regardless handed out a task of a job that was
    // already completed or switched off: work nobody wanted, and for a
    // completed job a result landing after an export had checked that nothing
    // was in flight. Postgres re-checks the condition on the row it waited
    // for, so a claim that loses that race hands out nothing.
    let still_active = sqlx::query(
        "UPDATE jobs
         SET claims_issued = claims_issued + 1, tasks_total = tasks_total + $2,
             last_claimed_at = now()
         WHERE id = $1 AND status = 'active'",
    )
    .bind(job.id)
    .bind(i64::from(task_created))
    .execute(&mut **tx)
    .await?
    .rows_affected()
        > 0;

    Ok(still_active.then_some(claim_token))
}

/// Release a claim that ended in something other than a submission.
///
/// Decline and expiry are the same operation -- mark the claim terminal, drop
/// the job's live claim count, recompute the task against `redundancy` -- and
/// writing it twice is how the counter drifts. A drifting counter makes the
/// scheduler believe a job is saturated and dispatch quietly stops, days later
/// and nowhere near the cause.
///
/// Acts only on a claim that is still `claimed`, and returns whether it did:
/// releasing a claim twice would decrement the counter twice.
pub async fn release_claim(
    tx: &mut sqlx::PgTransaction<'_>,
    claim_id: Uuid,
    terminal_state: &str,
) -> AppResult<bool> {
    let released = sqlx::query(
        "UPDATE task_claims SET state = $2::claim_state WHERE id = $1 AND state = 'claimed'",
    )
    .bind(claim_id)
    .bind(terminal_state)
    .execute(&mut **tx)
    .await?
    .rows_affected()
        > 0;
    if !released {
        return Ok(false);
    }

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

    Ok(true)
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

    // This request owns the transition: `next_step` wrote the
    // `leave_generation_transitions` row that stops any other claim starting the
    // same one. If it fails, ownership has to go back, or the generation would
    // sit untouched until the takeover timeout expired -- a transient object
    // store error would cost half an hour of idle workers. Backdating
    // `started_at` hands it to the next claim through the same takeover path,
    // which keeps the attempt count and says in the log that a transition was
    // started and did not finish.
    let result = leave_gen::run_transition(
        state,
        job.id,
        generation,
        &config,
        &job_data.letterdist,
    )
    .await;

    let key = match result {
        Ok(key) => key,
        Err(err) => {
            // The original failure is what the caller needs to see, so a
            // failure to hand ownership back is logged rather than returned in
            // its place; the takeover timeout still covers it.
            if let Err(release) = sqlx::query(
                "UPDATE leave_generation_transitions SET started_at = to_timestamp(0)
                 WHERE job_id = $1 AND generation = $2 AND completed_at IS NULL",
            )
            .bind(job.id)
            .bind(generation)
            .execute(&state.pool)
            .await
            {
                tracing::error!(
                    job_id = %job.id, generation, error = %release,
                    "could not release a failed generation transition"
                );
            }
            return Err(err);
        }
    };
    tracing::info!(job_id = %job.id, generation, artifact_key = %key, "leave generation complete");
    Ok(())
}

/// Seed a leave generation's rack universe, off any request.
///
/// Holds the job's dispatch lock for as long as the seeding runs, which is
/// what keeps claims from reading the half-written universe: each waits its
/// bounded two seconds, is told there is nothing here right now, and moves on
/// to another job. The lock is taken without waiting, because whoever holds it
/// is either a claim -- which asks for this again if the universe is still
/// missing when it looks -- or another copy of this seeding, and waiting would
/// hold a pool connection for the duration of either.
///
/// A seeding that is interrupted rolls back whole, and the next claim that
/// finds the universe missing starts it again.
async fn seed_leave_universe(state: &AppState, job: &Job, generation: i32) -> AppResult<()> {
    let mut tx = state.pool.begin().await?;
    if !crate::jobs::try_lock_job_dispatch_now(&mut tx, job.id).await? {
        return Ok(());
    }
    // Until the commit below releases the lock.
    let _hold = state.dispatch_holds.hold(
        job.id,
        crate::jobs::HoldKind::DispatchOnly,
        state.cfg.heartbeat_timeout,
    );
    let job_data = crate::jobs::load_job_data(&mut tx, job.id).await?;
    leave_gen::ensure_universe(&mut tx, job.id, generation, &job_data.letterdist).await?;
    tx.commit().await?;
    Ok(())
}
