use super::dispatch::JobTemplate;
use super::handler::*;
use super::racks::{LetterDistribution, RackIndex};
use super::JobData;
use crate::artifacts::ArtifactStore;
use crate::error::{AppError, AppResult};
use crate::magpie::{Builders, Magpie, ScratchData};
use crate::models::job::LeaveConfig;
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, Row};
use uuid::Uuid;

/// Tiles on a full rack. Leave generation observes full racks, never leaves,
/// and MAGPIE's `RACK_SIZE` is the same seven.
pub const RACK_SIZE: usize = 7;

/// The name the server hands MAGPIE for a generation's files inside a scratch
/// directory. Nothing outside that directory ever sees it.
const SCRATCH_KLV_NAME: &str = "generation";

pub struct LeaveGenHandler;

/// Writes the typed request row alongside the task.
pub async fn insert_request(
    conn: &mut PgConnection,
    task_id: Uuid,
    req: &LeaveRequest,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO leave_requests
             (task_id, lexicon, variant, letter_distribution, board_layout, generation,
              seed, forced_racks, num_games, previous_artifact_key, use_wordmap)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(task_id)
    .bind(&req.lexicon)
    .bind(&req.variant)
    .bind(&req.letter_distribution)
    .bind(&req.board_layout)
    .bind(req.generation)
    .bind(req.seed as i64)
    .bind(&req.forced_racks)
    .bind(req.num_games)
    .bind(&req.previous_artifact_key)
    .bind(req.use_wordmap)
    .execute(conn)
    .await?;
    Ok(())
}

impl JobHandler for LeaveGenHandler {
    type Request = LeaveRequest;
    type Response = LeaveResponse;
    type Record = LeaveRecord;

    async fn load_request(
        conn: &mut PgConnection,
        template: &JobTemplate,
        task_id: Uuid,
    ) -> AppResult<Self::Request> {
        // The artifact's hash is read from the row that names it rather than
        // stored with the request: one source, whichever path sends the task.
        let row = sqlx::query(
            "SELECT r.lexicon, r.variant, r.letter_distribution, r.board_layout, r.generation,
                    r.seed, r.forced_racks, r.num_games, r.previous_artifact_key, r.use_wordmap,
                    COALESCE(a.served_sha256, a.sha256) AS previous_artifact_sha256
             FROM leave_requests r
             JOIN leave_generation_artifacts a ON a.artifact_key = r.previous_artifact_key
             WHERE r.task_id = $1",
        )
        .bind(task_id)
        .fetch_one(&mut *conn)
        .await?;
        Ok(LeaveRequest {
            lexicon: row.get("lexicon"),
            variant: row.get("variant"),
            letter_distribution: row.get("letter_distribution"),
            board_layout: row.get("board_layout"),
            generation: row.get("generation"),
            seed: row.get::<i64, _>("seed") as u64,
            forced_racks: row.get("forced_racks"),
            num_games: row.get("num_games"),
            previous_artifact_key: row.get("previous_artifact_key"),
            previous_artifact_sha256: row.get("previous_artifact_sha256"),
            use_wordmap: row.get("use_wordmap"),
            bingo_bonus: template.data.bingo_bonus,
        })
    }

    fn process_response(response: Self::Response) -> AppResult<Self::Record> {
        if response.racks.is_empty() {
            return Err(AppError::bad_request("leave result carried no rack occurrences"));
        }
        super::plausibility::check_rack_occurrences(&response.racks)?;
        Ok(LeaveRecord { racks: response.racks })
    }

    /// Credits the claim and stages its occurrences for the generation's next
    /// merge ([`stage_fold`]). Only the first accepted result for a task is
    /// staged -- see [`credit_claim`] for the others.
    async fn insert_record(
        conn: &mut PgConnection,
        template: &JobTemplate,
        task_id: Uuid,
        claim_id: Uuid,
        record: &Self::Record,
    ) -> AppResult<()> {
        credit_claim(conn, task_id, claim_id, record).await?;
        stage_fold(conn, template.job_id, task_id, record).await
    }
}

/// Records that a claim did a task's work, without adding its occurrences to
/// the generation.
///
/// This is all a redundant result gets. With redundancy above 1 every claim of
/// a task plays the same seed, so folding each of them in counted the same
/// games `redundancy` times -- a generation reached its occurrence target on
/// a fraction of the coverage it names, and closed early. Every other job
/// type's aggregates already read one result per task, the first accepted
/// (PLAN.md, "Redundant task execution"); this is the leave-generation half of
/// that rule.
pub async fn credit_claim(
    conn: &mut PgConnection,
    task_id: Uuid,
    claim_id: Uuid,
    record: &LeaveRecord,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO leave_records (task_claim_id, task_id, rack_count)
         VALUES ($1, $2, $3)",
    )
    .bind(claim_id)
    .bind(task_id)
    .bind(record.racks.len() as i32)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Record a task's occurrences for its generation, without touching the
/// generation's per-rack rows.
///
/// **Why this is an append and not the fold itself.** A submission used to run
/// `UPDATE leave_rack_progress ... FROM UNNEST(...)` over every rack its games
/// drew -- tens to hundreds of thousands of rows scattered uniformly over a
/// table of 3.2 million. Measured on a seeded generation: 2.5 to 5.5 seconds
/// inside the transaction the worker waits on, with every other submission of
/// the generation queued behind its row locks; **147 to 409 MB of WAL for one
/// fold** of 40,000 racks, because the first touch of each page after a
/// checkpoint writes the whole page; and not one HOT update, because
/// `occurrence_count` is an indexed column (claim-time selection orders on it),
/// so every row written was a new heap tuple, a new entry in both indexes and
/// a dead tuple for autovacuum. At a fold a minute that is on the order of
/// ten gigabytes of WAL an hour per leave job.
///
/// Nothing needs the per-rack totals that promptly. Selection needs them
/// roughly; closing a generation and building its KLV need them exactly, but
/// only at that moment. So a submission writes **one row** here -- its racks,
/// counts and equity sums as three arrays, which Postgres compresses and
/// stores out of line -- plus the generation's live counters, and
/// [`merge_staged`] folds everything staged into `leave_rack_progress` in one
/// pass: periodically, when a claim finds the generation nearly done, and
/// always before a generation closes. The submit path drops to milliseconds,
/// and the only lock it takes that another submission wants is the
/// generation's one counter row, for a single-row update -- as it already does
/// on the job's own counters.
///
/// A rack that is not a full rack of the job's distribution is staged like
/// any other and dropped by the merge, which updates rows and creates none.
async fn stage_fold(
    conn: &mut PgConnection,
    job_id: Uuid,
    task_id: Uuid,
    record: &LeaveRecord,
) -> AppResult<()> {
    let row = sqlx::query("SELECT generation, num_games FROM leave_requests WHERE task_id = $1")
        .bind(task_id)
        .fetch_one(&mut *conn)
        .await?;
    let generation: i32 = row.get("generation");
    let num_games: i32 = row.get("num_games");

    // A result for a generation that has already been aggregated is credited
    // to the worker -- it did the work, and the claim completes normally --
    // but must not be staged. The generation's KLV is already built and
    // uploaded, so nothing will ever read these occurrences; merging them would
    // only make the rows disagree with the artifact built from them, which is
    // the one signal reserved for a corrupted or stale object (see
    // `rebuild_artifacts`).
    //
    // Defence in depth, not a path the claim flow takes: a generation closes
    // only when none of its claims is still `claimed`, a claim that times out
    // is abandoned (and its submission refused before reaching here), and a
    // reopened task is reissued only while its own generation is current. What
    // remains is state the flow never writes -- a partial restore, a hand edit
    // -- and folding into a built generation is the one outcome worth guarding
    // against there.
    let closed = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM leave_generation_artifacts
                        WHERE job_id = $1 AND generation = $2)",
    )
    .bind(job_id)
    .bind(generation)
    .fetch_one(&mut *conn)
    .await?;
    if closed {
        tracing::warn!(
            job_id = %job_id, generation, task_id = %task_id,
            racks = record.racks.len(),
            "discarding a leave result for a generation that has already closed"
        );
        return Ok(());
    }

    let racks: Vec<&str> = record.racks.iter().map(|o| o.rack.as_str()).collect();
    let counts: Vec<i64> = record.racks.iter().map(|o| o.count).collect();
    let sums: Vec<f64> = record.racks.iter().map(|o| o.mean * o.count as f64).collect();

    sqlx::query(
        "INSERT INTO leave_rack_staging (job_id, generation, task_id, racks, counts, equity_sums)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(job_id)
    .bind(generation)
    .bind(task_id)
    .bind(&racks)
    .bind(&counts)
    .bind(&sums)
    .execute(&mut *conn)
    .await?;

    // The generation's live figures: exact, and current to the submission. One
    // row per generation, so submissions of one generation take turns on it for
    // a single-row update -- as they already do on the job's own counters.
    sqlx::query(
        "INSERT INTO leave_generation_progress (job_id, generation, tasks_completed, games_played)
         VALUES ($1, $2, 1, $3)
         ON CONFLICT (job_id, generation) DO UPDATE
             SET tasks_completed = leave_generation_progress.tasks_completed + 1,
                 games_played = leave_generation_progress.games_played + EXCLUDED.games_played",
    )
    .bind(job_id)
    .bind(generation)
    .bind(i64::from(num_games))
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// The advisory-lock namespace for merges, distinct from dispatch's (1) and
/// the rating fits' (2).
const MERGE_LOCK_NAMESPACE: i32 = 3;

/// Take `job_id`'s merge lock for the rest of the caller's transaction, waiting
/// out a merge that is running.
///
/// [`merge_staged`] takes it, and so must anything else that writes both
/// `leave_rack_staging` and `leave_rack_progress` for a job -- which is a purge
/// and a job delete. A merge takes the staged rows first and then updates the
/// per-rack rows, in whatever order its plan visits them; a purge deleted the
/// per-rack rows first and the staged rows after. Run together, the purge
/// stopped on a rack the merge had updated while holding racks the merge had
/// yet to reach, and the merge then stopped on one of those: a deadlock, which
/// Postgres breaks by failing one of the two -- the purge, as a `500` with
/// nothing deleted, or the merge. A full-size merge runs for a minute or more,
/// so that window is not small. Taken *before* the job's dispatch lock and
/// before any row, the purge waits here holding nothing, and no merge starts
/// until it has committed.
///
/// Nothing takes this lock while holding another, so it is first in the lock
/// order everywhere it appears: merge, then dispatch, then claim, task, job.
pub async fn lock_merges(conn: &mut PgConnection, job_id: Uuid) -> AppResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock($1, hashtext($2::text))")
        .bind(MERGE_LOCK_NAMESPACE)
        .bind(job_id)
        .execute(conn)
        .await?;
    Ok(())
}

/// What one merge did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct MergeOutcome {
    /// Staged submissions folded in.
    pub folds_merged: i64,
    /// `leave_rack_progress` rows they changed.
    pub racks_updated: i64,
}

/// Fold everything staged for one generation into `leave_rack_progress`, and
/// refresh the generation's summary (racks at target, the rack furthest from
/// it) while the rows are warm.
///
/// One statement takes the staged rows and applies their sum, so a staged
/// submission is either still staged or folded in, never both and never
/// neither; a submission that commits while this runs is simply not in the
/// statement's snapshot, and waits for the next merge.
///
/// Merges of one job serialize on an advisory lock: two of them would
/// otherwise each take a disjoint set of staged rows and then update
/// overlapping racks in whatever order their plans visited them, which is a
/// deadlock. `wait` decides what a second caller does. The transition **waits**
/// -- it must not read a generation's totals until everything staged is in
/// them. Everything else gives up (`None`): whoever holds the lock is doing
/// the same work, and waiting would hold a pool connection for the minute a
/// full-size merge can take, once per caller.
///
/// A merge rewrites up to every row of the generation and none of them can be
/// a HOT update, exactly as before -- but once per merge rather than once per
/// submission, which is the whole saving.
pub async fn merge_staged(
    pool: &sqlx::PgPool,
    job_id: Uuid,
    generation: i32,
    wait: bool,
) -> AppResult<Option<MergeOutcome>> {
    let mut tx = pool.begin().await?;
    if wait {
        lock_merges(&mut tx, job_id).await?;
    } else {
        let taken: bool =
            sqlx::query_scalar("SELECT pg_try_advisory_xact_lock($1, hashtext($2::text))")
                .bind(MERGE_LOCK_NAMESPACE)
                .bind(job_id)
                .fetch_one(&mut *tx)
                .await?;
        if !taken {
            return Ok(None);
        }
    }

    let started = std::time::Instant::now();
    // An UPDATE rather than an upsert: the generation's universe is every full
    // rack, seeded up front, so a rack with no row is not a rack of this
    // distribution and must not create one.
    let row = sqlx::query(
        "WITH taken AS (
             DELETE FROM leave_rack_staging
             WHERE job_id = $1 AND generation = $2
             RETURNING racks, counts, equity_sums
         ),
         folded AS (
             SELECT u.rack, SUM(u.count)::bigint AS count, SUM(u.equity_sum) AS equity_sum
             FROM taken, UNNEST(taken.racks, taken.counts, taken.equity_sums)
                  AS u(rack, count, equity_sum)
             GROUP BY u.rack
         ),
         applied AS (
             UPDATE leave_rack_progress p SET
                 occurrence_count = p.occurrence_count + f.count,
                 equity_sum       = p.equity_sum + f.equity_sum,
                 updated_at       = now()
             FROM folded f
             WHERE p.job_id = $1 AND p.generation = $2 AND p.rack = f.rack
             RETURNING 1
         )
         SELECT (SELECT COUNT(*) FROM taken) AS folds, (SELECT COUNT(*) FROM applied) AS racks",
    )
    .bind(job_id)
    .bind(generation)
    .fetch_one(&mut *tx)
    .await?;
    let outcome =
        MergeOutcome { folds_merged: row.get("folds"), racks_updated: row.get("racks") };

    // The summary the dashboard reads, recomputed only when the rows moved (or
    // it has never been computed): a count over the generation, which was the
    // most expensive read in a leave job's live stats and ran on every push.
    let summarised: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM leave_generation_progress
                        WHERE job_id = $1 AND generation = $2 AND merged_at IS NOT NULL)",
    )
    .bind(job_id)
    .bind(generation)
    .fetch_one(&mut *tx)
    .await?;
    if outcome.folds_merged > 0 || !summarised {
        refresh_summary(&mut tx, job_id, generation).await?;
    }
    tx.commit().await?;

    if outcome.folds_merged > 0 {
        tracing::info!(
            job_id = %job_id, generation, folds = outcome.folds_merged,
            racks = outcome.racks_updated, elapsed_ms = started.elapsed().as_millis(),
            "merged staged leave results"
        );
    }
    Ok(Some(outcome))
}

/// Recompute a generation's summary row from its per-rack rows.
async fn refresh_summary(conn: &mut PgConnection, job_id: Uuid, generation: i32) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO leave_generation_progress
             (job_id, generation, racks_total, racks_at_target, min_rack, min_rack_count, merged_at)
         SELECT $1, $2, totals.total, totals.at_target, lowest.rack, lowest.occurrence_count, now()
         FROM (SELECT COUNT(*)::bigint AS total,
                      COUNT(*) FILTER (WHERE p.occurrence_count >= c.target_rack_count)::bigint
                          AS at_target
               FROM leave_rack_progress p
               JOIN job_leave_config c ON c.job_id = p.job_id
               WHERE p.job_id = $1 AND p.generation = $2) totals
         LEFT JOIN LATERAL (
               SELECT rack, occurrence_count FROM leave_rack_progress
               WHERE job_id = $1 AND generation = $2
               ORDER BY occurrence_count ASC LIMIT 1) lowest ON TRUE
         ON CONFLICT (job_id, generation) DO UPDATE
             SET racks_total = EXCLUDED.racks_total,
                 racks_at_target = EXCLUDED.racks_at_target,
                 min_rack = EXCLUDED.min_rack,
                 min_rack_count = EXCLUDED.min_rack_count,
                 merged_at = EXCLUDED.merged_at",
    )
    .bind(job_id)
    .bind(generation)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// [`merge_staged`] for every generation of `job_id` with anything staged,
/// which in practice is the current one.
pub async fn merge_staged_for_job(
    pool: &sqlx::PgPool,
    job_id: Uuid,
    wait: bool,
) -> AppResult<MergeOutcome> {
    let generations: Vec<i32> = sqlx::query_scalar(
        "SELECT DISTINCT generation FROM leave_rack_staging WHERE job_id = $1 ORDER BY 1",
    )
    .bind(job_id)
    .fetch_all(pool)
    .await?;
    let mut total = MergeOutcome { folds_merged: 0, racks_updated: 0 };
    for generation in generations {
        if let Some(outcome) = merge_staged(pool, job_id, generation, wait).await? {
            total.folds_merged += outcome.folds_merged;
            total.racks_updated += outcome.racks_updated;
        }
    }
    Ok(total)
}

/// The periodic sweep: merge whatever is staged, for every job. One job's
/// failure does not stop the others.
pub async fn merge_all_staged(pool: &sqlx::PgPool) -> AppResult<i64> {
    let jobs: Vec<Uuid> = sqlx::query_scalar("SELECT DISTINCT job_id FROM leave_rack_staging")
        .fetch_all(pool)
        .await?;
    let mut folds = 0;
    for job_id in jobs {
        match merge_staged_for_job(pool, job_id, false).await {
            Ok(outcome) => folds += outcome.folds_merged,
            Err(err) => tracing::error!(
                %job_id, error = %err.message, "merging staged leave results failed; skipping job"
            ),
        }
    }
    Ok(folds)
}

/// How often the sweep in `main.rs` merges. A merge touches most pages of a
/// generation whatever it carries, so its cost is per merge, not per
/// submission, and the interval is what sets the write volume: half an hour is
/// roughly a gigabyte of WAL an hour for a full-size job, against ten when
/// every submission folded itself. What it costs is freshness nobody needs:
/// selection's counts and the dashboard's "racks at target" lag by up to this
/// long mid-generation (racks a staged task forced are excluded from selection
/// meanwhile, see [`next_step`]), and near a generation's end claims ask for a
/// merge themselves ([`TAIL_MERGE_INTERVAL`]).
pub const MERGE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// The shortest gap between merges a *claim* asks for. A claim that is handed
/// fewer racks than a task holds has found the generation nearly done, which
/// is when staleness costs most: every task dispatched on old counts forces
/// racks that may already be at target, and the generation cannot close until
/// a merge shows that they are.
pub const TAIL_MERGE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

/// When each job's last claim-requested merge started, so a busy tail asks
/// once a minute rather than once a claim. In memory: losing it on a restart
/// costs one early merge.
#[derive(Clone, Default)]
pub struct TailMerges(std::sync::Arc<std::sync::Mutex<std::collections::HashMap<Uuid, std::time::Instant>>>);

impl TailMerges {
    /// Whether a claim-requested merge for `job_id` is due, and if so, notes
    /// that one is starting now.
    pub fn due(&self, job_id: Uuid) -> bool {
        let mut last = self.0.lock().expect("tail merge map poisoned");
        let now = std::time::Instant::now();
        match last.get(&job_id) {
            Some(started) if now.duration_since(*started) < TAIL_MERGE_INTERVAL => false,
            _ => {
                last.insert(job_id, now);
                true
            }
        }
    }

    pub fn forget(&self, job_id: Uuid) {
        self.0.lock().expect("tail merge map poisoned").remove(&job_id);
    }
}

/// How long a started transition may go without finishing before another claim
/// takes it over.
///
/// Only reached if the process died mid-transition: a live transition holds no
/// lock and leaves no heartbeat, so its row is the only evidence it exists, and
/// without a timeout a crash would stall the job permanently. Far longer than a
/// transition takes (about 15 seconds on the dev database, about a minute on the
/// largest measured one) so a slow one is never taken over
/// while it is still working -- a duplicate is exactly what the row exists to
/// prevent.
const TRANSITION_TAKEOVER_AFTER: &str = "30 minutes";

/// Startup: hand every transition a previous process left open to the next
/// claim, instead of leaving it to the takeover timeout.
///
/// A transition runs on a spawned task and leaves no heartbeat, so its row is
/// the only evidence it exists, and [`TRANSITION_TAKEOVER_AFTER`] is what
/// covers a process that died part-way. But the ordinary way a process dies is
/// a deployment, and birdtest runs as a single instance whose deployments stop
/// the old task before starting the new one (PLAN.md, "Decisions settled
/// before Phase 2") -- so at startup every open transition belongs to a process
/// that is gone, exactly as every `running` import and export does, and
/// waiting out the timeout left that job with nothing to hand out for half an
/// hour after every deploy that happened to land inside one. Backdating
/// `started_at` is the same hand-back a *failed* transition performs
/// (`scheduler::run_leave_generation_transition`): the next claim takes it
/// over through the usual path, and `attempts` records that it happened.
pub async fn release_orphaned_transitions(pool: &sqlx::PgPool) -> AppResult<u64> {
    Ok(sqlx::query(
        "UPDATE leave_generation_transitions SET started_at = to_timestamp(0)
         WHERE completed_at IS NULL AND started_at > to_timestamp(0)",
    )
    .execute(pool)
    .await?
    .rows_affected())
}

/// Serialize this job's claim decisions against each other.
///
/// Every read `next_step` makes -- which racks are below target, which are out
/// with an open claim, whether any claim for the generation is still in flight
/// -- is invisible to a *concurrent* claim transaction until that transaction
/// commits. Two consequences:
///
/// - a claim still being issued is not counted as in flight, so the generation
///   it belongs to could be closed while its task was going out, and the work
///   that task did would land in a generation whose KLV was already built;
/// - two claims could both find the generation complete and both start its
///   transition.
///
/// The lock is taken per job, so claims for other jobs are unaffected, and it is
/// transaction-scoped: it is released when the claim transaction commits or
/// rolls back, whichever happens, and a dropped connection releases it too.
/// It is *not* held across the transition itself -- that would hold a Postgres
/// transaction open across an S3 upload -- so what stops a second transition is
/// the `leave_generation_transitions` row this lock makes it safe to test and
/// write.
///
/// It is [`super::try_lock_job_dispatch`], which every job type now takes for
/// the same underlying reason; leave generation just has the most to lose by
/// not holding it -- and the most to gain from the bounded wait, since seeding
/// a generation's rack universe holds this lock for tens of seconds.
///
/// `false` means another claim holds it and this one should move on.
pub async fn lock_claim_decisions(conn: &mut PgConnection, job_id: Uuid) -> AppResult<bool> {
    super::try_lock_job_dispatch(conn, job_id).await
}

/// What the scheduler should do next for a leave-generation job.
pub enum LeaveGenStep {
    /// Dispatch this forced-rack partition.
    Dispatch(LeaveRequest),
    /// Every rack in this generation hit its target and no claim is in flight;
    /// the generation must be aggregated before any more work exists. Done
    /// outside the claim transaction because it uploads to S3.
    Transition { generation: i32 },
    /// A transition for this generation is already running (or, past the
    /// takeover timeout, was running when the process died and has just been
    /// taken over by this caller). Distinct from `Transition` only in who is
    /// responsible for it.
    TransitionInProgress { generation: i32 },
    /// All configured generations are complete.
    Finished,
    /// Nothing is left to hand out and nothing is in flight, but submissions
    /// are still staged, so whether the generation is *complete* is not yet
    /// known: the racks they forced are held out of selection, and their
    /// occurrences are not in the totals. The caller merges, and the next
    /// claim decides on exact figures.
    NeedsMerge { generation: i32 },
    /// Every rack below target is already out with an open claim for this
    /// generation (or every rack has reached target and claims are still in
    /// flight). Their results may yet land, so the generation cannot be
    /// closed, and handing the same racks out again would only duplicate
    /// coverage. Nothing to hand out right now.
    NoWorkYet,
}

/// Whether `generation`'s transition is owned by a request that is still
/// working on it.
///
/// Distinct from asking whether the generation has closed: between the claim
/// that commits the `leave_generation_transitions` row and the transition's own
/// commit, the artifact row does not exist yet, so `current_generation` still
/// names the closing generation and nothing else marks it as off limits. That
/// window is tens of seconds -- streaming millions of rows, deriving leave
/// values, uploading the KLV -- and anything dispatched inside it plays racks
/// whose totals the transition is in the middle of reading.
///
/// A row past the takeover timeout is deliberately *not* counted: that is a
/// transition whose process died, and `next_step` exists to take it over. Using
/// the same bound as the takeover keeps the two decisions from disagreeing,
/// which would stall the job permanently.
pub async fn transition_in_progress(
    conn: &mut PgConnection,
    job_id: Uuid,
    generation: i32,
) -> AppResult<bool> {
    Ok(sqlx::query_scalar::<_, bool>(&format!(
        "SELECT EXISTS (
             SELECT 1 FROM leave_generation_transitions
             WHERE job_id = $1 AND generation = $2 AND completed_at IS NULL
               AND started_at >= now() - interval '{TRANSITION_TAKEOVER_AFTER}'
         )"
    ))
    .bind(job_id)
    .bind(generation)
    .fetch_one(&mut *conn)
    .await?)
}

/// The generation claims are currently for: one past the last completed, or
/// `None` once every configured generation is complete.
pub async fn current_generation(
    conn: &mut PgConnection,
    job_id: Uuid,
    config: &LeaveConfig,
) -> AppResult<Option<i32>> {
    // Generation 0 has an artifact too -- the zeroed KLV generation 1 plays
    // with -- so it must not count as a completed generation.
    let completed = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM leave_generation_artifacts
         WHERE job_id = $1 AND generation >= 1",
    )
    .bind(job_id)
    .fetch_one(&mut *conn)
    .await?;
    Ok((completed < config.generation_count as i64).then_some(completed as i32 + 1))
}

/// A generation has "many" racks below target -- and is selected by sweep --
/// while there are more than this many tasks' worth of them, as of the last
/// merge. The factor is where the two selections cost the same: a sweep skips
/// at-target rows to find its racks, about `universe / below_target` rows per
/// rack handed out, and the tail selection hashes every rack that is out, at
/// most `below_target` of them. At a hundred tasks' worth both are bounded by
/// about a hundredth of the universe -- some 32,000 rows for English, tens of
/// milliseconds -- whatever the fleet's size and however much is staged.
pub const SWEEP_WHILE_TASKS_REMAIN: i64 = 100;

/// Claim-time rack selection, in one of two ways.
///
/// **While many racks are below target: a sweep** ([`sweep`]). The generation's
/// racks are handed out in primary-key order from a cursor that is remembered
/// between claims (`leave_selection_cursors`), a lap at a time. Everything
/// behind the cursor has been handed out this lap and nothing ahead of it has,
/// so no claim needs to be told what is out: selection costs the same with one
/// result staged as with ten thousand. It was not always so. Racks were taken
/// lowest count first, and between merges the racks of every staged result --
/// whose counts have not moved yet -- are exactly the lowest: each claim built
/// a hash of all of them and walked past them, about a microsecond a rack,
/// inside the job's dispatch lock. At a hundred workers that is a second a
/// claim, and the lock's other claimants give up after two.
///
/// **Once few remain: lowest count first** ([`furthest_below_target`]), with
/// everything that is out excluded. A sweep would spend the end of a generation
/// walking past racks already at target; here the set below target is small by
/// construction, so the exclusion is too.
///
/// Which one is decided from the generation's summary row, which a merge
/// refreshes -- the same age as the counts both selections read. Going from the
/// first to the second is safe at any moment, because the second excludes
/// everything out; nothing goes the other way, since counts only grow.
///
/// `lexicon` is the name of the row the job pins, from its template.
pub async fn next_step(
    conn: &mut PgConnection,
    job_id: Uuid,
    config: &LeaveConfig,
    job_data: &JobData,
    lexicon: &str,
) -> AppResult<LeaveGenStep> {
    let Some(generation) = current_generation(&mut *conn, job_id, config).await? else {
        return Ok(LeaveGenStep::Finished);
    };

    // Absent until the universe is seeded, which the caller has checked; read
    // as "few" if it is missing anyway, since that selection assumes nothing.
    let below_target: i64 = sqlx::query_scalar(
        "SELECT racks_total - racks_at_target FROM leave_generation_progress
         WHERE job_id = $1 AND generation = $2",
    )
    .bind(job_id)
    .bind(generation)
    .fetch_optional(&mut *conn)
    .await?
    .unwrap_or(0);

    let selected = if below_target > SWEEP_WHILE_TASKS_REMAIN * i64::from(config.racks_per_task) {
        sweep(conn, job_id, generation, config).await?
    } else {
        furthest_below_target(conn, job_id, generation, config).await?
    };
    let racks = match selected {
        Selected::Racks(racks) => racks,
        Selected::Step(step) => return Ok(step),
    };

    // Never optional: generation 1 reads the zeroed KLV written at generation
    // 0 when the job was created, so every generation fetches its leaves the
    // same way.
    let (previous_artifact_key, previous_artifact_sha256) = sqlx::query_as::<_, (String, String)>(
        "SELECT artifact_key, COALESCE(served_sha256, sha256) FROM leave_generation_artifacts
         WHERE job_id = $1 AND generation = $2",
    )
    .bind(job_id)
    .bind(generation - 1)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| {
        AppError::internal(format!(
            "leave job {job_id} has no generation-{} KLV to play generation {generation} with",
            generation - 1
        ))
    })?;

    Ok(LeaveGenStep::Dispatch(LeaveRequest {
        lexicon: lexicon.to_string(),
        variant: job_data.variant.clone(),
        letter_distribution: job_data.letterdist_name.clone(),
        board_layout: job_data.layout_name.clone(),
        generation,
        // Drawn fresh per task rather than derived from the job, so two tasks
        // of one generation do not replay the same games over different
        // forced racks; stored with the request, so a reissued task replays
        // its own.
        seed: rand::random(),
        forced_racks: racks,
        previous_artifact_key,
        previous_artifact_sha256,
        num_games: config.num_iterations,
        use_wordmap: config.use_wordmap,
        bingo_bonus: job_data.bingo_bonus,
    }))
}

/// What a selection came back with: racks to force, or the reason there are
/// none.
enum Selected {
    Racks(Vec<String>),
    Step(LeaveGenStep),
}

/// Claims of this generation still `claimed`.
async fn claims_in_flight(conn: &mut PgConnection, job_id: Uuid, generation: i32) -> AppResult<i64> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)
         FROM task_claims c
         JOIN leave_requests r ON r.task_id = c.task_id
         JOIN tasks t ON t.id = c.task_id
         WHERE t.job_id = $1 AND r.generation = $2 AND c.state = 'claimed'",
    )
    .bind(job_id)
    .bind(generation)
    .fetch_one(&mut *conn)
    .await?)
}

/// Whether any accepted result of this generation is still waiting for a merge.
async fn anything_staged(conn: &mut PgConnection, job_id: Uuid, generation: i32) -> AppResult<bool> {
    Ok(sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM leave_rack_staging WHERE job_id = $1 AND generation = $2)",
    )
    .bind(job_id)
    .bind(generation)
    .fetch_one(&mut *conn)
    .await?)
}

/// Why nothing can be handed out when no rack was selectable -- or, when the
/// answer is "because every rack is at target", the start of the transition.
///
/// **The order of the two reads is load-bearing.** Submissions are not
/// serialized with claims, so one may commit between any two statements here.
/// Claims in flight are read first: a submission that commits after that read
/// was counted as in flight, and one that committed before it is visible to the
/// second read as staged. There is no moment at which a result is neither.
async fn nothing_to_hand_out(
    conn: &mut PgConnection,
    job_id: Uuid,
    generation: i32,
) -> AppResult<Option<LeaveGenStep>> {
    if claims_in_flight(conn, job_id, generation).await? > 0 {
        return Ok(Some(LeaveGenStep::NoWorkYet));
    }
    // With nothing in flight, what is staged is all that the per-rack counts
    // are missing. Until it is merged it is not known whether the generation
    // is complete, and deciding that it is would close it on totals that are
    // missing every staged result -- safe for the KLV (the transition drains
    // first) but wrong for the racks those tasks left short of target, which
    // would never be forced again.
    if anything_staged(conn, job_id, generation).await? {
        return Ok(Some(LeaveGenStep::NeedsMerge { generation }));
    }
    Ok(None)
}

/// The generation is complete. Whoever writes this row owns its transition;
/// everyone else waits. Safe to test and write without re-reading because
/// `lock_claim_decisions` holds the job's lock for the rest of this
/// transaction, so no other claim is between its own test and its own write.
///
/// A row whose transition never finished is taken over rather than trusted
/// forever -- see TRANSITION_TAKEOVER_AFTER. Taking over bumps `attempts`,
/// which is the only place a crash mid-transition is recorded.
async fn claim_transition(
    conn: &mut PgConnection,
    job_id: Uuid,
    generation: i32,
) -> AppResult<LeaveGenStep> {
    let claimed = sqlx::query_scalar::<_, bool>(&format!(
        "INSERT INTO leave_generation_transitions (job_id, generation)
         VALUES ($1, $2)
         ON CONFLICT (job_id, generation) DO UPDATE
             SET started_at = now(), attempts = leave_generation_transitions.attempts + 1
             WHERE leave_generation_transitions.completed_at IS NULL
               AND leave_generation_transitions.started_at
                   < now() - interval '{TRANSITION_TAKEOVER_AFTER}'
         RETURNING attempts > 1"
    ))
    .bind(job_id)
    .bind(generation)
    .fetch_optional(&mut *conn)
    .await?;

    Ok(match claimed {
        Some(taken_over) => {
            if taken_over {
                tracing::warn!(
                    job_id = %job_id,
                    generation,
                    "restarting a generation transition that was started but never finished"
                );
            }
            LeaveGenStep::Transition { generation }
        }
        // Someone else owns it. `completed_at` set with no artifact row is
        // not a state the transition writes -- both happen in one
        // transaction -- so this is always a transition still in progress.
        None => LeaveGenStep::TransitionInProgress { generation },
    })
}

/// The next `racks_per_task` racks below target after `after`, in primary-key
/// order, and whether the lap has any left beyond them. `''` sorts before every
/// rack, so it stands for the start of a lap -- and keeps the comparison a bare
/// index condition, which `$3 IS NULL OR ...` would not be.
///
/// One rack more than a task holds is read, to learn whether this is the lap's
/// last task while there is still a task to commit that knowledge with. Found
/// out by the *next* claim instead, it would be found out by reading from the
/// cursor to the end of the universe and coming back empty -- in a claim that
/// hands out nothing and so rolls back, and so by every claim after it, for as
/// long as the lap's last results take to arrive.
async fn racks_after(
    conn: &mut PgConnection,
    job_id: Uuid,
    generation: i32,
    config: &LeaveConfig,
    after: &str,
) -> AppResult<(Vec<String>, bool)> {
    let mut racks = sqlx::query_scalar::<_, String>(
        "SELECT p.rack FROM leave_rack_progress p
         WHERE p.job_id = $1 AND p.generation = $2 AND p.rack > $3
           AND p.occurrence_count < $4
         ORDER BY p.rack ASC
         LIMIT $5",
    )
    .bind(job_id)
    .bind(generation)
    .bind(after)
    .bind(config.target_rack_count as i64)
    .bind(config.racks_per_task as i64 + 1)
    .fetch_all(&mut *conn)
    .await?;
    let more = racks.len() > config.racks_per_task as usize;
    racks.truncate(config.racks_per_task as usize);
    Ok((racks, more))
}

/// Selection by sweep: the generation's racks in primary-key order, from where
/// the last claim left off.
///
/// A **lap** is one pass over the universe, handing out every rack that is
/// below target as the pass reaches it. `leave_selection_cursors` holds the
/// last rack handed out; its row exists exactly while a lap has racks left.
///
/// What makes a sweep need no exclusion list is the rule for *starting* a lap:
/// only with no claim of the generation in flight and nothing staged. From
/// there, every rack that is out -- forced by an open claim, or by a result not
/// yet merged -- was handed out during this lap, and so lies behind the cursor;
/// nothing ahead of it is out. (A task whose claim lapsed is reissued as it
/// stands, before anything new is selected -- see `registry::generate_leave_gen`
/// -- so its racks stay behind the cursor with it.) When a lap runs off the end
/// of the universe the job therefore pauses until the lap's last results are
/// in and merged, and the next lap selects on exact counts. That pause is one
/// task's duration and one merge per lap -- some 6,400 tasks for English -- and
/// it is the same wait that already precedes closing a generation, which is
/// simply a lap that finds nothing below target.
///
/// A cursor row lost to a partial restore, or deleted by a purge, is a lap not
/// started: the same rule applies, and nothing is handed out twice.
async fn sweep(
    conn: &mut PgConnection,
    job_id: Uuid,
    generation: i32,
    config: &LeaveConfig,
) -> AppResult<Selected> {
    let cursor: Option<String> = sqlx::query_scalar(
        "SELECT cursor_rack FROM leave_selection_cursors WHERE job_id = $1 AND generation = $2",
    )
    .bind(job_id)
    .bind(generation)
    .fetch_optional(&mut *conn)
    .await?;

    if let Some(cursor) = cursor {
        let (racks, more) = racks_after(conn, job_id, generation, config, &cursor).await?;
        if more {
            sqlx::query(
                "UPDATE leave_selection_cursors SET cursor_rack = $3
                 WHERE job_id = $1 AND generation = $2",
            )
            .bind(job_id)
            .bind(generation)
            .bind(racks.last())
            .execute(&mut *conn)
            .await?;
            return Ok(Selected::Racks(racks));
        }
        // The lap ends here: with this task if it found any racks, already if
        // it did not (a merge since the last claim can put the racks that were
        // left at target).
        sqlx::query("DELETE FROM leave_selection_cursors WHERE job_id = $1 AND generation = $2")
            .bind(job_id)
            .bind(generation)
            .execute(&mut *conn)
            .await?;
        if !racks.is_empty() {
            return Ok(Selected::Racks(racks));
        }
    }

    // Starting a lap, which is also how a generation is found complete.
    if let Some(step) = nothing_to_hand_out(conn, job_id, generation).await? {
        return Ok(Selected::Step(step));
    }
    let (racks, more) = racks_after(conn, job_id, generation, config, "").await?;
    if racks.is_empty() {
        return Ok(Selected::Step(claim_transition(conn, job_id, generation).await?));
    }
    // A lap of a single task has no cursor: it is over as it begins.
    if more {
        sqlx::query(
            "INSERT INTO leave_selection_cursors (job_id, generation, cursor_rack)
             VALUES ($1, $2, $3)
             ON CONFLICT (job_id, generation) DO UPDATE SET cursor_rack = EXCLUDED.cursor_rack",
        )
        .bind(job_id)
        .bind(generation)
        .bind(racks.last())
        .execute(&mut *conn)
        .await?;
    }
    Ok(Selected::Racks(racks))
}

/// Selection once few racks remain below target: the racks furthest from it
/// that nothing is already playing.
async fn furthest_below_target(
    conn: &mut PgConnection,
    job_id: Uuid,
    generation: i32,
    config: &LeaveConfig,
) -> AppResult<Selected> {
    // Racks named by an open claim are skipped: two concurrent claims would
    // otherwise both be handed the same lowest-count racks. The anti-join is
    // over this job's open claims only, a few hundred racks each.
    //
    // So are the racks forced by a task whose result is **staged but not yet
    // merged**. Their counts in `leave_rack_progress` are as they were before
    // that task played, so ordered on those counts they are still the lowest
    // in the generation the moment their claim completes -- and would be handed
    // straight out again, to every claim until the next merge, while racks
    // nobody has forced yet wait. Held out until the merge says what they
    // actually reached. A staged row's arrays are stored out of line, so
    // reading its `task_id` does not read them.
    //
    // **The shape of this statement is what keeps it off the whole
    // generation.** It orders on `occurrence_count` *alone*, which
    // `leave_rack_progress_pick_idx` supplies, so the scan starts at the lowest
    // count and stops once it has `racks_per_task` rows that pass the filter;
    // racks with equal counts come in whatever order the index holds them. It
    // used to break ties by rack, and counts tie in their millions -- every
    // rack starts at zero and the rare ones stay there -- so the order came
    // from sorting every row below target: 4.9 s a claim at full size. Carrying
    // `rack` in the index fixed that and cost 160 MB a generation, because
    // unique keys cannot be deduplicated; nothing needs the tie broken. And
    // the exclusion is `NOT IN` over an uncorrelated subquery, which Postgres
    // evaluates as a *hashed subplan*: one pass to build it, one probe per
    // index entry. Written as `NOT EXISTS` it is an anti-join, which the
    // planner is free to run as a nested loop over the excluded racks -- and,
    // misled by its fixed guess of ten elements per `unnest`, did. `NOT IN` is
    // only safe with no NULL on its right-hand side, which the subquery
    // guarantees for itself.
    let racks = sqlx::query_scalar::<_, String>(
        "WITH out_now AS (
             SELECT unnest(r.forced_racks) AS rack
             FROM task_claims c
             JOIN tasks t ON t.id = c.task_id
             JOIN leave_requests r ON r.task_id = c.task_id
             WHERE t.job_id = $1 AND r.generation = $2 AND c.state = 'claimed'
             UNION
             SELECT unnest(r.forced_racks)
             FROM leave_rack_staging s
             JOIN leave_requests r ON r.task_id = s.task_id
             WHERE s.job_id = $1 AND s.generation = $2
         )
         SELECT p.rack FROM leave_rack_progress p
         WHERE p.job_id = $1 AND p.generation = $2 AND p.occurrence_count < $3
           AND p.rack NOT IN (SELECT o.rack FROM out_now o WHERE o.rack IS NOT NULL)
         ORDER BY p.occurrence_count ASC
         LIMIT $4",
    )
    .bind(job_id)
    .bind(generation)
    .bind(config.target_rack_count as i64)
    .bind(config.racks_per_task as i64)
    .fetch_all(&mut *conn)
    .await?;
    if !racks.is_empty() {
        return Ok(Selected::Racks(racks));
    }

    // With nothing in flight the only racks held out of the selection above
    // are those of staged tasks, so an empty selection means "no rack is below
    // target" only once nothing is staged.
    Ok(Selected::Step(match nothing_to_hand_out(conn, job_id, generation).await? {
        Some(step) => step,
        None => claim_transition(conn, job_id, generation).await?,
    }))
}

/// Write a generation's rack universe at zero occurrences: every full rack the
/// distribution can draw (3,199,724 for English). "Racks with no row yet count
/// as 0" needs a known universe to draw from, and materializing it is what lets
/// claim-time selection be a single indexed `ORDER BY occurrence_count` query.
///
/// Every generation's, the first included, comes from [`ensure_universe`], on a
/// task the first claim to find it missing starts. There is one implementation
/// of what a generation's universe *is*, derived from the pinned letter
/// distribution rather than from the previous generation's rows.
pub async fn seed_generation(
    conn: &mut PgConnection,
    job_id: Uuid,
    generation: i32,
    distribution: &LetterDistribution,
) -> AppResult<i64> {
    let index = std::sync::Arc::new(RackIndex::new(distribution, RACK_SIZE)?);
    let total = index.total();

    // Idempotent: a universe already seeded (a seeding started twice) is left
    // as it is.
    let seeded: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM leave_rack_progress WHERE job_id = $1 AND generation = $2)",
    )
    .bind(job_id)
    .bind(generation)
    .fetch_one(&mut *conn)
    .await?;
    if seeded {
        return Ok(total as i64);
    }
    tracing::info!(job_id = %job_id, generation, racks = total, "seeding full-rack universe");

    // COPY rather than INSERT: millions of rows, and the job's claims wait for
    // the seeding's lock while it runs. Racks are unranked in chunks so they
    // are never all in memory together. Rack
    // strings are letters and `?`, which need no escaping in COPY's text
    // format.
    const CHUNK: u64 = 50_000;
    let mut copy = conn
        .copy_in_raw("COPY leave_rack_progress (job_id, generation, rack) FROM STDIN")
        .await?;
    let mut start = 0;
    while start < total {
        // Built on the blocking pool: unranking and formatting fifty thousand
        // racks is a burst of pure computation, sixty-four times over, and an
        // async worker that does not yield can hold up every other request
        // (see `exports::upload_rows`). Measured while a universe was seeded,
        // `/health` went from 2 ms to as much as 1.3 s in a debug build.
        let rows = {
            let index = index.clone();
            tokio::task::spawn_blocking(move || {
                let mut rows = String::with_capacity(CHUNK as usize * 48);
                for rack in index.racks_in_enumeration_range(start, CHUNK) {
                    rows.push_str(&format!("{job_id}\t{generation}\t{rack}\n"));
                }
                rows.into_bytes()
            })
            .await
            .map_err(|e| AppError::internal(format!("enumerating a rack universe failed: {e}")))?
        };
        copy.send(rows).await?;
        start += CHUNK;
    }
    copy.finish().await?;

    // The generation's summary starts from what was just written, so the
    // dashboard has a denominator before the first merge.
    sqlx::query(
        "INSERT INTO leave_generation_progress (job_id, generation, racks_total, merged_at)
         VALUES ($1, $2, $3, now())
         ON CONFLICT (job_id, generation) DO UPDATE
             SET racks_total = EXCLUDED.racks_total, merged_at = EXCLUDED.merged_at",
    )
    .bind(job_id)
    .bind(generation)
    .bind(total as i64)
    .execute(&mut *conn)
    .await?;
    Ok(total as i64)
}

/// Whether `generation`'s rack universe has been written. One index probe.
pub async fn universe_exists(
    conn: &mut PgConnection,
    job_id: Uuid,
    generation: i32,
) -> AppResult<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM leave_rack_progress WHERE job_id = $1 AND generation = $2)",
    )
    .bind(job_id)
    .bind(generation)
    .fetch_one(&mut *conn)
    .await?)
}

/// Make sure `generation`'s rack universe exists, seeding it if it does not.
///
/// Every generation's, the first included, is written here, the first time a
/// claim asks for work in that generation -- not at job creation or purge, and
/// not by the transition that closed the generation before it. That keeps the
/// millions of rows off the transition's critical path -- a claim that arrives
/// to find the universe missing pays for it once, while a transition that
/// wrote it made every worker on the job wait, every time.
///
/// Idempotent and cheap when there is nothing to do: [`seed_generation`]
/// returns on an `EXISTS` check, which is one index probe.
pub async fn ensure_universe(
    conn: &mut PgConnection,
    job_id: Uuid,
    generation: i32,
    distribution: &LetterDistribution,
) -> AppResult<()> {
    seed_generation(conn, job_id, generation, distribution).await?;
    Ok(())
}

/// Close out a generation: derive leave values from `leave_rack_progress`'s
/// full-rack means as MAGPIE does (see `klv::FullRackLeaves`), build the
/// generation's KLV, and store the artifact. The next generation's rack
/// universe is seeded when a claim first asks for work in it.
pub async fn run_transition(
    state: &crate::state::AppState,
    job_id: Uuid,
    generation: i32,
    config: &LeaveConfig,
    distribution: &LetterDistribution,
) -> AppResult<String> {
    let (pool, artifacts, magpie, builders) =
        (&state.pool, &state.artifacts, &state.magpie, &state.builders);
    // Drain first, and wait for any merge already running: the KLV is built
    // from `leave_rack_progress`, and a generation's totals are only complete
    // once every staged submission is in them. Nothing new can be staged for
    // this generation meanwhile -- it closes only with no claim in flight, and
    // nothing is dispatched for it while its transition runs -- so what this
    // merge leaves behind is nothing. The claim path already refuses to start
    // a transition with anything staged (`LeaveGenStep::NeedsMerge`); this is
    // the same rule held where the totals are read, for a takeover and for
    // any path that reaches here another way.
    merge_staged(pool, job_id, generation, true).await?;

    // Shared with `rebuild_artifacts` rather than written twice: a rebuild is
    // only meaningful if it folds the rows exactly as the original write did,
    // and two copies of this would be free to drift into producing different
    // bytes for the same generation.
    let klv = generation_klv(pool, magpie, job_id, generation, distribution).await?;

    // Hashed as written, not read back: the object store holds the only copy
    // of these bytes, and this is what a later rebuild is compared against.
    let sha256 = hex::encode(Sha256::digest(&klv));
    let key = artifact_key(job_id, generation);
    artifacts.put(&key, klv).await?;
    close_generation(pool, job_id, generation, &key, &sha256, &builders.klv(), config).await?;
    Ok(key)
}

/// The second half of [`run_transition`]: everything that has to happen in one
/// transaction once the KLV is in the object store.
///
/// Separate so it can be exercised without an object store, and because the
/// ownership check below is the only thing standing between a concurrent purge
/// and a job that believes a generation it no longer has results for is closed.
pub async fn close_generation(
    pool: &sqlx::PgPool,
    job_id: Uuid,
    generation: i32,
    key: &str,
    sha256: &str,
    builder: &str,
    config: &LeaveConfig,
) -> AppResult<()> {
    let mut tx = pool.begin().await?;
    // The merge lock first, as a purge and a delete take it: they hold it for
    // their whole transaction, so this waits holding nothing and then finds its
    // transition row gone. Without it the close locked that row and then
    // waited on the job's row (the artifact insert's foreign-key check) while
    // the purge, holding the job's row, waited on the transition row -- a
    // deadlock Postgres resolved by aborting the purge after it had run for
    // its whole length.
    lock_merges(&mut tx, job_id).await?;
    // Claiming ownership back, and the one place this transition can find out
    // it no longer has any. A purge deletes the transitions row along with the
    // artifacts and progress rows -- all while a transition spawned before it
    // may still be streaming. Writing the artifact anyway would hand the purged
    // job a generation-1 KLV derived from results it no longer has.
    // The row this request committed when it took the transition is the
    // evidence that the job is still the one it started on, so the close is
    // conditional on it.
    //
    // The uploaded object is left behind in that case: it is keyed by job and
    // generation, so a later transition of the same generation overwrites it,
    // and nothing reads a key no `leave_generation_artifacts` row names
    // (`/api/worker/artifact` checks).
    let still_ours = sqlx::query(
        "UPDATE leave_generation_transitions SET completed_at = now()
         WHERE job_id = $1 AND generation = $2 AND completed_at IS NULL",
    )
    .bind(job_id)
    .bind(generation)
    .execute(&mut *tx)
    .await?
    .rows_affected()
        > 0;
    if !still_ours {
        tx.rollback().await?;
        return Err(AppError::internal(format!(
            "leave job {job_id} generation {generation} was purged or closed by someone else \
             while its transition ran; the KLV built for it was discarded"
        )));
    }

    // In the same transaction as the close above: the pair is what "this
    // generation is closed" means, and a claim that saw one without the other
    // would either start a finished transition again or wait on a transition
    // that is over.
    //
    // DO NOTHING keeps the FIRST hash. A restore that replays this transition
    // against fewer results writes the same key with different bytes; keeping
    // the original hash is what makes that visible afterwards instead of
    // quietly agreeing with whatever landed last.
    sqlx::query(
        "INSERT INTO leave_generation_artifacts
             (job_id, generation, artifact_key, sha256, builder)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (job_id, generation) DO NOTHING",
    )
    .bind(job_id)
    .bind(generation)
    .bind(key)
    .bind(sha256)
    .bind(builder)
    .execute(&mut *tx)
    .await?;

    // A closed generation is never selected from again. (A sweep that ended
    // deletes its own cursor; one overtaken by the switch to lowest-count-first
    // selection leaves it behind.)
    sqlx::query("DELETE FROM leave_selection_cursors WHERE job_id = $1 AND generation = $2")
        .bind(job_id)
        .bind(generation)
        .execute(&mut *tx)
        .await?;

    // Guarded on `active`, as every other automatic completion is: an admin
    // who deactivated the job during its last transition decided something,
    // and it stands. Reactivated, its first claim finds the last generation
    // closed and completes it then.
    if generation >= config.generation_count {
        sqlx::query("UPDATE jobs SET status = 'completed' WHERE id = $1 AND status = 'active'")
            .bind(job_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;

    // The next generation's universe is NOT written here. It is seeded when
    // that generation opens -- see `ensure_universe`, called from the claim
    // path -- for two reasons. It is millions of rows (3.2 million for
    // English), which inside this transaction made closing a generation a
    // minute-long write that every worker on the job waited out, and which the
    // transition then had to redo in full if anything failed, because the close
    // and the copy stood or fell together. Seeded at the other end it happens
    // while workers are busy, and a failure costs a retry of the seeding alone.
    Ok(())
}

/// The lexicon name a leave job's bot plays with, from the row it pins.
pub async fn lexicon_name(conn: &mut PgConnection, kwg_id: Uuid) -> AppResult<String> {
    Ok(
        sqlx::query_scalar::<_, String>("SELECT name FROM input_data WHERE id = $1")
            .bind(kwg_id)
            .fetch_one(conn)
            .await?,
    )
}

/// The zeroed KLV generation 1 plays with, stored as generation 0's artifact.
///
/// Generation 1 has no predecessor to learn from, and it starts from leaves
/// worth exactly nothing rather than from whatever leaves a contributor's
/// lexicon happens to ship. Building it here rather than letting the client
/// zero its own means there is no first-generation branch on the client at
/// all: every generation fetches a KLV by key and plays.
///
/// Called from job initialization *outside* the creating transaction -- it
/// builds a multi-megabyte artifact and writes it to the object store.
pub async fn seed_zero_generation(
    pool: &sqlx::PgPool,
    artifacts: &ArtifactStore,
    magpie: &Magpie,
    builders: &Builders,
    job_id: Uuid,
    distribution: &LetterDistribution,
) -> AppResult<String> {
    let klv = zero_klv(magpie, distribution).await?;
    let sha256 = hex::encode(Sha256::digest(&klv));
    let key = artifact_key(job_id, 0);
    artifacts.put(&key, klv).await?;

    sqlx::query(
        "INSERT INTO leave_generation_artifacts
             (job_id, generation, artifact_key, sha256, builder)
         VALUES ($1, 0, $2, $3, $4)
         ON CONFLICT (job_id, generation) DO NOTHING",
    )
    .bind(job_id)
    .bind(&key)
    .bind(&sha256)
    .bind(builders.klv())
    .execute(pool)
    .await?;

    Ok(key)
}

/// What rebuilding one generation's KLV from the database found.
#[derive(Debug, serde::Serialize)]
pub struct ArtifactRebuild {
    pub generation: i32,
    pub artifact_key: String,
    pub stored_sha256: String,
    pub rebuilt_sha256: String,
    pub matches: bool,
    /// Whether the stored artifact was written by the builder this rebuild
    /// used. When it was not, `matches` says nothing: two builders producing
    /// different bytes for the same values is the expected outcome, not a
    /// fault, and the admin view reads this first.
    pub same_builder: bool,
    pub stored_builder: String,
    pub rebuilt_builder: String,
    pub object_present: bool,
    pub rewritten: bool,
}

/// Recompute every generation's KLV from the database and report what it found.
///
/// The KLVs are the only application state outside Postgres, and they are pure
/// functions of state that is still in it: `leave_rack_progress` rows are never
/// deleted per generation, so every generation's inputs remain available for the
/// life of the job. That is what makes rebuilding an alternative to backing them
/// up -- see PLAN.md, "Artifacts: back up, or rebuild?".
///
/// Three things this deliberately does *not* do:
///
/// - **It does not overwrite an object that is present but differs.** A hash
///   mismatch means the stored bytes are not what this code would produce now,
///   and that is evidence to look at rather than a fault to paper over: it is
///   equally consistent with a corrupted object and with a legitimate change to
///   MAGPIE's KLV builder. Rewriting on sight would destroy the only copy of
///   whichever one it was. `force` is the deliberate override.
/// - **It does not treat a different builder as a mismatch.** Until MAGPIE
///   built these, there was one implementation and differing bytes could only
///   mean corruption. Now a MAGPIE upgrade can legitimately change them, so an
///   artifact written by a different builder is reported as exactly that, and
///   `matches` is not the question being asked of it. Without this the first
///   upgrade after a restore drill would read as data loss.
/// - **It does not rebuild generation 0 from `leave_rack_progress`.** Generation
///   0 is the zeroed KLV every job starts from, not a fold of any results, and
///   there are no progress rows behind it. It is rebuilt the way
///   `seed_zero_generation` built it.
pub async fn rebuild_artifacts(
    pool: &sqlx::PgPool,
    artifacts: &ArtifactStore,
    magpie: &Magpie,
    builders: &Builders,
    job_id: Uuid,
    distribution: &LetterDistribution,
    force: bool,
) -> AppResult<Vec<ArtifactRebuild>> {
    let rows = sqlx::query(
        "SELECT generation, artifact_key, sha256, builder
         FROM leave_generation_artifacts
         WHERE job_id = $1
         ORDER BY generation",
    )
    .bind(job_id)
    .fetch_all(pool)
    .await?;

    let rebuilding_with = builders.klv();
    let mut report = Vec::with_capacity(rows.len());
    for row in rows {
        let generation: i32 = row.get("generation");
        let artifact_key: String = row.get("artifact_key");
        let stored_sha256: String = row.get("sha256");
        let stored_builder: String = row.get("builder");

        let klv = if generation == 0 {
            zero_klv(magpie, distribution).await?
        } else {
            generation_klv(pool, magpie, job_id, generation, distribution).await?
        };
        let rebuilt_sha256 = hex::encode(Sha256::digest(&klv));

        let object_present = artifacts.exists(&artifact_key).await?;
        let same_builder = stored_builder == rebuilding_with;
        let matches = rebuilt_sha256 == stored_sha256;
        // A missing object has no bytes to lose, so restoring it needs no
        // permission. Replacing one that is present does -- and replacing one
        // built by a different builder needs it twice over, since the
        // difference is expected rather than evidence of anything.
        let rewritten = !object_present || force;
        if rewritten {
            artifacts.put(&artifact_key, klv).await?;
            // What workers are sent to check the object against has to be
            // what the object now holds; `sha256` keeps the first hash.
            sqlx::query(
                "UPDATE leave_generation_artifacts
                 SET served_sha256 = NULLIF($3, sha256)
                 WHERE job_id = $1 AND generation = $2",
            )
            .bind(job_id)
            .bind(generation)
            .bind(&rebuilt_sha256)
            .execute(pool)
            .await?;
        }

        report.push(ArtifactRebuild {
            generation,
            artifact_key,
            stored_sha256,
            rebuilt_sha256,
            matches,
            same_builder,
            stored_builder,
            rebuilt_builder: rebuilding_with.clone(),
            object_present,
            rewritten,
        });
    }
    Ok(report)
}

/// One generation's KLV from its full-rack results, built by MAGPIE.
///
/// The server used to derive this itself, with a Rust translation of MAGPIE's
/// `rack_list_write_to_klv` and `generate_leaves`. That translation had to
/// change whenever MAGPIE's did and nothing made it, and its KLVs differed in
/// bytes from MAGPIE's for the same values -- a plain trie where MAGPIE builds
/// a minimized DAWG -- which was a standing source of confusion. `convert
/// rackequity2klv` is the same derivation, run by the code that defines it.
///
/// The transfer is a CSV in a scratch directory: one `rack,count,equity_sum`
/// row per full rack, roughly 3.2 million of them for English. Rows are
/// streamed from Postgres and written as they arrive, so neither side holds
/// the generation in memory.
///
/// The job's pinned letter distribution is written into the same directory
/// from `input_data.content`, so MAGPIE reads exactly the row the job pins
/// rather than anything on the server's disk -- the rule PLAN.md sets, and the
/// reason the backend image ships no `data/`.
async fn generation_klv(
    pool: &sqlx::PgPool,
    magpie: &Magpie,
    job_id: Uuid,
    generation: i32,
    distribution: &LetterDistribution,
) -> AppResult<Vec<u8>> {
    use futures::TryStreamExt;
    use tokio::io::AsyncWriteExt;

    let scratch = ScratchData::empty().await?;
    scratch
        .write(
            "letterdistributions",
            &distribution.name,
            ".csv",
            &distribution.bytes,
        )
        .await?;

    let csv_path = scratch.lexicon_path(SCRATCH_KLV_NAME, ".csv");
    let mut csv = tokio::io::BufWriter::new(
        tokio::fs::File::create(&csv_path)
            .await
            .map_err(|e| AppError::internal(format!("could not write the rack equity csv: {e}")))?,
    );

    let mut rows = sqlx::query(
        "SELECT rack, occurrence_count, equity_sum
         FROM leave_rack_progress
         WHERE job_id = $1 AND generation = $2
         ORDER BY rack",
    )
    .bind(job_id)
    .bind(generation)
    .fetch(pool);

    let mut written: u64 = 0;
    while let Some(row) = rows.try_next().await? {
        let rack: String = row.get("rack");
        let count: i64 = row.get("occurrence_count");
        let equity_sum: f64 = row.get("equity_sum");
        // The sum, not the mean: MAGPIE divides, and handing it the number it
        // would compute anyway keeps one rounding step out of the transfer.
        // A rack that never occurred carries a sum of zero and still counts
        // toward the weighted average, as it does inside a leavegen run.
        csv.write_all(format!("{rack},{count},{equity_sum:.10}\n").as_bytes())
            .await
            .map_err(|e| AppError::internal(format!("could not write the rack equity csv: {e}")))?;
        written += 1;
    }
    drop(rows);
    csv.flush()
        .await
        .map_err(|e| AppError::internal(format!("could not write the rack equity csv: {e}")))?;
    drop(csv);

    // MAGPIE refuses a file that does not cover every full rack exactly once,
    // so this is a second check rather than the only one -- but it fails with
    // the job and generation in the message, where MAGPIE's failure would only
    // name a path inside a directory that no longer exists.
    let expected = RackIndex::new(distribution, RACK_SIZE)?.total();
    if written != expected {
        return Err(AppError::internal(format!(
            "leave job {job_id} generation {generation} has {written} progress rows, but the \
             distribution draws {expected} full racks"
        )));
    }

    magpie
        .convert(&scratch, "rackequity2klv", SCRATCH_KLV_NAME, &distribution.name)
        .await?;
    read_built_klv(&scratch, SCRATCH_KLV_NAME).await
}

/// The zeroed KLV a leave-generation job's first generation plays with.
///
/// `createdata klv` builds it from the letter distribution alone, with every
/// leave worth zero. MAGPIE_DEPENDENCY.md proposed a `convert zero2klv` for
/// this; `createdata klv` already is it, through the same `klv_create_empty`,
/// so there is one spelling rather than two to keep in step.
async fn zero_klv(magpie: &Magpie, distribution: &LetterDistribution) -> AppResult<Vec<u8>> {
    let scratch = ScratchData::empty().await?;
    scratch
        .write(
            "letterdistributions",
            &distribution.name,
            ".csv",
            &distribution.bytes,
        )
        .await?;
    magpie
        .create_zero_klv(&scratch, SCRATCH_KLV_NAME, &distribution.name)
        .await?;
    read_built_klv(&scratch, SCRATCH_KLV_NAME).await
}

/// Reads back what MAGPIE wrote, before the scratch directory is dropped.
///
/// MAGPIE reports a failed conversion on its error stack and can still leave
/// no file behind, so the output's existence is the real check -- the same
/// rule the derived-file builder and the worker both apply.
async fn read_built_klv(scratch: &ScratchData, name: &str) -> AppResult<Vec<u8>> {
    let path = scratch.lexicon_path(name, ".klv2");
    tokio::fs::read(&path).await.map_err(|e| {
        AppError::internal(format!("MAGPIE reported no error but wrote no KLV: {e}"))
    })
}

/// Where a generation's KLV lives in the object store: under its job, named
/// for its generation, so no two jobs and no two generations of one job can
/// write the same object. Every artifact key is minted here -- the worker
/// artifact route serves only keys the server recorded, and this is the only
/// shape those take.
pub fn artifact_key(job_id: Uuid, generation: i32) -> String {
    format!("leaves/{job_id}/generation-{generation}.klv2")
}
