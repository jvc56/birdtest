//! Building a job's results into one downloadable artifact.
//!
//! The live results stream (`/api/admin/jobs/:id/results/stream`) scans the
//! job's result tables from a cursor and holds a pool connection for as long as
//! the caller keeps reading. That is fine for a spot check and wrong for a
//! corpus: a full English opening-rack job is tens of millions of rows, and one
//! caller per scan is one connection per scan against a pool of twenty.
//!
//! An export is that read done **once**, at any point in a job's life. What it
//! is depends on when it was read, and the row says which:
//!
//! - **Final** (`is_final`): read from a job that was completed, with no claim
//!   still open and nothing staged, at the moment its snapshot was taken. A
//!   completed job's results are immutable, so this artifact is built once and
//!   reused by every later download -- the results stream redirects to it.
//! - **Snapshot**: anything else -- a job still running, or one that completed
//!   while results were still landing. It is the corpus as of `snapshot_at`,
//!   consistent in itself, and never served as the finished job's corpus: that
//!   was a real bug, when "the newest ready export" was whatever had been
//!   built, and an export taken mid-run became the completed job's download.
//!
//! The marker is decided **inside** the snapshot, by the snapshot's first
//! statement, not when the export is requested: a job can complete between the
//! two, and an export marked final on the strength of a status read before its
//! rows were would be exactly the stale corpus the marker exists to rule out.
//!
//! Shaped like [`crate::inputdata`]'s import: an admin starts it, a spawned
//! task does the work, the admin polls. birdtest runs as a single instance, so
//! the task needs no lease and startup may fail any row left `running`.

use crate::artifacts::MultipartUpload;
use crate::error::{AppError, AppResult};
use crate::models::job::{Job, JobStatus, JobType};
use crate::state::AppState;
use flate2::write::GzEncoder;
use flate2::Compression;
use futures::TryStreamExt;
use sha2::{Digest, Sha256};
use sqlx::Row;
use std::io::Write;
use uuid::Uuid;

/// How long a download URL stays valid.
///
/// Long enough to start a multi-gigabyte download over a slow link, short
/// enough that a URL pasted somewhere it should not be does not stay useful.
/// The download itself may outlast it: S3 checks the signature when the request
/// is made, not while it is being served.
pub const DOWNLOAD_URL_TTL: std::time::Duration = std::time::Duration::from_secs(3600);

/// An opening-rack job's corpus: one row per analysed rack, **with its ranked
/// moves and their per-ply statistics nested inside it**.
///
/// The record row alone is a header -- the rack, how many moves were ranked,
/// when -- and the analysis itself is in `position_analysis_moves` and
/// `position_analysis_plies`. This used to export the header only, so the
/// artifact PLAN.md names as "the path for analysing the corpus properly" held
/// no move, score or equity at all, and nothing else reads the moves in bulk:
/// the public feed returns the best move only, and `?rack=` one rack at a time.
///
/// Nested rather than joined, so the unit stays one line per record and
/// `row_count` still counts records. A consensus job analyses a rack more than
/// once, so each of its lines also carries its rack's standing (`consensus`:
/// its analyses, its most common best move and how many ranked it first, and
/// whether it is settled, and without a consensus) from
/// `opening_rack_progress`, a primary-key probe per line; null for a job
/// wanting one analysis per rack (which keeps the rows too, see
/// `opening_rack::record_consensus`, but has no consensus to report), and for an in-game position. Each record's moves come through
/// `position_analysis_moves_record_idx (record_id, rank)` and each move's plies
/// through the `(move_id, ply)` unique index, so the cost is an index probe per
/// record and per simmed move, on a background task (or under the two-stream
/// cap) rather than on anything a worker waits for.
const OPENING_RACK_CORPUS: &str = "
    SELECT (to_jsonb(r) || jsonb_build_object('moves', COALESCE((
               SELECT jsonb_agg(
                          jsonb_build_object(
                              'rank', m.rank, 'move', m.move, 'score', m.score,
                              'equity', m.equity, 'iterations', m.iterations,
                              'win_percentage', m.win_percentage,
                              'blended_utility', m.blended_utility,
                              'mean_spread', m.mean_spread,
                              'fidelity_plies', m.fidelity_plies,
                              'plies', COALESCE((
                                  SELECT jsonb_agg(
                                             jsonb_build_object(
                                                 'ply', p.ply,
                                                 'bingo_percentage', p.bingo_percentage,
                                                 'average_score', p.average_score)
                                             ORDER BY p.ply)
                                  FROM position_analysis_plies p WHERE p.move_id = m.id
                              ), '[]'::jsonb))
                          ORDER BY m.rank)
               FROM position_analysis_moves m WHERE m.record_id = r.id
           ), '[]'::jsonb),
           'inference', (
               SELECT jsonb_build_object(
                          'num_leaves', i.num_leaves, 'total_draws', i.total_draws,
                          'average_equity', i.average_equity, 'leaves', i.leaves)
               FROM position_analysis_inference i WHERE i.record_id = r.id
           ),
           'consensus', (
               SELECT jsonb_build_object(
                          'results', c.results, 'top_move', c.top_move,
                          'top_count', c.top_count, 'settled', c.settled,
                          'without_consensus', c.without_consensus)
               FROM opening_rack_progress c
               JOIN job_opening_rack_config o ON o.job_id = c.job_id
               WHERE c.job_id = r.job_id AND c.rack = r.rack AND r.game_index IS NULL
                 AND o.max_results_per_rack > 1
           )))::text AS row
    FROM position_analysis_records r
    WHERE r.job_id = $1";

/// The positions a games or game-pairs job captured while playing
/// (`capture_positions`), in the same shape as an opening-rack line: the record
/// -- here with its CGP, game index and turn number -- and its ranked moves.
/// Every `position_analysis_records` row of such a job is a captured position,
/// so the corpus query serves unchanged.
///
/// A second artifact rather than tagged lines in the first: a games job's
/// export has always been its result rows, one shape per file, and a consumer
/// of that file should not start meeting lines of another kind. The corpus
/// capture exists to build had no way out of the database before this.
pub fn positions_query() -> &'static str {
    OPENING_RACK_CORPUS
}

/// Whether a job type can have captured positions beside its results.
pub fn may_capture_positions(job_type: JobType) -> bool {
    matches!(job_type, JobType::Games | JobType::GamePairs)
}

/// The rows an export contains, per job type. The live stream
/// (`routes::public::job_results_stream`) runs the same queries, so the two
/// produce the same corpus.
///
/// Every query returns its line **as text**, already serialized by Postgres.
/// As `jsonb` each row was parsed into a `serde_json::Value` and written out
/// again, per row, on the async executor: twice the work of the copy this is
/// now, on threads whose job is answering workers (see [`upload_rows`]).
pub fn export_query(job_type: JobType) -> &'static str {
    match job_type {
        JobType::OpeningRack => OPENING_RACK_CORPUS,
        JobType::Games | JobType::GamePairs => {
            "SELECT to_jsonb(r)::text AS row FROM game_results r WHERE r.job_id = $1"
        }
        JobType::LeaveGeneration => {
            "SELECT to_jsonb(r)::text AS row FROM leave_rack_progress r WHERE r.job_id = $1"
        }
    }
}

/// Fold in whatever a leave-generation job still has staged, so that the rows
/// an export or a stream is about to read are the job's whole corpus.
///
/// A leave job's corpus is `leave_rack_progress`, and an accepted result
/// reaches that table only at a merge (`leave_gen::merge_staged`). A job whose
/// last generation closed has nothing staged -- the transition drains first --
/// but a job an admin **force-completed** mid-generation does: its claims
/// already out are still played and still accepted, and their results sit in
/// `leave_rack_staging` until the half-hourly sweep. An export started in that
/// window read totals that were missing them, and every later download of the
/// completed job is redirected to that export. The rule is the one `start`
/// applies to claims still in flight: a completed job is exported once its
/// results have settled, and for a leave job settled includes merged.
///
/// Called for a completed job only: a running one's export and stream reflect
/// its last merge, as its page does. Waits for a merge already running.
/// Nothing for any other job type.
pub async fn settle(pool: &sqlx::PgPool, job: &Job) -> AppResult<()> {
    if job.job_type == JobType::LeaveGeneration {
        crate::jobs::leave_gen::merge_staged_for_job(pool, job.id, true).await?;
    }
    Ok(())
}

/// Start an export, returning its id. The work happens on a spawned task.
///
/// Any job can be exported, active, inactive or completed: one still taking
/// results is exported as a snapshot, labelled so (see the module docs), and
/// an active job always has claims in flight, so nothing waits on them.
///
/// A completed job still refuses while claims are in flight: the admin asking
/// for a completed job's export wants the final corpus, and what they would be
/// handed meanwhile is a snapshot that looks like it.
pub async fn start(state: &AppState, job: &Job, requested_by: Uuid) -> AppResult<Uuid> {
    if job.status == JobStatus::Completed {
        refuse_unsettled(state, job).await?;
    }

    // Under a share lock on the job's row, so a purge committing meanwhile --
    // it holds that row -- is waited for. Inserted beside one instead, the row
    // survived the purge's delete of the job's exports, its snapshot could
    // still read the job as completed and full, and the re-run's completed job
    // then redirected its downloads to that export. Waited for, the export
    // reads the purged job, which is inactive, and is a snapshot of it.
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT 1 FROM jobs WHERE id = $1 FOR SHARE")
        .bind(job.id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| AppError::not_found("no such job"))?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO job_exports (job_id, requested_by) VALUES ($1, $2) RETURNING id",
    )
    .bind(job.id)
    .bind(requested_by)
    .fetch_one(&mut *tx)
    .await
    .map_err(|err| {
        let err: AppError = err.into();
        if err.is_unique_violation() {
            AppError::conflict("an export of this job is already running")
        } else {
            err
        }
    })?;
    // In the transaction that starts it: an export refused (the job not
    // settled, one already running) writes no row, and one begun always has
    // one. Written by the handler before this, every refusal was logged as a
    // start (the adversarial check of the audit's pass 9).
    crate::audit::log_detail(
        &mut tx,
        "job.export_started",
        requested_by,
        "job",
        job.id.to_string(),
        Some(job.id),
        format!("export={id}"),
    )
    .await?;
    tx.commit().await?;

    let (state, job) = (state.clone(), job.clone());
    tokio::spawn(async move { run(state, job, id).await });
    Ok(id)
}

/// Refuses a completed job whose claims are still in flight.
async fn refuse_unsettled(state: &AppState, job: &Job) -> AppResult<()> {
    // Completed is not yet settled. A job flips to completed the moment its
    // stopping rule is met or an admin forces it, but every claim already out
    // is still played and still accepted -- the submit path checks the claim,
    // not the job's status. An export built in that window misses those
    // results; its snapshot sees the open claims and it is marked a snapshot,
    // never the final corpus the stream redirects to, but it is not what an
    // admin exporting a completed job asked for either. No claim can be issued
    // against a completed job, so once none is open the results really are
    // fixed.
    //
    // Reclamation is lazy: a lapsed claim is flipped to `abandoned` when a
    // worker next asks for work and the job is a candidate, and nothing ever
    // asks for work from a completed job. So a claim whose worker died stayed
    // `claimed` for good, and refused every export of the job for good --
    // where the design says a claim lapses at the heartbeat timeout. Reclaimed
    // here first, through the same statement dispatch uses, so "open" below
    // means live.
    crate::scheduler::reclaim_lapsed(state, &[job.id]).await?;
    let settling = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM task_claims c JOIN tasks t ON t.id = c.task_id
                        WHERE t.job_id = $1 AND c.state = 'claimed')",
    )
    .bind(job.id)
    .fetch_one(&state.pool)
    .await?;
    if settling {
        return Err(AppError::conflict(
            "this job completed with claims still in flight, and their results are still \
             arriving; export it once they have landed or lapsed, which is at most the \
             heartbeat timeout (counted from the server's last start, if that is more recent \
             than the claim's last heartbeat)",
        ));
    }

    Ok(())
}

/// How many exports build at once. Each holds a main-pool connection for the
/// whole of its corpus read -- minutes, for a full opening-rack job -- on the
/// pool claims and submissions share; beyond these, an export waits its turn
/// `running`.
static EXPORT_BUILDS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

/// Longer than any build of a real corpus takes. A row still `running` past
/// it is a build that hung (a stalled store call) rather than one in progress,
/// and with one export per job at a time it would otherwise refuse every
/// export of the job until a restart reaped it.
const EXPORT_BUILD_LIMIT: std::time::Duration = std::time::Duration::from_secs(6 * 3600);

async fn run(state: AppState, job: Job, export_id: Uuid) {
    let _turn = EXPORT_BUILDS.acquire().await;
    // Its turn may have come after a purge or a delete removed the row; then
    // there is nothing to build for, and building would leave objects nobody
    // names.
    let still_wanted = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM job_exports WHERE id = $1 AND state = 'running')",
    )
    .bind(export_id)
    .fetch_one(&state.pool)
    .await
    .unwrap_or(true);
    if !still_wanted {
        return;
    }
    // On a task of its own and under a time limit, so that a panic or a hang
    // still ends in the row being marked failed below rather than left
    // `running` -- which, one export per job at a time, refused every later
    // export of the job until a restart.
    let build_task = {
        let (state, job) = (state.clone(), job.clone());
        tokio::spawn(async move { build(&state, &job, export_id).await })
    };
    // Dropping a JoinHandle detaches the task rather than stopping it: a hung
    // build past its limit would go on holding its connection after its turn
    // was handed on.
    let abort = build_task.abort_handle();
    let outcome = tokio::time::timeout(EXPORT_BUILD_LIMIT, build_task).await;
    if outcome.is_err() {
        abort.abort();
    }
    let result = match outcome {
        Ok(Ok(result)) => result,
        // Logged, not stored: the row's error is shown on the admin page.
        Ok(Err(panicked)) => Err(AppError::task_failed("the export", panicked)),
        Err(_) => Err(AppError::internal("the export did not finish within its time limit")),
    };
    match result {
        Ok(()) => tracing::info!(job_id = %job.id, %export_id, "job export ready"),
        Err(err) => {
            tracing::warn!(job_id = %job.id, %export_id, error = %err.message, "job export failed");
            let _ = sqlx::query(
                "UPDATE job_exports SET state = 'failed', error = $2, completed_at = now()
                 WHERE id = $1 AND state = 'running'",
            )
            .bind(export_id)
            .bind(&err.message)
            .execute(&state.pool)
            .await;
        }
    }
}

/// What one uploaded artifact turned out to be.
struct Uploaded {
    bytes: i64,
    sha256: String,
    rows: i64,
}

/// How much NDJSON is gathered before it is handed to the blocking pool to be
/// compressed. Small enough that memory stays flat, large enough that the hop
/// to another thread is noise beside the compression it buys.
const COMPRESS_BATCH_BYTES: usize = 1024 * 1024;

/// The gzip stream and the digest of what it has produced, moved onto the
/// blocking pool and back for each batch.
struct Compressor {
    encoder: GzEncoder<Vec<u8>>,
    hasher: Sha256,
    bytes: i64,
}

impl Compressor {
    /// Compress `batch`, and hand back a part for the upload once a whole one
    /// has accumulated.
    fn push(mut self, batch: Vec<u8>) -> AppResult<(Self, Option<Vec<u8>>)> {
        self.encoder
            .write_all(&batch)
            .map_err(|e| AppError::internal(format!("export encode failed: {e}")))?;
        let part = (self.encoder.get_ref().len() >= MultipartUpload::PART_SIZE).then(|| {
            let part = std::mem::take(self.encoder.get_mut());
            self.hasher.update(&part);
            self.bytes += part.len() as i64;
            part
        });
        Ok((self, part))
    }

    /// Close the stream: the encoder's trailer has to go out with the final
    /// part, so it is finished before the last flush rather than after it.
    fn finish(mut self) -> AppResult<(Vec<u8>, i64, String)> {
        let tail = self
            .encoder
            .finish()
            .map_err(|e| AppError::internal(format!("export finalize failed: {e}")))?;
        self.hasher.update(&tail);
        self.bytes += tail.len() as i64;
        Ok((tail, self.bytes, hex::encode(self.hasher.finalize())))
    }
}

/// Run `work` on the blocking pool.
async fn off_the_executor<T: Send + 'static>(
    work: impl FnOnce() -> AppResult<T> + Send + 'static,
) -> AppResult<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| AppError::task_failed("compressing an export", e))?
}

/// Stream one query's rows out as gzipped NDJSON, straight into a multipart
/// upload at `key`.
///
/// Nothing larger than one part is ever resident: rows are read from a cursor,
/// written through the encoder, and flushed to S3 whenever the compressed
/// buffer reaches a part. That is what lets this run against a job whose
/// results do not fit in memory, which is every job worth exporting.
///
/// **The compression and the digest run on the blocking pool, not here.** They
/// are the whole cost of an export -- a corpus is gigabytes of JSON -- and done
/// inline they were one long computation on an async worker thread: the
/// database delivers rows faster than they compress, so the loop's `await`
/// never had to wait and never yielded. A Tokio worker that does not yield
/// stops more than its own task. Whichever worker last polled the I/O driver
/// is the one new socket events are waiting on, and if that is the worker doing
/// the compressing, nothing is accepted, read or written until it next comes
/// up for air -- by the whole server: `/health`, and every claim, heartbeat
/// and submission. Found with an export of one full-size leave generation
/// running: every other worker thread parked, one at 100%, and `/health`
/// unanswered for as long as the export ran. What is left on the executor is a
/// copy of each row's text into the next batch.
///
/// Read on the caller's connection, inside its snapshot ([`read_snapshot`]).
async fn upload_rows(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    key: &str,
    sql: &str,
    job_id: Uuid,
) -> AppResult<Uploaded> {
    let mut upload = state.artifacts.start_multipart(key).await?;
    let mut rows_written: i64 = 0;

    let result = async {
        let mut compressor = Compressor {
            encoder: GzEncoder::new(Vec::new(), Compression::default()),
            hasher: Sha256::new(),
            bytes: 0,
        };
        let mut batch: Vec<u8> = Vec::with_capacity(COMPRESS_BATCH_BYTES + 64 * 1024);

        let mut rows = sqlx::query(sql).bind(job_id).fetch(&mut *conn);
        while let Some(row) = rows.try_next().await? {
            let line: String = row.get("row");
            batch.extend_from_slice(line.as_bytes());
            batch.push(b'\n');
            rows_written += 1;

            if batch.len() >= COMPRESS_BATCH_BYTES {
                let full = std::mem::replace(
                    &mut batch,
                    Vec::with_capacity(COMPRESS_BATCH_BYTES + 64 * 1024),
                );
                let (next, part) = off_the_executor(move || compressor.push(full)).await?;
                compressor = next;
                if let Some(part) = part {
                    upload.upload_part(part).await?;
                }
            }
        }
        drop(rows);

        let (tail, bytes, sha256) = off_the_executor(move || {
            let (compressor, part) = compressor.push(batch)?;
            // A part that filled on the very last batch still has to precede
            // the trailer, so both come back, in order.
            let (tail, bytes, sha256) = compressor.finish()?;
            Ok((part.into_iter().chain([tail]).collect::<Vec<_>>(), bytes, sha256))
        })
        .await?;
        for part in tail {
            // S3 rejects a zero-length part, and an empty job is a legitimate
            // export: gzip's own trailer means the last part is never actually
            // empty, but the guard costs nothing and says so.
            if !part.is_empty() {
                upload.upload_part(part).await?;
            }
        }
        Ok::<(i64, String), AppError>((bytes, sha256))
    }
    .await;

    match result {
        Ok((bytes, sha256)) => {
            upload.finish().await?;
            Ok(Uploaded { bytes, sha256, rows: rows_written })
        }
        Err(err) => {
            upload.abort().await;
            Err(err)
        }
    }
}

/// Build the export's artifacts and mark the row ready.
///
/// One artifact for every job type -- its result rows -- and a second for a
/// games or game-pairs job that captured positions: see [`positions_query`].
/// Both are written before the row says `ready`, so a download never finds one
/// without the other.
async fn build(state: &AppState, job: &Job, export_id: Uuid) -> AppResult<()> {
    // Here rather than in `start`: a full-size merge is the best part of a
    // minute, and this is the background task. Read afresh, since a job
    // exported mid-run may have completed while this waited its turn. A
    // running leave job is not merged: its export reflects the last merge,
    // as its page does, and forcing one per export would put a merge's write
    // volume in an admin's hands (`leave_gen::MERGE_INTERVAL`).
    let completed = sqlx::query_scalar::<_, bool>("SELECT status = 'completed' FROM jobs WHERE id = $1")
        .bind(job.id)
        .fetch_optional(&state.pool)
        .await?
        .unwrap_or(false);
    if completed {
        settle(&state.pool, job).await?;
    }

    let key = format!("exports/{}/{export_id}.ndjson.gz", job.id);
    let positions_key = format!("exports/{}/{export_id}.positions.ndjson.gz", job.id);
    let read = read_snapshot(state, job, export_id, &key, &positions_key).await;
    let finished = match read {
        Ok(snapshot) => mark_ready(state, export_id, &key, &positions_key, snapshot).await,
        Err(err) => Err(err),
    };
    // Failed after an object was written -- the positions' upload, or marking
    // the row -- and the row will say `failed`, naming neither object: removed
    // now rather than left to the thirty-day lifecycle rule. (An object never
    // written is a delete of nothing.) Unless the row says `ready` after all --
    // the update committed and only its answer was lost -- or cannot be read to
    // say: a row naming a deleted object would redirect every download of the
    // job to it.
    if finished.is_err() && !named_ready(state, export_id).await {
        for key in [&key, &positions_key] {
            if let Err(err) = state.artifacts.delete(key).await {
                tracing::warn!(%export_id, key, error = %err.message, "removing a failed export's object failed");
            }
        }
    }
    finished
}

/// What one snapshot read produced: the artifacts, and what the job was when
/// the snapshot was taken.
struct Snapshot {
    results: Uploaded,
    positions: Option<Uploaded>,
    is_final: bool,
    taken_at: chrono::DateTime<chrono::Utc>,
}

/// Reads the corpus -- the results, then the captured positions -- in **one**
/// `REPEATABLE READ` transaction on one connection, and uploads each as it
/// goes.
///
/// One snapshot, because a job being exported may be taking results: read on
/// two connections, each scan saw its own moment, and the positions file
/// could name results the results file does not hold. Read-only, since
/// nothing here writes. The connection is held for both scans, as the scans
/// each held one before; `EXPORT_BUILDS` bounds how many do.
///
/// The first statement takes the snapshot and asks, of the state it sees,
/// whether this is the job's final corpus: completed, with no claim still open
/// and, for a leave job, nothing staged that a merge has not folded in. What
/// `start` saw does not count -- a job exported while active can complete
/// before this runs, with its last claims still landing.
async fn read_snapshot(
    state: &AppState,
    job: &Job,
    export_id: Uuid,
    key: &str,
    positions_key: &str,
) -> AppResult<Snapshot> {
    // Closed rather than returned when this ends: a pool connection dropped
    // mid-result is drained before it is reused, so an upload that failed
    // part way left Postgres building the rest of the corpus on a connection
    // nothing counted (thirty-first audit). Closing it ends the query at the
    // server's next write -- and, here, the transaction with it.
    let mut conn = state.pool.acquire().await?;
    conn.close_on_drop();
    let mut tx = sqlx::Connection::begin(&mut *conn).await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await?;
    let (is_final, taken_at): (bool, chrono::DateTime<chrono::Utc>) = sqlx::query_as(
        "SELECT CASE WHEN j.status <> 'completed' THEN FALSE
                     ELSE NOT EXISTS (SELECT 1 FROM task_claims c JOIN tasks t ON t.id = c.task_id
                                      WHERE t.job_id = j.id AND c.state = 'claimed')
                      AND NOT EXISTS (SELECT 1 FROM leave_rack_staging s WHERE s.job_id = j.id)
                END,
                clock_timestamp()
         FROM jobs j WHERE j.id = $1",
    )
    .bind(job.id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::not_found("the job was deleted while its export waited"))?;

    let results = upload_rows(state, &mut tx, key, export_query(job.job_type), job.id).await?;
    // Asked of the rows rather than of the job's `capture_positions` setting:
    // what matters is whether there is anything to export, and a capture job
    // nobody contributed positions to should not grow an empty artifact.
    let captured = may_capture_positions(job.job_type)
        && sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM position_analysis_records WHERE job_id = $1)",
        )
        .bind(job.id)
        .fetch_one(&mut *tx)
        .await?;
    let positions = if captured {
        Some(upload_rows(state, &mut tx, positions_key, positions_query(), job.id).await?)
    } else {
        None
    };
    tx.commit().await?;
    tracing::debug!(%export_id, is_final, "export snapshot read");
    Ok(Snapshot { results, positions, is_final, taken_at })
}

/// Whether the export's row says `ready` -- or cannot be read to say it does
/// not.
async fn named_ready(state: &AppState, export_id: Uuid) -> bool {
    sqlx::query_scalar::<_, bool>(
        "SELECT COALESCE((SELECT state = 'ready' FROM job_exports WHERE id = $1), FALSE)",
    )
    .bind(export_id)
    .fetch_one(&state.pool)
    .await
    .unwrap_or(true)
}

/// Marks the export ready, naming the objects its snapshot wrote.
async fn mark_ready(
    state: &AppState,
    export_id: Uuid,
    key: &str,
    positions_key: &str,
    snapshot: Snapshot,
) -> AppResult<()> {
    let Snapshot { results, positions, is_final, taken_at } = snapshot;
    let positions = positions.map(|uploaded| (positions_key, uploaded));

    // Guarded on `running`, like the import's: a process starting while this
    // one works reaps rows left `running` as failed, and a rolling deployment
    // overlaps the two. Without the guard a reaped row would come back
    // `ready`, and an admin would be handed a download of an export nobody
    // was sure had finished.
    //
    // The job's status is read `FOR SHARE`, which waits out a transaction
    // changing it and then reads what that committed. A consensus edit
    // reopening the job demotes its final exports and fails its running ones
    // (`unfinalize`) before it commits; a plain read here, between that
    // statement and the commit, still saw `completed`, and stored this export
    // final for a job that was active again. Waiting, it re-reads this row as
    // the edit left it, `failed`, and matches nothing.
    let marked = sqlx::query(
        "UPDATE job_exports
         SET state = 'ready', artifact_key = $2, bytes = $3, sha256 = $4,
             row_count = $5, positions_artifact_key = $6, positions_bytes = $7,
             positions_sha256 = $8, positions_row_count = $9,
             is_final = $10 AND (SELECT j.status = 'completed' FROM jobs j
                                 WHERE j.id = job_exports.job_id FOR SHARE),
             snapshot_at = $11, completed_at = now()
         WHERE id = $1 AND state = 'running'",
    )
    .bind(export_id)
    .bind(key)
    .bind(results.bytes)
    .bind(&results.sha256)
    .bind(results.rows)
    .bind(positions.as_ref().map(|(key, _)| *key))
    .bind(positions.as_ref().map(|(_, p)| p.bytes))
    .bind(positions.as_ref().map(|(_, p)| p.sha256.as_str()))
    .bind(positions.as_ref().map(|(_, p)| p.rows))
    .bind(is_final)
    .bind(taken_at)
    .execute(&state.pool)
    .await?
    .rows_affected();
    // No row to hand them to -- a purge or delete removed it while this
    // built, or a new process reaped it -- so nothing will ever name these
    // objects. Removed now rather than left to the thirty-day lifecycle rule.
    if marked == 0 {
        for key in std::iter::once(key).chain(positions.as_ref().map(|(key, _)| *key)) {
            if let Err(err) = state.artifacts.delete(key).await {
                tracing::warn!(%export_id, key, error = %err.message, "removing an unclaimed export failed");
            }
        }
    }
    Ok(())
}

/// A ready export's objects.
pub struct ReadyExport {
    pub id: Uuid,
    pub artifact_key: String,
    /// The captured positions' artifact, for a games or game-pairs job that
    /// captured any.
    pub positions_artifact_key: Option<String>,
}

/// How long an export's objects are relied on after it is built: a day short
/// of the bucket's lifecycle rule, which deletes everything under `exports/`
/// thirty days after it is written (`infra/s3.tf`, `expire-job-exports`). The
/// two move together.
///
/// Past it the row still says `ready`, but the objects are gone or about to
/// be. Counted as ready, it was what a completed job's result stream
/// redirected to -- a `404` from the store, every time, with the scan it would
/// otherwise fall back to never reached -- and what the admin page offered as
/// a download.
pub const EXPORT_LIFETIME_DAYS: i32 = 29;

/// The newest **final** ready export for a job whose objects are still in the
/// store, if there is one: the completed job's corpus, which the results
/// stream redirects to. A snapshot is never this, however new -- it was read
/// while the job was still taking results.
pub async fn newest_ready(pool: &sqlx::PgPool, job_id: Uuid) -> AppResult<Option<ReadyExport>> {
    Ok(sqlx::query_as::<_, (Uuid, String, Option<String>)>(
        "SELECT id, artifact_key, positions_artifact_key FROM job_exports
         WHERE job_id = $1 AND state = 'ready' AND is_final AND artifact_key IS NOT NULL
           AND completed_at > now() - make_interval(days => $2)
         ORDER BY requested_at DESC LIMIT 1",
    )
    .bind(job_id)
    .bind(EXPORT_LIFETIME_DAYS)
    .fetch_optional(pool)
    .await?
    .map(|(id, artifact_key, positions_artifact_key)| ReadyExport {
        id,
        artifact_key,
        positions_artifact_key,
    }))
}

/// One export's objects, if it is ready and they are still in the store --
/// final or snapshot. What the admin page offers as the download of the export
/// it shows.
pub async fn ready_objects(pool: &sqlx::PgPool, export_id: Uuid) -> AppResult<Option<ReadyExport>> {
    Ok(sqlx::query_as::<_, (Uuid, String, Option<String>)>(
        "SELECT id, artifact_key, positions_artifact_key FROM job_exports
         WHERE id = $1 AND state = 'ready' AND artifact_key IS NOT NULL
           AND completed_at > now() - make_interval(days => $2)",
    )
    .bind(export_id)
    .bind(EXPORT_LIFETIME_DAYS)
    .fetch_optional(pool)
    .await?
    .map(|(id, artifact_key, positions_artifact_key)| ReadyExport {
        id,
        artifact_key,
        positions_artifact_key,
    }))
}

/// Drop a job's exports and the objects behind them.
///
/// Called by purge, which deletes the results an export describes. A row left
/// saying `ready` would then hand an admin a stable-looking artifact of a job
/// that no longer holds any of it, which is worse than no export at all.
pub async fn purge(conn: &mut sqlx::PgConnection, job_id: Uuid) -> AppResult<Vec<String>> {
    // In the purge's own transaction: removed after it, a failure left the
    // old run's `ready` row to be served -- once the job had completed again
    // -- as the new run's results. Returns the objects to remove once it has
    // committed ([`remove_objects`]).
    let rows: Vec<(Option<String>, Option<String>)> = sqlx::query_as(
        "DELETE FROM job_exports WHERE job_id = $1
         RETURNING artifact_key, positions_artifact_key",
    )
    .bind(job_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().flat_map(|(results, positions)| [results, positions]).flatten().collect())
}

/// A reopened job's final exports become snapshots: each is still the corpus
/// as of its `snapshot_at`, but no longer the finished job's, which the job
/// will have again once it completes. Left final, `newest_ready` would serve
/// the old corpus as the completed job's once it completed again.
///
/// An export still building fails, and is to be taken again. Its snapshot may
/// have been read while the job was completed, and the job can complete again
/// before the export finishes -- a large corpus uploads for minutes, an edit
/// that unsettles a handful of racks is re-completed within one reissue's
/// analysis -- and `mark_ready`, reading the job completed once more, stored
/// the corpus from before the edit as the final one: what every download of
/// the completed job was redirected to until someone exported it again.
/// Failed here, its `mark_ready` matches no row and removes its objects. (That update reads the job's row `FOR SHARE`
/// before it locks its own, so one waiting on the reopening re-reads this
/// row once it commits rather than deadlocking with it.)
pub async fn unfinalize(conn: &mut sqlx::PgConnection, job_id: Uuid) -> AppResult<()> {
    sqlx::query("UPDATE job_exports SET is_final = FALSE WHERE job_id = $1 AND is_final")
        .bind(job_id)
        .execute(&mut *conn)
        .await?;
    sqlx::query(
        "UPDATE job_exports
         SET state = 'failed', completed_at = now(),
             error = 'the job reopened while this export was building: export it again'
         WHERE job_id = $1 AND state = 'running'",
    )
    .bind(job_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Removes a purge's export objects, off the request: best-effort cleanup of
/// derived data whose rows are already gone, so an object that survives is
/// one the bucket's lifecycle rule expires rather than anything anyone can
/// reach. Awaiting an unreachable object store here would hold the purge
/// response open for the SDK's whole retry budget, once per object.
pub fn remove_objects(state: &AppState, keys: Vec<String>) {
    if keys.is_empty() {
        return;
    }
    let artifacts = state.artifacts.clone();
    tokio::spawn(async move {
        for key in keys {
            if let Err(err) = artifacts.delete(&key).await {
                tracing::warn!(%key, error = %err.message, "could not delete an export object");
            }
        }
    });
}

/// Startup reaper. Single instance, so a row left `running` belongs to a
/// process that is gone.
pub async fn fail_orphaned(pool: &sqlx::PgPool) -> AppResult<u64> {
    Ok(sqlx::query(
        "UPDATE job_exports
         SET state = 'failed', completed_at = now(),
             error = 'the server restarted while this export was running'
         WHERE state = 'running'",
    )
    .execute(pool)
    .await?
    .rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::RngCore;
    use std::io::Read;

    /// The compressor is moved to the blocking pool and back once per batch,
    /// and hands out a part whenever a whole one has accumulated. What the
    /// upload receives -- every part, then the tail -- has to be one gzip
    /// stream of exactly the lines pushed, and the digest and byte count
    /// recorded on the export have to describe those bytes.
    #[test]
    fn the_parts_are_one_gzip_stream_of_what_was_pushed() {
        let mut compressor = Compressor {
            encoder: GzEncoder::new(Vec::new(), Compression::default()),
            hasher: Sha256::new(),
            bytes: 0,
        };
        // Incompressible, so the compressed stream passes a part's size.
        let mut rng = rand::thread_rng();
        let mut pushed = Vec::new();
        let mut uploaded = Vec::new();
        let mut parts = 0;
        for _ in 0..10 {
            let mut batch = vec![0u8; COMPRESS_BATCH_BYTES];
            rng.fill_bytes(&mut batch);
            pushed.extend_from_slice(&batch);
            let (next, part) = compressor.push(batch).unwrap();
            compressor = next;
            if let Some(part) = part {
                assert!(part.len() >= MultipartUpload::PART_SIZE, "S3 refuses a short part");
                uploaded.extend_from_slice(&part);
                parts += 1;
            }
        }
        assert_eq!(parts, 1, "ten incompressible mebibytes are one 8 MiB part and a tail");
        let (tail, bytes, sha256) = compressor.finish().unwrap();
        uploaded.extend_from_slice(&tail);

        assert_eq!(bytes as usize, uploaded.len());
        assert_eq!(sha256, hex::encode(Sha256::digest(&uploaded)));
        let mut read_back = Vec::new();
        flate2::read::GzDecoder::new(&uploaded[..]).read_to_end(&mut read_back).unwrap();
        assert!(read_back == pushed, "the artifact decompresses to the lines that were pushed");
    }
}
