//! Building a completed job's results into one downloadable artifact.
//!
//! The live results stream (`/api/admin/jobs/:id/results/stream`) scans the
//! job's result tables from a cursor and holds a pool connection for as long as
//! the caller keeps reading. That is fine for a spot check and wrong for a
//! corpus: a full English opening-rack job is tens of millions of rows, and one
//! caller per scan is one connection per scan against a pool of twenty.
//!
//! An export is that read done **once**. It is restricted to completed jobs,
//! and that restriction is what makes it worth building: a completed job's
//! results are immutable, so the artifact is built once and reused by every
//! later download, where an export of an active job would be stale as it was
//! written.
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

/// The rows an export contains, per job type. The same queries the live stream
/// runs, so the two produce the same corpus.
fn export_query(job_type: JobType) -> &'static str {
    match job_type {
        JobType::OpeningRack => {
            "SELECT to_jsonb(r) AS row FROM position_analysis_records r
             WHERE r.job_id = $1"
        }
        JobType::Games | JobType::GamePairs => {
            "SELECT to_jsonb(r) AS row FROM game_results r WHERE r.job_id = $1"
        }
        JobType::LeaveGeneration => {
            "SELECT to_jsonb(r) AS row FROM leave_rack_progress r WHERE r.job_id = $1"
        }
    }
}

/// Start an export, returning its id. The work happens on a spawned task.
///
/// Refuses a job that is not completed: an export of a job still taking results
/// would be obsolete before anyone downloaded it, and nothing would say so.
pub async fn start(state: &AppState, job: &Job, requested_by: Uuid) -> AppResult<Uuid> {
    if job.status != JobStatus::Completed {
        return Err(AppError::conflict(
            "only a completed job can be exported: an export of a job still \
             taking results would be stale before it finished",
        ));
    }

    // Completed is not yet settled. A job flips to completed the moment its
    // stopping rule is met or an admin forces it, but every claim already out
    // is still played and still accepted -- the submit path checks the claim,
    // not the job's status. An export built in that window missed those
    // results, and the stream redirects every later download to it, so the
    // corpus a completed job hands out would be short for good. No claim can be
    // issued against a completed job, so once none is open the results really
    // are fixed.
    //
    // Reclamation is lazy: a lapsed claim is flipped to `abandoned` when a
    // worker next asks for work from the job's priority tier, and nothing ever
    // asks for work from a completed job. So a claim whose worker died stayed
    // `claimed` for good, and refused every export of the job for good --
    // where the design says a claim lapses at the heartbeat timeout. Reclaimed
    // here first, through the same statement dispatch uses, so "open" below
    // means live.
    crate::scheduler::reclaim_expired(
        &state.pool,
        job.id,
        state.cfg.heartbeat_timeout.as_secs_f64(),
    )
    .await?;
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
             heartbeat timeout",
        ));
    }

    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO job_exports (job_id, requested_by) VALUES ($1, $2) RETURNING id",
    )
    .bind(job.id)
    .bind(requested_by)
    .fetch_one(&state.pool)
    .await?;

    let (state, job) = (state.clone(), job.clone());
    tokio::spawn(async move { run(state, job, id).await });
    Ok(id)
}

async fn run(state: AppState, job: Job, export_id: Uuid) {
    match build(&state, &job, export_id).await {
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

/// Stream the job's rows out as gzipped NDJSON, straight into a multipart
/// upload.
///
/// Nothing larger than one part is ever resident: rows are read from a cursor,
/// written through the encoder, and flushed to S3 whenever the compressed
/// buffer reaches a part. That is what lets this run against a job whose
/// results do not fit in memory, which is every job worth exporting.
async fn build(state: &AppState, job: &Job, export_id: Uuid) -> AppResult<()> {
    let key = format!("exports/{}/{export_id}.ndjson.gz", job.id);
    let mut upload = state.artifacts.start_multipart(&key).await?;

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    let mut hasher = Sha256::new();
    let mut bytes: i64 = 0;
    let mut rows_written: i64 = 0;

    let result = async {
        let mut rows = sqlx::query(export_query(job.job_type)).bind(job.id).fetch(&state.pool);
        while let Some(row) = rows.try_next().await? {
            let value: serde_json::Value = row.get("row");
            writeln!(encoder, "{value}")
                .map_err(|e| AppError::internal(format!("export encode failed: {e}")))?;
            rows_written += 1;

            if encoder.get_ref().len() >= MultipartUpload::PART_SIZE {
                let part = std::mem::take(encoder.get_mut());
                hasher.update(&part);
                bytes += part.len() as i64;
                upload.upload_part(part).await?;
            }
        }
        drop(rows);

        // The encoder's trailer has to go out with the final part, so the
        // stream is finished before the last flush rather than after it.
        let tail = encoder
            .finish()
            .map_err(|e| AppError::internal(format!("export finalize failed: {e}")))?;
        hasher.update(&tail);
        bytes += tail.len() as i64;
        // S3 rejects a zero-length part, and an empty job is a legitimate
        // export: gzip's own trailer means `tail` is never actually empty, but
        // the guard costs nothing and says so.
        if !tail.is_empty() {
            upload.upload_part(tail).await?;
        }
        Ok::<(), AppError>(())
    }
    .await;

    if let Err(err) = result {
        upload.abort().await;
        return Err(err);
    }
    upload.finish().await?;

    // Guarded on `running`, like the import's: a process starting while this
    // one works reaps rows left `running` as failed, and a rolling deployment
    // overlaps the two. Without the guard a reaped row would come back
    // `ready`, and an admin would be handed a download of an export nobody
    // was sure had finished.
    sqlx::query(
        "UPDATE job_exports
         SET state = 'ready', artifact_key = $2, bytes = $3, sha256 = $4,
             row_count = $5, completed_at = now()
         WHERE id = $1 AND state = 'running'",
    )
    .bind(export_id)
    .bind(&key)
    .bind(bytes)
    .bind(hex::encode(hasher.finalize()))
    .bind(rows_written)
    .execute(&state.pool)
    .await?;
    Ok(())
}

/// The newest ready export for a job, if there is one.
pub async fn newest_ready(pool: &sqlx::PgPool, job_id: Uuid) -> AppResult<Option<(Uuid, String)>> {
    Ok(sqlx::query_as::<_, (Uuid, String)>(
        "SELECT id, artifact_key FROM job_exports
         WHERE job_id = $1 AND state = 'ready' AND artifact_key IS NOT NULL
         ORDER BY requested_at DESC LIMIT 1",
    )
    .bind(job_id)
    .fetch_optional(pool)
    .await?)
}

/// Drop a job's exports and the objects behind them.
///
/// Called by purge, which deletes the results an export describes. A row left
/// saying `ready` would then hand an admin a stable-looking artifact of a job
/// that no longer holds any of it, which is worse than no export at all.
pub async fn purge(state: &AppState, job_id: Uuid) -> AppResult<()> {
    let keys: Vec<String> = sqlx::query_scalar(
        "DELETE FROM job_exports WHERE job_id = $1 AND artifact_key IS NOT NULL
         RETURNING artifact_key",
    )
    .bind(job_id)
    .fetch_all(&state.pool)
    .await?;
    sqlx::query("DELETE FROM job_exports WHERE job_id = $1")
        .bind(job_id)
        .execute(&state.pool)
        .await?;

    // Off the request, because purge is a synchronous admin call and this is
    // best-effort cleanup of derived data: the rows are already gone, so an
    // object that survives is one the bucket's lifecycle rule expires rather
    // than anything anyone can reach. Awaiting an unreachable object store here
    // would hold the purge response open for the SDK's whole retry budget, once
    // per object.
    if !keys.is_empty() {
        let artifacts = state.artifacts.clone();
        tokio::spawn(async move {
            for key in keys {
                if let Err(err) = artifacts.delete(&key).await {
                    tracing::warn!(%key, error = %err.message, "could not delete an export object");
                }
            }
        });
    }
    Ok(())
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
