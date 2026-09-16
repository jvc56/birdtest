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

    async fn load_request(conn: &mut PgConnection, task_id: Uuid) -> AppResult<Self::Request> {
        let row = sqlx::query(
            "SELECT lexicon, variant, letter_distribution, board_layout, generation, seed,
                    forced_racks, num_games, previous_artifact_key, use_wordmap
             FROM leave_requests WHERE task_id = $1",
        )
        .bind(task_id)
        .fetch_one(&mut *conn)
        .await?;
        let job_data = super::load_job_data_for_task(conn, task_id).await?;
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
            use_wordmap: row.get("use_wordmap"),
            bingo_bonus: job_data.bingo_bonus,
        })
    }

    fn process_response(response: Self::Response) -> AppResult<Self::Record> {
        if response.racks.is_empty() {
            return Err(AppError::bad_request("leave result carried no rack occurrences"));
        }
        super::plausibility::check_rack_occurrences(&response.racks)?;
        Ok(LeaveRecord { racks: response.racks })
    }

    /// Credits the claim and folds its occurrences into the generation. Only
    /// the first accepted result for a task is folded -- see
    /// [`credit_claim`] for the others.
    async fn insert_record(
        conn: &mut PgConnection,
        job_id: Uuid,
        task_id: Uuid,
        claim_id: Uuid,
        record: &Self::Record,
    ) -> AppResult<()> {
        credit_claim(conn, task_id, claim_id, record).await?;
        fold_into_generation(conn, job_id, task_id, record).await
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

async fn fold_into_generation(
    conn: &mut PgConnection,
    job_id: Uuid,
    task_id: Uuid,
    record: &LeaveRecord,
) -> AppResult<()> {
    let generation: i32 =
        sqlx::query_scalar("SELECT generation FROM leave_requests WHERE task_id = $1")
            .bind(task_id)
            .fetch_one(&mut *conn)
            .await?;

    // A result for a generation that has already been aggregated is credited
    // to the worker -- it did the work, and the claim completes normally --
    // but must not be folded in. The generation's KLV is already built and
    // uploaded, so nothing will ever read these occurrences; adding them would
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

    // Lock the rows first, in rack order. Submissions for different tasks of
    // one generation do not serialize on anything else -- each holds only its
    // own claim and task -- and they overlap heavily: every game draws common
    // racks, whatever the task forced. The UPDATE below locks rows in whatever
    // order its plan visits them, so two of them could each hold a rack the
    // other was waiting for, and Postgres broke the deadlock by failing one
    // submission with a 500 after `deadlock_timeout`. Taken in one order,
    // the second submission waits for the first instead, holding nothing.
    sqlx::query(
        "SELECT 1 FROM leave_rack_progress
         WHERE job_id = $1 AND generation = $2 AND rack = ANY($3::text[])
         ORDER BY rack
         FOR UPDATE",
    )
    .bind(job_id)
    .bind(generation)
    .bind(&racks)
    .execute(&mut *conn)
    .await?;

    // One statement per submission, however many racks it carries. An UPDATE
    // rather than an upsert: the generation's universe is every full rack,
    // seeded up front, so a rack with no row is not a rack of this
    // distribution and must not create one.
    sqlx::query(
        "UPDATE leave_rack_progress p SET
             occurrence_count = p.occurrence_count + u.count,
             equity_sum       = p.equity_sum + u.equity_sum,
             updated_at       = now()
         FROM UNNEST($3::text[], $4::bigint[], $5::float8[]) AS u(rack, count, equity_sum)
         WHERE p.job_id = $1 AND p.generation = $2 AND p.rack = u.rack",
    )
    .bind(job_id)
    .bind(generation)
    .bind(&racks)
    .bind(&counts)
    .bind(&sums)
    .execute(&mut *conn)
    .await?;
    Ok(())
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

/// Claim-time rack selection: the racks furthest from this generation's target
/// that no open claim is already playing.
pub async fn next_step(
    conn: &mut PgConnection,
    job_id: Uuid,
    config: &LeaveConfig,
    job_data: &JobData,
) -> AppResult<LeaveGenStep> {
    let Some(generation) = current_generation(&mut *conn, job_id, config).await? else {
        return Ok(LeaveGenStep::Finished);
    };

    // Racks named by an open claim are skipped: two concurrent claims would
    // otherwise both be handed the same lowest-count racks. The anti-join is
    // over this job's open claims only, a few hundred racks each.
    let racks = sqlx::query_scalar::<_, String>(
        "WITH out_now AS (
             SELECT DISTINCT unnest(r.forced_racks) AS rack
             FROM task_claims c
             JOIN tasks t ON t.id = c.task_id
             JOIN leave_requests r ON r.task_id = c.task_id
             WHERE t.job_id = $1 AND r.generation = $2 AND c.state = 'claimed'
         )
         SELECT p.rack FROM leave_rack_progress p
         WHERE p.job_id = $1 AND p.generation = $2 AND p.occurrence_count < $3
           AND NOT EXISTS (SELECT 1 FROM out_now o WHERE o.rack = p.rack)
         ORDER BY p.occurrence_count ASC, p.rack ASC
         LIMIT $4",
    )
    .bind(job_id)
    .bind(generation)
    .bind(config.target_rack_count as i64)
    .bind(config.racks_per_task as i64)
    .fetch_all(&mut *conn)
    .await?;

    if racks.is_empty() {
        let in_flight = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*)
             FROM task_claims c
             JOIN leave_requests r ON r.task_id = c.task_id
             JOIN tasks t ON t.id = c.task_id
             WHERE t.job_id = $1 AND r.generation = $2 AND c.state = 'claimed'",
        )
        .bind(job_id)
        .bind(generation)
        .fetch_one(&mut *conn)
        .await?;

        if in_flight > 0 {
            return Ok(LeaveGenStep::NoWorkYet);
        }

        // The generation is complete. Whoever writes this row owns its
        // transition; everyone else waits. Safe to test and write without
        // re-reading because `lock_claim_decisions` holds the job's lock for
        // the rest of this transaction, so no other claim is between its own
        // test and its own write.
        //
        // A row whose transition never finished is taken over rather than
        // trusted forever -- see TRANSITION_TAKEOVER_AFTER. Taking over bumps
        // `attempts`, which is the only place a crash mid-transition is
        // recorded.
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

        return Ok(match claimed {
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
        });
    }

    // Never optional: generation 1 reads the zeroed KLV written at generation
    // 0 when the job was created, so every generation fetches its leaves the
    // same way.
    let previous_artifact_key = sqlx::query_scalar::<_, String>(
        "SELECT artifact_key FROM leave_generation_artifacts
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
        lexicon: lexicon_name(&mut *conn, config.kwg_id).await?,
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
        num_games: config.num_iterations,
        use_wordmap: config.use_wordmap,
        bingo_bonus: job_data.bingo_bonus,
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
    let index = RackIndex::new(distribution, RACK_SIZE);
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
        let mut rows = String::with_capacity(CHUNK as usize * 48);
        for rack in index.racks_in_enumeration_range(start, CHUNK) {
            rows.push_str(&format!("{job_id}\t{generation}\t{rack}\n"));
        }
        copy.send(rows.into_bytes()).await?;
        start += CHUNK;
    }
    copy.finish().await?;
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
    // Shared with `rebuild_artifacts` rather than written twice: a rebuild is
    // only meaningful if it folds the rows exactly as the original write did,
    // and two copies of this would be free to drift into producing different
    // bytes for the same generation.
    let klv = generation_klv(pool, magpie, job_id, generation, distribution).await?;

    // Hashed as written, not read back: the object store holds the only copy
    // of these bytes, and this is what a later rebuild is compared against.
    let sha256 = hex::encode(Sha256::digest(&klv));
    let key = format!("leaves/{job_id}/generation-{generation}.klv2");
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

    if generation >= config.generation_count {
        sqlx::query("UPDATE jobs SET status = 'completed' WHERE id = $1")
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
    let key = format!("leaves/{job_id}/generation-0.klv2");
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
    let expected = RackIndex::new(distribution, RACK_SIZE).total();
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
