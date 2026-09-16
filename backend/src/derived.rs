//! Wordmaps and rack info tables: what a job needs, and the reference copies
//! the server builds so a worker's can be checked.
//!
//! See MAGPIE_DEPENDENCY.md. A worker derives both files locally from data the
//! job pins, because a wordmap is 179 MB and a rack info table 1.9 GB and
//! neither can be shipped. Nothing checked them: a rack info table was refused
//! outright, and a wordmap was covered only by a sidecar naming the `.kwg` it
//! was built from -- which says a file was built from the right things and
//! still trusts the builder. A CSW24 wordmap built in December 2025 and one
//! built nine months later differ in 72,852,152 bytes with the same inputs and
//! the same format version, so trusting the builder is exactly the gap.
//!
//! So the server builds its own copy with a pinned MAGPIE, keeps the hash,
//! throws the file away, and sends the hash with the claim. The worker builds
//! its own and uses it only if the bytes agree.
//!
//! **Why a job waits for this.** A job whose derived files are not built yet is
//! not dispatched, the same way a leave-generation job waits for its universe
//! to be seeded. Dispatching without the hash would mean either sending no
//! `derived` entry -- so the worker falls back to the old unchecked behaviour,
//! quietly -- or sending an entry with nothing in it. Waiting is the only
//! option that cannot be mistaken for success.

use crate::artifacts::ArtifactStore;
use crate::error::{AppError, AppResult};
use crate::magpie::{Builders, Magpie, ScratchData};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool, Row};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// How long a builder holds a row before another builder may take it over.
///
/// Longer than the longest build (about three minutes for a rack info table on
/// one core) with room for a slow fetch of the inputs, and short enough that a
/// builder killed mid-build does not strand the job that is waiting on it for
/// an afternoon.
const LEASE: chrono::Duration = chrono::Duration::minutes(45);

/// How many times a row is retried before it is left failed for an admin.
///
/// A build is a pure function of its inputs, so a repeated failure is a
/// missing input or a broken binary, neither of which a fourth attempt fixes.
/// The attempt limit is what stops the builder task from spending every run on
/// the same doomed row while real requests queue behind it.
const MAX_ATTEMPTS: i32 = 3;

/// A derived file some job's tasks will load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedNeed {
    /// `wmp` or `rit`.
    pub role: String,
    /// The name the worker loads it under.
    pub name: String,
    pub kwg_id: Uuid,
    /// `None` for a wordmap.
    pub klv_id: Option<Uuid>,
    pub letterdist_id: Uuid,
}

/// The name a rack info table for this pair is loaded under.
///
/// A table stores precomputed leave values that move generation uses in place
/// of the loaded KLV, so it belongs to the pair and not to the lexicon.
/// MAGPIE's CLI finds a table by lexicon name alone, which is why a player
/// pinning NWL23 words and CSW21 leaves -- a pairing birdtest accepts on
/// purpose -- would have loaded `NWL23.rit` and ranked every full rack on
/// NWL23's leaves instead of the CSW21 leaves the job pinned. Naming the file
/// for both makes that impossible to express.
pub fn rack_info_table_name(lexicon: &str, leaves: &str) -> String {
    format!("{lexicon}.{leaves}")
}

/// The derived files a job's tasks will load, as a CTE.
///
/// Shared by the two callers rather than written twice: what a job needs is
/// the definition this whole module turns on, and two copies would be free to
/// disagree about, say, whether a player asking for a rack info table also
/// needs a wordmap -- which it does, because the table is built from one.
///
/// Reads the same rows `jobs::expected_data` reads, for the same reason: this
/// is a query over what the job and its players pin, not an inference about
/// what a worker might want.
const NEEDS_CTE: &str = "WITH players AS (
         SELECT unnest(ARRAY[player1_config_id, player2_config_id]) AS id
         FROM job_game_config WHERE job_id = $1
         UNION
         SELECT unnest(ARRAY[player1_config_id, player2_config_id])
         FROM job_game_pair_config WHERE job_id = $1
         UNION
         SELECT player_config_id FROM job_opening_rack_config WHERE job_id = $1
     ),
     wants AS (
         SELECT pc.kwg_id, pc.klv_id, pc.use_wordmap, pc.use_rit
         FROM player_configs pc JOIN players p ON p.id = pc.id
         UNION ALL
         -- Leave generation has one bot and no player_configs row, and never
         -- a rack info table: every generation plays with a different KLV,
         -- which is exactly what a table would cache.
         SELECT c.kwg_id, NULL::uuid, c.use_wordmap, false
         FROM job_leave_config c WHERE c.job_id = $1
     ),
     needs AS (
         -- A rack info table is built from the wordmap for its lexicon, so a
         -- player that asks for a table needs the wordmap built and checked
         -- whether or not it asked for one to play with.
         SELECT 'wmp' AS role, kwg.name AS name, w.kwg_id, NULL::uuid AS klv_id
         FROM wants w JOIN input_data kwg ON kwg.id = w.kwg_id
         WHERE w.use_wordmap OR w.use_rit
         UNION
         SELECT 'rit', kwg.name || '.' || klv.name, w.kwg_id, w.klv_id
         FROM wants w
         JOIN input_data kwg ON kwg.id = w.kwg_id
         JOIN input_data klv ON klv.id = w.klv_id
         WHERE w.use_rit
     )";

/// Every derived file the job's tasks will load, deduplicated.
pub async fn needs_for_job(conn: &mut PgConnection, job_id: Uuid) -> AppResult<Vec<DerivedNeed>> {
    // The distribution a derived file is built against belongs to the job, not
    // to the player: two jobs on one lexicon under different distributions
    // build different files, and a worker that inferred one from the lexicon's
    // name would build a third.
    let letterdist_id: Option<Uuid> =
        sqlx::query_scalar("SELECT letterdist_id FROM jobs WHERE id = $1")
            .bind(job_id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some(letterdist_id) = letterdist_id else {
        return Ok(Vec::new());
    };

    let rows = sqlx::query(&format!(
        "{NEEDS_CTE} SELECT role, name, kwg_id, klv_id FROM needs ORDER BY role, name"
    ))
    .bind(job_id)
    .fetch_all(&mut *conn)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| DerivedNeed {
            role: row.get("role"),
            name: row.get("name"),
            kwg_id: row.get("kwg_id"),
            klv_id: row.get("klv_id"),
            letterdist_id,
        })
        .collect())
}

/// Queues a build for everything the job needs that is not already built.
///
/// Idempotent, and called both when a job is created and when it is activated:
/// a table takes minutes, so the earlier the request lands the less an admin
/// waits, and a job created before a MAGPIE upgrade needs its files again
/// under the new builder.
pub async fn request_for_job(
    conn: &mut PgConnection,
    job_id: Uuid,
    builders: &Builders,
) -> AppResult<usize> {
    let needs = needs_for_job(conn, job_id).await?;
    let mut requested = 0;
    for need in &needs {
        let builder = builders.for_role(&need.role)?;
        // DO NOTHING rather than an upsert: a row that already exists is
        // either built (nothing to do) or queued (someone is already doing
        // it), and a failed one is deliberately left failed so a broken input
        // does not silently re-enter the queue on every activation. An admin
        // retries it explicitly.
        let inserted = sqlx::query(
            "INSERT INTO derived_data
                 (role, name, builder, kwg_id, klv_id, letterdist_id)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT DO NOTHING",
        )
        .bind(&need.role)
        .bind(&need.name)
        .bind(&builder)
        .bind(need.kwg_id)
        .bind(need.klv_id)
        .bind(need.letterdist_id)
        .execute(&mut *conn)
        .await?
        .rows_affected();
        requested += inserted as usize;
    }
    Ok(requested)
}

/// One derived file as a claim states it.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ExpectedDerived {
    pub role: String,
    pub name: String,
    pub sha256: String,
    pub bytes: i64,
    /// The builder that produced this hash, e.g. `wmp-1`.
    pub builder: String,
    /// The instruction-set target the building MAGPIE was compiled for.
    ///
    /// Diagnostic, not a gate. The worker does not refuse work over it: on
    /// x86-64 with GCC 10, `-march=native` and `-march=nehalem` produce
    /// byte-identical wordmaps and rack info tables, so refusing on a
    /// difference here would lock out every contributor who builds from source
    /// in exchange for nothing. If that ever stops holding, a worker builds the
    /// file, finds the hash differs, and declines with both -- which is the
    /// outcome that makes it visible instead of assumed.
    pub build_target: String,
}

/// What a job's derived files look like right now.
pub struct DerivedStatus {
    /// Built and ready to state on a claim.
    pub ready: Vec<ExpectedDerived>,
    /// Still queued or building. A job with any of these is not dispatched.
    pub pending: Vec<String>,
    /// Gave up. A job with any of these is not dispatched either, and an admin
    /// has something to look at.
    pub failed: Vec<String>,
}

impl DerivedStatus {
    /// Whether this job may be handed out.
    pub fn dispatchable(&self) -> bool {
        self.pending.is_empty() && self.failed.is_empty()
    }
}

/// The derived files a claim for this job should state, and what is missing.
///
/// One query, not one per file. This runs on every claim that considers this
/// job, before the dispatch lock is taken, so each round trip here is latency
/// on every worker's request -- the same reason `expected_data` is one query
/// rather than three.
pub async fn status_for_job(
    conn: &mut PgConnection,
    job_id: Uuid,
    builders: &Builders,
) -> AppResult<DerivedStatus> {
    let letterdist_id: Option<Uuid> =
        sqlx::query_scalar("SELECT letterdist_id FROM jobs WHERE id = $1")
            .bind(job_id)
            .fetch_optional(&mut *conn)
            .await?;
    let mut status = DerivedStatus { ready: Vec::new(), pending: Vec::new(), failed: Vec::new() };
    let Some(letterdist_id) = letterdist_id else {
        return Ok(status);
    };

    let rows = sqlx::query(&format!(
        "{NEEDS_CTE}
         SELECT n.role, n.name, d.state, d.sha256, d.bytes, d.build_target
         FROM needs n
         LEFT JOIN derived_data d
           ON d.role = n.role AND d.name = n.name
          AND d.builder = CASE n.role WHEN 'wmp' THEN $2 ELSE $3 END
          AND d.kwg_id = n.kwg_id
          AND d.klv_id IS NOT DISTINCT FROM n.klv_id
          AND d.letterdist_id = $4
         ORDER BY n.role, n.name"
    ))
    .bind(job_id)
    .bind(builders.wmp())
    .bind(builders.rit())
    .bind(letterdist_id)
    .fetch_all(&mut *conn)
    .await?;

    for row in rows {
        let role: String = row.get("role");
        let name: String = row.get("name");
        let label = format!("{role} {name}");
        // NULL state is a LEFT JOIN miss: nothing has requested this file
        // under this builder yet, which from a worker's point of view is the
        // same wait as a queued one. The caller requests it; this reports.
        match row.get::<Option<String>, _>("state").as_deref() {
            Some("built") => status.ready.push(ExpectedDerived {
                builder: builders.for_role(&role)?,
                role,
                name,
                sha256: row.get("sha256"),
                bytes: row.get("bytes"),
                build_target: builders.build_target.clone(),
            }),
            Some("failed") => status.failed.push(label),
            _ => status.pending.push(label),
        }
    }
    Ok(status)
}

/// The built hashes of every job this process has found dispatchable, by job.
///
/// `status_for_job` runs on every claim, for every candidate job in the
/// worker's tier, before that job's dispatch lock is taken -- a join over the
/// job's config, its players, `input_data` and `derived_data` on the path a
/// worker waits on to get its next task. The answer for a job that is
/// dispatchable never changes for the life of the process, so it is asked
/// once:
///
/// - what a job needs is fixed when it is created (player configs are
///   immutable and the job's own config has no update endpoint), so the set of
///   `derived_data` rows the query looks up cannot grow or shrink;
/// - a row only ever moves toward `built` -- the builder task writes `built`,
///   and an admin retry touches `failed` rows only -- and nothing deletes one
///   (`input_data` refuses a delete while a derived row references it);
/// - the builder identity the query matches on is a constant of the running
///   binary, so a deployment with a different MAGPIE starts with an empty
///   cache and asks again.
///
/// Only a dispatchable answer is remembered. A job still waiting on a build
/// is asked about on every claim, which is what lets it be dispatched the
/// moment its last file is built.
#[derive(Clone, Default)]
pub struct DerivedCache(Arc<Mutex<HashMap<Uuid, Arc<Vec<ExpectedDerived>>>>>);

impl DerivedCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// The built hashes for `job_id`, if this process has already found the
    /// job dispatchable.
    pub fn get(&self, job_id: Uuid) -> Option<Arc<Vec<ExpectedDerived>>> {
        self.0.lock().expect("derived cache poisoned").get(&job_id).cloned()
    }

    /// Remember a dispatchable job's hashes.
    pub fn remember(&self, job_id: Uuid, ready: Vec<ExpectedDerived>) -> Arc<Vec<ExpectedDerived>> {
        let ready = Arc::new(ready);
        self.0.lock().expect("derived cache poisoned").insert(job_id, ready.clone());
        ready
    }

    /// Drop a job's entry. Only a deleted job has one that is no longer
    /// wanted; nothing else can make a remembered answer wrong.
    pub fn forget(&self, job_id: Uuid) {
        self.0.lock().expect("derived cache poisoned").remove(&job_id);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.0.lock().expect("derived cache poisoned").len()
    }
}

/// The hashes a claim for `job_id` states, or `None` while the job is waiting
/// on a build (or a failed one). Answered from [`DerivedCache`] after the
/// first dispatchable answer; see there for why that is sound.
pub async fn ready_for_job(
    conn: &mut PgConnection,
    job_id: Uuid,
    builders: &Builders,
    cache: &DerivedCache,
) -> AppResult<Option<Arc<Vec<ExpectedDerived>>>> {
    if let Some(ready) = cache.get(job_id) {
        return Ok(Some(ready));
    }
    let status = status_for_job(conn, job_id, builders).await?;
    if !status.dispatchable() {
        // Logged at debug: a table takes minutes to build, and every worker
        // asking during those minutes would otherwise produce a line each.
        // `GET /api/admin/derived-data` is where an admin looks.
        tracing::debug!(
            %job_id, pending = ?status.pending, failed = ?status.failed,
            "job is waiting on a derived file"
        );
        return Ok(None);
    }
    Ok(Some(cache.remember(job_id, status.ready)))
}

/// A row the builder task has taken.
struct Lease {
    role: String,
    name: String,
    kwg_id: Uuid,
    klv_id: Option<Uuid>,
    letterdist_id: Uuid,
    builder: String,
}

/// Takes the oldest row that needs building, or `None` if the queue is empty.
///
/// `FOR UPDATE SKIP LOCKED` plus the lease is what keeps two builder tasks off
/// the same row: the lock holds for the moment it takes to mark the row, and
/// the lease holds for the minutes it takes to build it.
async fn take_next(pool: &PgPool, builders: &Builders) -> AppResult<Option<Lease>> {
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        "SELECT role, name, builder, kwg_id, klv_id, letterdist_id
         FROM derived_data
         WHERE (state = 'pending'
                OR (state = 'building' AND leased_until < now()))
           AND attempts < $1
         ORDER BY requested_at
         FOR UPDATE SKIP LOCKED
         LIMIT 1",
    )
    .bind(MAX_ATTEMPTS)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else {
        tx.rollback().await?;
        return Ok(None);
    };
    let lease = Lease {
        role: row.get("role"),
        name: row.get("name"),
        kwg_id: row.get("kwg_id"),
        klv_id: row.get("klv_id"),
        letterdist_id: row.get("letterdist_id"),
        builder: row.get("builder"),
    };

    // A row queued under a builder this binary is not is left alone rather
    // than built wrongly: the hash would be recorded against a builder that
    // did not produce it, which is the one thing this whole design exists to
    // prevent. It waits for a deployment that has that builder, or for an
    // admin to notice.
    let ours = builders.for_role(&lease.role)?;
    if ours != lease.builder {
        tx.rollback().await?;
        tracing::warn!(
            role = %lease.role, name = %lease.name,
            wanted = %lease.builder, have = %ours,
            "a derived file is queued for a builder this MAGPIE does not have"
        );
        return Ok(None);
    }

    sqlx::query(
        "UPDATE derived_data
         SET state = 'building', leased_until = $1, attempts = attempts + 1,
             error = NULL
         WHERE role = $2 AND name = $3 AND builder = $4
           AND kwg_id = $5 AND klv_id IS NOT DISTINCT FROM $6
           AND letterdist_id = $7",
    )
    .bind(chrono::Utc::now() + LEASE)
    .bind(&lease.role)
    .bind(&lease.name)
    .bind(&lease.builder)
    .bind(lease.kwg_id)
    .bind(lease.klv_id)
    .bind(lease.letterdist_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Some(lease))
}

/// The bytes of an `input_data` row, from the object store.
async fn input_bytes(
    pool: &PgPool,
    artifacts: &ArtifactStore,
    id: Uuid,
) -> AppResult<(String, Vec<u8>)> {
    let row = sqlx::query("SELECT name, role, path, content, object_key FROM input_data WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await?;
    let name: String = row.get("name");
    // Letter distributions and layouts have always been kept in the row: the
    // server reads them itself, and they are hundreds of bytes.
    if let Some(content) = row.get::<Option<Vec<u8>>, _>("content") {
        return Ok((name, content));
    }
    let Some(key) = row.get::<Option<String>, _>("object_key") else {
        let path: String = row.get("path");
        return Err(AppError::internal(format!(
            "{path} was imported before the server stored lexicon bytes, so it cannot be built \
             from. Re-import that tarball -- the import adds no rows for files whose bytes have \
             not changed, and will fill in the missing ones."
        )));
    };
    Ok((name, artifacts.get(&key).await?))
}

/// Builds one derived file and records its hash. `Ok(true)` means a row was
/// taken; the caller loops until it gets `Ok(false)`.
pub async fn build_next(
    pool: &PgPool,
    artifacts: &ArtifactStore,
    magpie: &Magpie,
    builders: &Builders,
) -> AppResult<bool> {
    let Some(lease) = take_next(pool, builders).await? else {
        return Ok(false);
    };
    tracing::info!(role = %lease.role, name = %lease.name, "building a derived file");
    let started = std::time::Instant::now();

    match build(pool, artifacts, magpie, &lease).await {
        Ok((sha256, bytes)) => {
            sqlx::query(
                "UPDATE derived_data
                 SET state = 'built', sha256 = $1, bytes = $2, build_target = $3,
                     built_at = now(), leased_until = NULL, error = NULL
                 WHERE role = $4 AND name = $5 AND builder = $6
                   AND kwg_id = $7 AND klv_id IS NOT DISTINCT FROM $8
                   AND letterdist_id = $9",
            )
            .bind(&sha256)
            .bind(bytes)
            .bind(&builders.build_target)
            .bind(&lease.role)
            .bind(&lease.name)
            .bind(&lease.builder)
            .bind(lease.kwg_id)
            .bind(lease.klv_id)
            .bind(lease.letterdist_id)
            .execute(pool)
            .await?;
            tracing::info!(
                role = %lease.role, name = %lease.name, %sha256, bytes,
                seconds = started.elapsed().as_secs(),
                "built a derived file"
            );
        }
        Err(err) => {
            // Left 'building' with a lapsed lease would be retried by the next
            // run; 'pending' says the same thing and reads correctly in the
            // admin view. The attempt counter, not the state, is what stops it
            // eventually.
            sqlx::query(
                "UPDATE derived_data
                 SET state = CASE WHEN attempts >= $1 THEN 'failed' ELSE 'pending' END,
                     leased_until = NULL, error = $2
                 WHERE role = $3 AND name = $4 AND builder = $5
                   AND kwg_id = $6 AND klv_id IS NOT DISTINCT FROM $7
                   AND letterdist_id = $8",
            )
            .bind(MAX_ATTEMPTS)
            .bind(&err.message)
            .bind(&lease.role)
            .bind(&lease.name)
            .bind(&lease.builder)
            .bind(lease.kwg_id)
            .bind(lease.klv_id)
            .bind(lease.letterdist_id)
            .execute(pool)
            .await?;
            tracing::error!(
                role = %lease.role, name = %lease.name, error = %err.message,
                "could not build a derived file"
            );
        }
    }
    Ok(true)
}

/// Writes the inputs into a scratch directory, runs MAGPIE, and hashes what it
/// wrote.
async fn build(
    pool: &PgPool,
    artifacts: &ArtifactStore,
    magpie: &Magpie,
    lease: &Lease,
) -> AppResult<(String, i64)> {
    let scratch = ScratchData::empty().await?;
    let (lexicon, kwg) = input_bytes(pool, artifacts, lease.kwg_id).await?;
    let (letterdist, ld) = input_bytes(pool, artifacts, lease.letterdist_id).await?;
    scratch.write("lexica", &lexicon, ".kwg", &kwg).await?;
    scratch.write("letterdistributions", &letterdist, ".csv", &ld).await?;

    // A rack info table is built from a wordmap, so both roles build one. It
    // is thrown away with the directory either way.
    magpie.convert(&scratch, "dawg2wordmap", &lexicon, &letterdist).await?;

    let built = match lease.role.as_str() {
        "wmp" => scratch.lexicon_path(&lexicon, ".wmp"),
        "rit" => {
            let klv_id = lease.klv_id.ok_or_else(|| {
                AppError::internal("a rack info table row has no leaves to build from")
            })?;
            let (leaves, klv) = input_bytes(pool, artifacts, klv_id).await?;
            scratch.write("lexica", &leaves, ".klv2", &klv).await?;
            // The table is named for the pair, and its inputs are named for
            // themselves: `klvwmp2rit` takes the KLV's and the wordmap's names
            // separately precisely so the output does not have to borrow one
            // of them.
            magpie
                .convert_rack_info_table(&scratch, &lease.name, &letterdist, &leaves, &lexicon)
                .await?;
            scratch.lexicon_path(&lease.name, ".rit")
        }
        other => return Err(AppError::internal(format!("unknown derived role {other:?}"))),
    };

    // MAGPIE reports a failed conversion on its error stack and can still
    // leave no file behind, so the output's existence and its hash are the
    // real check -- the same rule the worker applies to its own copy.
    hash_file(&built).await
}

/// The SHA-256 and size of a file, read in chunks.
///
/// Chunked because a rack info table is 1.9 GB and this runs in a task sized
/// for MAGPIE's own 2.4 GB peak; reading the file into memory on top of that
/// is the difference between a build and an OOM kill.
async fn hash_file(path: &std::path::Path) -> AppResult<(String, i64)> {
    use tokio::io::AsyncReadExt;

    let mut file = tokio::fs::File::open(path).await.map_err(|e| {
        AppError::internal(format!(
            "MAGPIE reported no error but wrote no {}: {e}",
            path.display()
        ))
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 8 * 1024 * 1024];
    let mut total: i64 = 0;
    loop {
        let read = file.read(&mut buffer).await.map_err(|e| {
            AppError::internal(format!("could not read {}: {e}", path.display()))
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        total += read as i64;
    }
    Ok((hex::encode(hasher.finalize()), total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_table_is_named_for_its_pair() {
        // The case the name exists for: NWL23 words with CSW21 leaves is a
        // configuration birdtest accepts on purpose, and `NWL23.rit` would
        // have been CSW21's table or NWL23's depending on who built it first.
        assert_eq!(rack_info_table_name("NWL23", "CSW21"), "NWL23.CSW21");
        assert_eq!(
            rack_info_table_name("CSW24", "CSW_quackle_leaves"),
            "CSW24.CSW_quackle_leaves"
        );
        assert_ne!(
            rack_info_table_name("CSW24", "CSW_quackle_leaves"),
            rack_info_table_name("CSW24", "CSW24")
        );
    }

    #[test]
    fn a_job_waits_for_anything_not_built() {
        let ready = ExpectedDerived {
            role: "wmp".into(),
            name: "NWL23".into(),
            sha256: "0".repeat(64),
            bytes: 1,
            builder: "wmp-1".into(),
            build_target: "nehalem".into(),
        };
        assert!(DerivedStatus { ready: vec![], pending: vec![], failed: vec![] }.dispatchable());
        assert!(DerivedStatus {
            ready: vec![ready.clone()],
            pending: vec![],
            failed: vec![]
        }
        .dispatchable());
        assert!(!DerivedStatus {
            ready: vec![ready.clone()],
            pending: vec!["rit NWL23.CSW21".into()],
            failed: vec![]
        }
        .dispatchable());
        // A failed build is not "dispatch without it": the worker would fall
        // back to whatever is on its disk, unchecked.
        assert!(!DerivedStatus {
            ready: vec![ready],
            pending: vec![],
            failed: vec!["rit NWL23.CSW21".into()]
        }
        .dispatchable());
    }

    #[test]
    fn the_cache_remembers_only_what_it_is_told_and_forgets_on_request() {
        let cache = DerivedCache::new();
        let job = Uuid::new_v4();
        assert!(cache.get(job).is_none());
        let ready = cache.remember(job, vec![]);
        assert!(ready.is_empty());
        assert!(cache.get(job).is_some(), "a dispatchable answer is kept");
        assert_eq!(cache.len(), 1);
        cache.forget(job);
        assert!(cache.get(job).is_none());
        assert_eq!(cache.len(), 0);
    }
}
