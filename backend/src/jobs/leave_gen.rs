use super::handler::*;
use super::racks::LetterDistribution;
use super::JobData;
use crate::artifacts::ArtifactStore;
use crate::error::{AppError, AppResult};
use crate::models::job::LeaveConfig;
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, Row};
use uuid::Uuid;

pub struct LeaveGenHandler;

/// Writes the typed request row alongside the task.
pub async fn insert_request(
    conn: &mut PgConnection,
    task_id: Uuid,
    req: &LeaveRequest,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO leave_requests
             (task_id, lexicon, variant, letter_distribution, generation,
              forced_racks, num_games, previous_artifact_key, use_wordmap)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(task_id)
    .bind(&req.lexicon)
    .bind(&req.variant)
    .bind(&req.letter_distribution)
    .bind(req.generation)
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
            "SELECT lexicon, variant, letter_distribution, generation, forced_racks,
                    num_games, previous_artifact_key, use_wordmap
             FROM leave_requests WHERE task_id = $1",
        )
        .bind(task_id)
        .fetch_one(conn)
        .await?;
        Ok(LeaveRequest {
            lexicon: row.get("lexicon"),
            variant: row.get("variant"),
            letter_distribution: row.get("letter_distribution"),
            generation: row.get("generation"),
            forced_racks: row.get("forced_racks"),
            num_games: row.get("num_games"),
            previous_artifact_key: row.get("previous_artifact_key"),
            use_wordmap: row.get("use_wordmap"),
        })
    }

    fn process_response(response: Self::Response) -> AppResult<Self::Record> {
        if response.racks.is_empty() {
            return Err(AppError::bad_request("leave result carried no rack occurrences"));
        }
        super::plausibility::check_rack_occurrences(&response.racks)?;
        Ok(LeaveRecord { racks: response.racks })
    }

    async fn insert_record(
        conn: &mut PgConnection,
        task_id: Uuid,
        claim_id: Uuid,
        record: &Self::Record,
    ) -> AppResult<()> {
        let row = sqlx::query(
            "SELECT r.generation, t.job_id
             FROM leave_requests r JOIN tasks t ON t.id = r.task_id
             WHERE r.task_id = $1",
        )
        .bind(task_id)
        .fetch_one(&mut *conn)
        .await?;
        let generation: i32 = row.get("generation");
        let job_id: Uuid = row.get("job_id");

        sqlx::query(
            "INSERT INTO leave_records (task_claim_id, task_id, rack_count)
             VALUES ($1, $2, $3)",
        )
        .bind(claim_id)
        .bind(task_id)
        .bind(record.racks.len() as i32)
        .execute(&mut *conn)
        .await?;

        // A single submission can carry thousands of racks, so the progress
        // upsert is issued as multi-row statements rather than row-by-row.
        const CHUNK: usize = 1000;
        for chunk in record.racks.chunks(CHUNK) {
            let mut builder = sqlx::QueryBuilder::new(
                "INSERT INTO leave_rack_progress
                     (job_id, generation, rack, occurrence_count, equity_sum) ",
            );
            builder.push_values(chunk.iter(), |mut b, occ| {
                b.push_bind(job_id)
                    .push_bind(generation)
                    .push_bind(occ.rack.clone())
                    .push_bind(occ.count)
                    .push_bind(occ.mean * occ.count as f64);
            });
            builder.push(
                " ON CONFLICT (job_id, generation, rack) DO UPDATE SET
                     occurrence_count = leave_rack_progress.occurrence_count + excluded.occurrence_count,
                     equity_sum       = leave_rack_progress.equity_sum + excluded.equity_sum,
                     updated_at       = now()",
            );
            builder.build().execute(&mut *conn).await?;
        }
        Ok(())
    }
}

/// What the scheduler should do next for a leave-generation job.
pub enum LeaveGenStep {
    /// Dispatch this forced-rack partition.
    Dispatch(LeaveRequest),
    /// Every rack in this generation hit its target and no claim is in flight;
    /// the generation must be aggregated before any more work exists. Done
    /// outside the claim transaction because it uploads to S3.
    Transition { generation: i32 },
    /// All configured generations are complete.
    Finished,
    /// Racks remain below target but every one of them is already out with a
    /// worker — nothing to hand out right now.
    NoWorkYet,
}

/// Claim-time rack selection: the racks furthest from this generation's target.
pub async fn next_step(
    conn: &mut PgConnection,
    job_id: Uuid,
    config: &LeaveConfig,
    job_data: &JobData,
) -> AppResult<LeaveGenStep> {
    // Generation 0 has an artifact too -- the zeroed KLV generation 1 plays
    // with -- so it must not count as a completed generation.
    let completed = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM leave_generation_artifacts
         WHERE job_id = $1 AND generation >= 1",
    )
    .bind(job_id)
    .fetch_one(&mut *conn)
    .await?;

    if completed >= config.generation_count as i64 {
        return Ok(LeaveGenStep::Finished);
    }
    let generation = completed as i32 + 1;

    let racks = sqlx::query_scalar::<_, String>(
        "SELECT rack FROM leave_rack_progress
         WHERE job_id = $1 AND generation = $2 AND occurrence_count < $3
         ORDER BY occurrence_count ASC, rack ASC
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

        return Ok(if in_flight > 0 {
            LeaveGenStep::NoWorkYet
        } else {
            LeaveGenStep::Transition { generation }
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
        generation,
        forced_racks: racks,
        previous_artifact_key,
        num_games: config.num_iterations,
        use_wordmap: config.use_wordmap,
    }))
}

/// Write the rack universe for `generation` at zero occurrences. "Racks with no
/// row yet count as 0" needs a known universe to draw from, and materializing it
/// once per generation is what lets claim-time selection be a single indexed
/// `ORDER BY occurrence_count` query.
pub async fn seed_generation(
    conn: &mut PgConnection,
    job_id: Uuid,
    generation: i32,
    config: &LeaveConfig,
    distribution: &LetterDistribution,
) -> AppResult<i64> {
    let leaves = distribution.enumerate_leaves(config.max_leave_size as usize);
    tracing::info!(job_id = %job_id, generation, leaves = leaves.len(), "seeding leave rack universe");

    const CHUNK: usize = 1000;
    for chunk in leaves.chunks(CHUNK) {
        let mut builder = sqlx::QueryBuilder::new(
            "INSERT INTO leave_rack_progress (job_id, generation, rack) ",
        );
        builder.push_values(chunk.iter(), |mut b, rack| {
            b.push_bind(job_id).push_bind(generation).push_bind(rack.clone());
        });
        builder.push(" ON CONFLICT (job_id, generation, rack) DO NOTHING");
        builder.build().execute(&mut *conn).await?;
    }
    Ok(leaves.len() as i64)
}

/// Close out a generation: fold `leave_rack_progress` into per-rack mean
/// equities, build the generation's KLV directly (see `klv.rs`), store the
/// artifact, and seed the next generation's rack universe.
pub async fn run_transition(
    pool: &sqlx::PgPool,
    artifacts: &ArtifactStore,
    job_id: Uuid,
    generation: i32,
    config: &LeaveConfig,
    distribution: &LetterDistribution,
) -> AppResult<String> {
    // Shared with `rebuild_artifacts` rather than written twice: a rebuild is
    // only meaningful if it folds the rows exactly as the original write did,
    // and two copies of this would be free to drift into producing different
    // bytes for the same generation.
    let mean_by_rack = generation_means(pool, job_id, generation).await?;

    let klv = super::klv::build(distribution, &mean_by_rack)?;

    // Hashed as written, not read back: the object store holds the only copy
    // of these bytes, and this is what a later rebuild is compared against.
    let sha256 = hex::encode(Sha256::digest(&klv));
    let key = format!("leaves/{job_id}/generation-{generation}.klv2");
    artifacts.put(&key, klv).await?;

    let mut tx = pool.begin().await?;
    // DO NOTHING keeps the FIRST hash. A restore that replays this transition
    // against fewer results writes the same key with different bytes; keeping
    // the original hash is what makes that visible afterwards instead of
    // quietly agreeing with whatever landed last.
    sqlx::query(
        "INSERT INTO leave_generation_artifacts (job_id, generation, artifact_key, sha256)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (job_id, generation) DO NOTHING",
    )
    .bind(job_id)
    .bind(generation)
    .bind(&key)
    .bind(&sha256)
    .execute(&mut *tx)
    .await?;

    if generation < config.generation_count {
        seed_generation(&mut tx, job_id, generation + 1, config, distribution).await?;
    } else {
        sqlx::query("UPDATE jobs SET status = 'completed' WHERE id = $1")
            .bind(job_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;

    Ok(key)
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
    job_id: Uuid,
    distribution: &LetterDistribution,
) -> AppResult<String> {
    // Empty means "no rack has a mean equity yet", which `build` renders as
    // 0.0 for every leave -- exactly the zeroed KLV wanted here.
    let klv = super::klv::build(distribution, &std::collections::HashMap::new())?;
    let sha256 = hex::encode(Sha256::digest(&klv));
    let key = format!("leaves/{job_id}/generation-0.klv2");
    artifacts.put(&key, klv).await?;

    sqlx::query(
        "INSERT INTO leave_generation_artifacts (job_id, generation, artifact_key, sha256)
         VALUES ($1, 0, $2, $3)
         ON CONFLICT (job_id, generation) DO NOTHING",
    )
    .bind(job_id)
    .bind(&key)
    .bind(&sha256)
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
/// Two things this deliberately does *not* do:
///
/// - **It does not overwrite an object that is present but differs.** A hash
///   mismatch means the stored bytes are not what this code would produce now,
///   and that is evidence to look at rather than a fault to paper over: it is
///   equally consistent with a corrupted object and with a legitimate change to
///   `klv::build`. Rewriting on sight would destroy the only copy of whichever
///   one it was. `force` is the deliberate override.
/// - **It does not rebuild generation 0 from `leave_rack_progress`.** Generation
///   0 is the zeroed KLV every job starts from, not a fold of any results, and
///   there are no progress rows behind it. It is rebuilt the way
///   `seed_zero_generation` built it, from an empty map.
pub async fn rebuild_artifacts(
    pool: &sqlx::PgPool,
    artifacts: &ArtifactStore,
    job_id: Uuid,
    distribution: &LetterDistribution,
    force: bool,
) -> AppResult<Vec<ArtifactRebuild>> {
    let rows = sqlx::query(
        "SELECT generation, artifact_key, sha256
         FROM leave_generation_artifacts
         WHERE job_id = $1
         ORDER BY generation",
    )
    .bind(job_id)
    .fetch_all(pool)
    .await?;

    let mut report = Vec::with_capacity(rows.len());
    for row in rows {
        let generation: i32 = row.get("generation");
        let artifact_key: String = row.get("artifact_key");
        let stored_sha256: String = row.get("sha256");

        let mean_by_rack = if generation == 0 {
            std::collections::HashMap::new()
        } else {
            generation_means(pool, job_id, generation).await?
        };
        let klv = super::klv::build(distribution, &mean_by_rack)?;
        let rebuilt_sha256 = hex::encode(Sha256::digest(&klv));

        let object_present = artifacts.exists(&artifact_key).await?;
        let matches = rebuilt_sha256 == stored_sha256;
        // A missing object has no bytes to lose, so restoring it needs no
        // permission. Replacing one that is present does.
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
            object_present,
            rewritten,
        });
    }
    Ok(report)
}

/// Per-rack mean equity for one generation: the same fold `run_transition`
/// performs, which is what makes a rebuild reproduce the original bytes.
async fn generation_means(
    pool: &sqlx::PgPool,
    job_id: Uuid,
    generation: i32,
) -> AppResult<std::collections::HashMap<String, f64>> {
    let rows = sqlx::query(
        "SELECT rack, occurrence_count, equity_sum
         FROM leave_rack_progress
         WHERE job_id = $1 AND generation = $2 AND occurrence_count > 0
         ORDER BY rack",
    )
    .bind(job_id)
    .bind(generation)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|row| {
            let rack: String = row.get("rack");
            let count: i64 = row.get("occurrence_count");
            let equity_sum: f64 = row.get("equity_sum");
            (rack, equity_sum / count as f64)
        })
        .collect())
}
