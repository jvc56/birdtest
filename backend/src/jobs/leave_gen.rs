use super::handler::*;
use super::racks::LetterDistribution;
use crate::artifacts::ArtifactStore;
use crate::error::{AppError, AppResult};
use crate::models::job::LeaveConfig;
use sqlx::{PgConnection, Row};
use std::path::Path;
use uuid::Uuid;

pub struct LeaveGenHandler;

impl JobHandler for LeaveGenHandler {
    type Request = LeaveRequest;
    type Response = LeaveResponse;
    type Record = LeaveRecord;

    fn creation_strategy() -> CreationStrategy {
        CreationStrategy::OnDemand
    }

    async fn insert_request(
        conn: &mut PgConnection,
        task_id: Uuid,
        req: &Self::Request,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO leave_requests
                 (task_id, lexicon, variant, generation, forced_racks, num_games,
                  previous_artifact_key)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(task_id)
        .bind(&req.lexicon)
        .bind(&req.variant)
        .bind(req.generation)
        .bind(&req.forced_racks)
        .bind(req.num_games)
        .bind(&req.previous_artifact_key)
        .execute(conn)
        .await?;
        Ok(())
    }

    async fn load_request(conn: &mut PgConnection, task_id: Uuid) -> AppResult<Self::Request> {
        let row = sqlx::query(
            "SELECT lexicon, variant, generation, forced_racks, num_games, previous_artifact_key
             FROM leave_requests WHERE task_id = $1",
        )
        .bind(task_id)
        .fetch_one(conn)
        .await?;
        Ok(LeaveRequest {
            lexicon: row.get("lexicon"),
            variant: row.get("variant"),
            generation: row.get("generation"),
            forced_racks: row.get("forced_racks"),
            num_games: row.get("num_games"),
            previous_artifact_key: row.get("previous_artifact_key"),
        })
    }

    fn process_response(response: Self::Response) -> AppResult<Self::Record> {
        if response.racks.is_empty() {
            return Err(AppError::bad_request("leave result carried no rack occurrences"));
        }
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
    /// outside the claim transaction because it uploads to S3 and shells out to
    /// MAGPIE.
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
) -> AppResult<LeaveGenStep> {
    let completed = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM leave_generation_artifacts WHERE job_id = $1",
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

    let previous_artifact_key = if generation > 1 {
        sqlx::query_scalar::<_, String>(
            "SELECT artifact_key FROM leave_generation_artifacts
             WHERE job_id = $1 AND generation = $2",
        )
        .bind(job_id)
        .bind(generation - 1)
        .fetch_optional(&mut *conn)
        .await?
    } else {
        None
    };

    Ok(LeaveGenStep::Dispatch(LeaveRequest {
        lexicon: config.lexicon.clone(),
        variant: config.variant.clone(),
        generation,
        forced_racks: racks,
        previous_artifact_key,
        num_games: config.num_iterations,
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
    data_path: &Path,
) -> AppResult<i64> {
    let dist = LetterDistribution::load(data_path, &config.lexicon)?;
    let leaves = dist.enumerate_leaves(config.max_leave_size as usize);
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

/// Close out a generation: fold `leave_rack_progress` into a `rack,value` CSV,
/// convert it to a KLV with the `magpie` executable, store the artifact, and
/// seed the next generation's rack universe.
///
/// This runs once per generation rather than once per task, which is why
/// shelling out is the right call — an isolated subprocess cannot take the
/// backend down the way an in-process FFI crash could.
pub async fn run_transition(
    pool: &sqlx::PgPool,
    artifacts: &ArtifactStore,
    magpie_dir: &Path,
    magpie_bin: &Path,
    magpie_data_dir: &Path,
    data_path: &Path,
    job_id: Uuid,
    generation: i32,
    config: &LeaveConfig,
) -> AppResult<String> {
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

    // `magpie convert csv2klv` wants `rack,value` — a mean equity per rack —
    // which is a different shape from the `rack,count,mean` workers submit.
    let mut csv = String::new();
    for row in &rows {
        let rack: String = row.get("rack");
        let count: i64 = row.get("occurrence_count");
        let equity_sum: f64 = row.get("equity_sum");
        csv.push_str(&format!("{rack},{:.6}\n", equity_sum / count as f64));
    }

    // `magpie convert` addresses files by *data name*, not by path: it reads
    // `<data path>/lexica/<name>.csv` and writes the result to the same
    // relative location under the FIRST entry of the colon-separated `-path`
    // list. So the scratch directory has to be laid out like a MAGPIE data
    // directory and listed first; MAGPIE's own data directory comes second, to
    // supply the letter distribution, and is never written to.
    let dir = std::env::temp_dir().join(format!("birdtest-leavegen-{job_id}-{generation}"));
    let lexica_dir = dir.join("lexica");
    std::fs::create_dir_all(&lexica_dir).map_err(|e| {
        AppError::internal(format!("could not create {}: {e}", lexica_dir.display()))
    })?;

    let name = format!("birdtest-{job_id}-gen{generation}");
    let csv_path = lexica_dir.join(format!("{name}.csv"));
    let klv_path = lexica_dir.join(format!("{name}.klv2"));
    std::fs::write(&csv_path, csv)
        .map_err(|e| AppError::internal(format!("could not write leave CSV: {e}")))?;

    // birdtest's own data directory is on the search path too, so the letter
    // distribution resolves from the files mirrored into
    // `data/letterdistributions/`. Every entry is absolutised because the
    // command runs with a different working directory (see below).
    let absolute = |path: &Path| {
        std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    };
    let search_path = format!(
        "{}:{}:{}",
        absolute(&dir).display(),
        absolute(data_path).display(),
        absolute(magpie_data_dir).display()
    );

    let output = tokio::process::Command::new(magpie_bin)
        .arg("convert")
        .arg("csv2klv")
        .arg(&name)
        // `convert` can only infer the distribution when the data name *is* a
        // lexicon name; ours is a generated one, so it is passed explicitly.
        .arg(super::racks::letter_distribution_name(&config.lexicon))
        .arg("-path")
        .arg(&search_path)
        // MAGPIE loads its default board layout from `./data` while building
        // its config, before it has parsed `-path` — so the working directory
        // has to be the MAGPIE checkout regardless of what the search path says.
        .current_dir(magpie_dir)
        .output()
        .await
        .map_err(|e| AppError::internal(format!("could not run {}: {e}", magpie_bin.display())))?;

    // MAGPIE reports conversion failures on its error stack and still exits 0,
    // so the exit status alone proves nothing — whether the output file appeared
    // is the real check, and MAGPIE's own output is what explains a failure.
    let klv = std::fs::read(&klv_path).map_err(|e| {
        AppError::internal(format!(
            "magpie convert csv2klv produced no KLV at {} ({e}); magpie said: {}{}",
            klv_path.display(),
            String::from_utf8_lossy(&output.stdout).trim(),
            String::from_utf8_lossy(&output.stderr).trim(),
        ))
    })?;
    let key = format!("leaves/{job_id}/generation-{generation}.klv2");
    artifacts.put(&key, klv).await?;

    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO leave_generation_artifacts (job_id, generation, artifact_key)
         VALUES ($1, $2, $3)
         ON CONFLICT (job_id, generation) DO NOTHING",
    )
    .bind(job_id)
    .bind(generation)
    .bind(&key)
    .execute(&mut *tx)
    .await?;

    if generation < config.generation_count {
        seed_generation(&mut tx, job_id, generation + 1, config, data_path).await?;
    } else {
        sqlx::query("UPDATE jobs SET status = 'completed' WHERE id = $1")
            .bind(job_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;

    let _ = std::fs::remove_dir_all(&dir);
    Ok(key)
}
