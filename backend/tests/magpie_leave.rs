//! Tier 6: leave generation's KLVs, built by a real MAGPIE and stored in a real
//! object store (`I-LEAVE-4`, `-6`, `-8`, `-9`, the conversion half of
//! `I-DATA-2`, and `M-8`'s misnamed-rack case).
//!
//! **Opt-in, then fail loudly**, as in `magpie_smoke.rs`: every test is
//! `#[ignore]` because it needs a MAGPIE build (`MAGPIE_BIN`) and a MinIO
//! (`TEST_S3_ENDPOINT`) on top of the tier-2 Postgres, and when asked for, a
//! missing MAGPIE fails rather than skips.
//!
//!     cargo nextest run --locked --test magpie_leave --run-ignored all
//!
//! Every MAGPIE run here happens in a `ScratchData` directory, and every test
//! points `MAGPIE_SCRATCH_DIR` at a directory of its own and asserts at the
//! end that nothing was left in it. That relies on nextest's one process per
//! test: the variable is process-wide.

mod common;

use axum::http::StatusCode;
use birdtest::jobs::leave_gen;
use birdtest::jobs::racks::{LetterDistribution, RackIndex};
use birdtest::magpie::{Builders, Magpie, ScratchData};
use birdtest::models::job::LeaveConfig;
use birdtest::state::AppState;
use common::*;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

const TESTDIST: &[u8] = include_bytes!("../src/jobs/testdata/testdist.csv");

fn magpie_bin() -> String {
    let binary = std::env::var("MAGPIE_BIN").unwrap_or_else(|_| "../MAGPIE/bin/magpie".to_string());
    assert!(
        std::path::Path::new(&binary).is_file(),
        "no MAGPIE at {binary}. Build one (`make magpie BUILD=portable_release` in a MAGPIE \
         checkout) and set MAGPIE_BIN. These tests fail rather than skip on purpose: a green \
         run must not mean nothing was exercised."
    );
    binary
}

/// This test's own `MAGPIE_SCRATCH_DIR`, so what MAGPIE runs leave behind can
/// be counted. Removed on drop whatever happened; `assert_clean` is the check.
struct ScratchRoot(PathBuf);

impl ScratchRoot {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("birdtest-leave-scratch-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("MAGPIE_SCRATCH_DIR", &dir);
        ScratchRoot(dir)
    }

    fn assert_clean(&self) {
        let left: Vec<_> = std::fs::read_dir(&self.0)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert!(
            left.is_empty(),
            "MAGPIE scratch directories left behind: {left:?}"
        );
    }
}

impl Drop for ScratchRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A state whose object store is a fresh MinIO bucket and whose MAGPIE is the
/// real binary, with the builder versions read out of it as the server does at
/// startup.
async fn real_state(db: &TestDb) -> (AppState, TestBucket) {
    let (with_store, bucket) = db.state_with_object_store().await;
    let mut cfg = (*with_store.cfg).clone();
    cfg.magpie_bin = magpie_bin();
    let mut state = db.state_with(cfg).await;
    state.builders = Arc::new(state.magpie.builders().await.expect("magpie builders"));
    (state, bucket)
}

/// Marks the job's derived files built under `builders` -- `derived_ready`,
/// but for the builders the real binary reports rather than the harness's.
async fn derived_ready_for(db: &TestDb, job: Uuid, builders: &Builders) {
    let mut conn = db.pool.acquire().await.unwrap();
    let needs = birdtest::derived::needs_for_job(&mut conn, job)
        .await
        .unwrap();
    for (i, need) in needs.iter().enumerate() {
        sqlx::query(
            "INSERT INTO derived_data
                 (role, name, builder, kwg_id, klv_id, letterdist_id,
                  state, sha256, bytes, build_target, built_at)
             VALUES ($1,$2,$3,$4,$5,$6,'built',$7,1,$8,now())
             ON CONFLICT DO NOTHING",
        )
        .bind(&need.role)
        .bind(&need.name)
        .bind(builders.for_role(&need.role).unwrap())
        .bind(need.kwg_id)
        .bind(need.klv_id)
        .bind(need.letterdist_id)
        .bind(format!("{i:064x}"))
        .bind(&builders.build_target)
        .execute(&mut *conn)
        .await
        .unwrap();
    }
}

/// A letter distribution row named `name` with exactly `content`.
async fn letterdist(db: &TestDb, name: &str, content: &[u8]) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO input_data (path, role, name, sha256, bytes, tarball_date, content)
         VALUES ($1, 'letterdist', $2, $3, $4, '20251004', $5) RETURNING id",
    )
    .bind(format!("letterdistributions/{name}.csv"))
    .bind(name)
    .bind(hex::encode(Sha256::digest(content)))
    .bind(content.len() as i64)
    .bind(content)
    .fetch_one(&db.pool)
    .await
    .unwrap()
}

/// A leave-generation job assembled with SQL -- pinning `ld` if given, the
/// harness's test distribution otherwise -- whose generation-0 KLV is built by
/// MAGPIE and uploaded exactly as job creation does it, and whose generation-1
/// universe is seeded.
async fn leave_job(
    db: &TestDb,
    state: &AppState,
    target: i32,
    generation_count: i32,
    ld: Option<Uuid>,
) -> (Uuid, LetterDistribution) {
    let admin = db
        .user(&format!("admin{}", Uuid::new_v4().simple()), true)
        .await;
    let job = db.bare_job("leave_generation", 1, admin).await;
    if let Some(ld) = ld {
        sqlx::query("UPDATE jobs SET letterdist_id = $2 WHERE id = $1")
            .bind(job)
            .bind(ld)
            .execute(&db.pool)
            .await
            .unwrap();
    }
    let kwg = db.input_data("kwg", "NWL23").await;
    sqlx::query(
        "INSERT INTO job_leave_config
             (job_id, kwg_id, num_iterations, generation_count, target_rack_count, racks_per_task)
         VALUES ($1, $2, 100, $3, $4, 2)",
    )
    .bind(job)
    .bind(kwg)
    .bind(generation_count)
    .bind(target)
    .execute(&db.pool)
    .await
    .unwrap();
    derived_ready_for(db, job, &state.builders).await;

    let mut conn = db.pool.acquire().await.unwrap();
    let data = birdtest::jobs::load_job_data(&mut conn, job).await.unwrap();
    leave_gen::seed_zero_generation(
        &state.pool,
        &state.artifacts,
        &state.magpie,
        &state.builders,
        job,
        &data.letterdist,
    )
    .await
    .expect("the zeroed KLV");
    leave_gen::seed_generation(&mut conn, job, 1, &data.letterdist)
        .await
        .unwrap();
    (job, data.letterdist)
}

async fn config(db: &TestDb, job: Uuid) -> LeaveConfig {
    sqlx::query_as::<_, LeaveConfig>("SELECT * FROM job_leave_config WHERE job_id = $1")
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

/// The owner row a claim writes when it finds a generation complete, so that
/// `run_transition` can be called directly as the spawned task calls it.
async fn own_transition(db: &TestDb, job: Uuid, generation: i32) {
    sqlx::query("INSERT INTO leave_generation_transitions (job_id, generation) VALUES ($1, $2)")
        .bind(job)
        .bind(generation)
        .execute(&db.pool)
        .await
        .unwrap();
}

/// The recorded artifact row of a generation: key, digest, builder.
async fn artifact(db: &TestDb, job: Uuid, generation: i32) -> Option<(String, String, String)> {
    sqlx::query_as(
        "SELECT artifact_key, sha256, builder FROM leave_generation_artifacts
         WHERE job_id = $1 AND generation = $2",
    )
    .bind(job)
    .bind(generation)
    .fetch_optional(&db.pool)
    .await
    .unwrap()
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// `klv` loaded back through MAGPIE (`convert klv2csv`) against
/// `distribution`: every leave it holds and its value.
async fn klv_values(
    magpie: &Magpie,
    distribution: &LetterDistribution,
    klv: &[u8],
) -> Vec<(String, f64)> {
    let scratch = ScratchData::empty().await.unwrap();
    scratch
        .write(
            "letterdistributions",
            &distribution.name,
            ".csv",
            &distribution.bytes,
        )
        .await
        .unwrap();
    scratch
        .write("lexica", "loaded", ".klv2", klv)
        .await
        .unwrap();
    magpie
        .convert(&scratch, "klv2csv", "loaded", &distribution.name)
        .await
        .expect("MAGPIE loads the KLV");
    let csv = tokio::fs::read_to_string(scratch.lexicon_path("loaded", ".csv"))
        .await
        .unwrap();
    csv.lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let (leave, value) = line.rsplit_once(',').unwrap();
            // Letters in canonical order, whatever order MAGPIE writes them.
            let mut leave: Vec<char> = leave.chars().collect();
            leave.sort();
            (leave.into_iter().collect(), value.parse::<f64>().unwrap())
        })
        .collect()
}

/// The KLV MAGPIE builds from `rows` (`rack -> (count, equity_sum)`), written
/// in the distribution's own enumeration order -- not the database's -- in the
/// format the server streams.
async fn klv_from(
    magpie: &Magpie,
    distribution: &LetterDistribution,
    rows: &HashMap<String, (i64, f64)>,
) -> Vec<u8> {
    let index = RackIndex::new(distribution, 7).unwrap();
    let mut csv = String::new();
    for i in 0..index.total() {
        let rack = index.rack_at(i).unwrap();
        let (count, sum) = rows[&rack];
        csv.push_str(&format!("{rack},{count},{sum:.10}\n"));
    }
    let scratch = ScratchData::empty().await.unwrap();
    scratch
        .write(
            "letterdistributions",
            &distribution.name,
            ".csv",
            &distribution.bytes,
        )
        .await
        .unwrap();
    scratch
        .write("lexica", "expected", ".csv", csv.as_bytes())
        .await
        .unwrap();
    magpie
        .convert(&scratch, "rackequity2klv", "expected", &distribution.name)
        .await
        .expect("rackequity2klv");
    tokio::fs::read(scratch.lexicon_path("expected", ".klv2"))
        .await
        .unwrap()
}

/// Gives every rack of `generation` `count` occurrences and an equity sum of
/// its own (a quarter-integer, so the sums are exact), and returns what it set.
async fn fill_generation(
    db: &TestDb,
    job: Uuid,
    generation: i32,
    count: i64,
) -> HashMap<String, (i64, f64)> {
    let racks: Vec<String> = sqlx::query_scalar(
        "SELECT rack FROM leave_rack_progress WHERE job_id = $1 AND generation = $2 ORDER BY rack",
    )
    .bind(job)
    .bind(generation)
    .fetch_all(&db.pool)
    .await
    .unwrap();
    let mut rows = HashMap::new();
    for (i, rack) in racks.iter().enumerate() {
        let sum = (i % 17) as f64 * 0.75 - 5.25;
        sqlx::query(
            "UPDATE leave_rack_progress SET occurrence_count = $4, equity_sum = $5
             WHERE job_id = $1 AND generation = $2 AND rack = $3",
        )
        .bind(job)
        .bind(generation)
        .bind(rack)
        .bind(count)
        .bind(sum)
        .execute(&db.pool)
        .await
        .unwrap();
        rows.insert(rack.clone(), (count, sum));
    }
    rows
}

/// Polls `condition` until it holds or a generous deadline passes, for a
/// transition or a seeding a claim leaves running on its own task.
async fn wait_for<F, Fut>(mut condition: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..2400 {
        if condition().await {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    false
}

async fn claim(app: &axum::Router) -> (StatusCode, serde_json::Value) {
    send(
        app,
        post_json("/api/worker/task", &[], claim_body("1.0.0", &[])),
    )
    .await
}

/// I-LEAVE-6: a leave job created through the admin API has its generation-0
/// KLV from the moment creation answers -- recorded under its key with the
/// digest of the bytes in the object store and the builder the binary reports
/// -- and that KLV, loaded back through MAGPIE, holds every leave of the
/// distribution at exactly zero.
#[tokio::test]
#[ignore = "needs a real MAGPIE (MAGPIE_BIN) and MinIO (TEST_S3_ENDPOINT); tier 6"]
async fn generation_zeros_klv_exists_at_creation_and_is_worth_exactly_nothing() {
    let scratch_root = ScratchRoot::new();
    let db = TestDb::new().await;
    let (state, bucket) = real_state(&db).await;
    let admin = db.user("root", true).await;
    let headers = admin_headers(&state.cfg, admin);
    let (ld, layout) = (
        db.input_data("letterdist", "english").await,
        db.input_data("layout", "standard15").await,
    );
    let kwg = db.input_data("kwg", "NWL23").await;
    let (magpie, builders) = (state.magpie.clone(), state.builders.clone());
    let app = birdtest::app(state.clone());

    let mut request =
        axum::http::Request::post("/api/admin/jobs").header("content-type", "application/json");
    for (name, value) in &headers {
        request = request.header(name.as_str(), value.as_str());
    }
    let body = json!({
        "job_type": "leave_generation", "variant": "classic",
        "letterdist_id": ld, "layout_id": layout, "kwg_id": kwg,
        "num_iterations": 100, "generation_count": 2, "target_rack_count": 10,
        "racks_per_task": 2, "use_wordmap": false,
    });
    let (status, created) = send(
        &app,
        request
            .body(axum::body::Body::from(body.to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let job: Uuid = created["job"]["id"].as_str().unwrap().parse().unwrap();

    let (key, recorded, builder) = artifact(&db, job, 0)
        .await
        .expect("a generation-0 artifact row");
    assert_eq!(key, leave_gen::artifact_key(job, 0));
    assert_eq!(builder, builders.klv(), "the builder the binary reports");
    assert_eq!(bucket.keys().await, vec![key.clone()]);
    let bytes = state.artifacts.get(&key).await.unwrap();
    assert_eq!(
        sha256(&bytes),
        recorded,
        "the digest is of the bytes stored"
    );

    let distribution = LetterDistribution::parse(TESTDIST, "english").unwrap();
    let values = klv_values(&magpie, &distribution, &bytes).await;
    let mut leaves: Vec<String> = values.iter().map(|(leave, _)| leave.clone()).collect();
    leaves.sort();
    let mut expected = distribution.enumerate_leaves(6);
    expected.sort();
    assert_eq!(leaves, expected, "every leave of one to six tiles, once");
    assert!(
        values.iter().all(|(_, value)| *value == 0.0),
        "a leave worth something: {values:?}"
    );
    assert_eq!(values.iter().map(|(_, value)| value).sum::<f64>(), 0.0);

    drop((app, state));
    scratch_root.assert_clean();
}

/// I-LEAVE-4: a transition drains what is staged into the generation's rows,
/// has MAGPIE fold them into a KLV, uploads it, records its digest and the
/// builder, and marks the generation complete -- and the bytes are exactly the
/// KLV MAGPIE builds from the totals this test set, results still staged at
/// the start included. (Also the accepting half of `M-8`: the CSV the server
/// streams is one MAGPIE takes.)
#[tokio::test]
#[ignore = "needs a real MAGPIE (MAGPIE_BIN) and MinIO (TEST_S3_ENDPOINT); tier 6"]
async fn a_transition_folds_the_generation_into_the_klv_it_uploads_and_closes_it() {
    let scratch_root = ScratchRoot::new();
    let db = TestDb::new().await;
    let (state, bucket) = real_state(&db).await;
    let (job, distribution) = leave_job(&db, &state, 3, 2, None).await;
    let mut rows = fill_generation(&db, job, 1, 3).await;

    // A result accepted but not yet merged, as a takeover can find one.
    let staged: Vec<String> = rows.keys().take(2).cloned().collect();
    sqlx::query(
        "INSERT INTO leave_rack_staging (job_id, generation, task_id, racks, counts, equity_sums)
         VALUES ($1, 1, gen_random_uuid(), $2, $3, $4)",
    )
    .bind(job)
    .bind(&staged)
    .bind(vec![2i64, 1])
    .bind(vec![5.0f64, -1.5])
    .execute(&db.pool)
    .await
    .unwrap();
    for (rack, (count, sum)) in staged.iter().zip([(2, 5.0), (1, -1.5)]) {
        let row = rows.get_mut(rack).unwrap();
        *row = (row.0 + count, row.1 + sum);
    }

    own_transition(&db, job, 1).await;
    let key = leave_gen::run_transition(&state, job, 1, &config(&db, job).await, &distribution)
        .await
        .expect("the transition");
    assert_eq!(key, leave_gen::artifact_key(job, 1));

    let expected = klv_from(&state.magpie, &distribution, &rows).await;
    let (recorded_key, recorded, builder) = artifact(&db, job, 1).await.expect("an artifact row");
    assert_eq!(recorded_key, key);
    assert_eq!(builder, state.builders.klv());
    let stored = state.artifacts.get(&key).await.unwrap();
    assert_eq!(
        sha256(&stored),
        recorded,
        "the digest is of the bytes uploaded"
    );
    assert_eq!(
        recorded,
        sha256(&expected),
        "the KLV of exactly these totals, staged ones included"
    );
    let mut keys = bucket.keys().await;
    keys.sort();
    assert_eq!(keys, vec![leave_gen::artifact_key(job, 0), key.clone()]);

    // Folded, not zeroed: the leaves carry values.
    let values = klv_values(&state.magpie, &distribution, &stored).await;
    assert!(
        values.iter().any(|(_, value)| *value != 0.0),
        "every leave is zero"
    );

    // Closed: the owner row completed, nothing staged, the job on generation 2.
    let (completed, staged_left): (bool, i64) = sqlx::query_as(
        "SELECT (SELECT completed_at IS NOT NULL FROM leave_generation_transitions
                 WHERE job_id = $1 AND generation = 1),
                (SELECT COUNT(*) FROM leave_rack_staging WHERE job_id = $1)",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!((completed, staged_left), (true, 0));
    let mut conn = db.pool.acquire().await.unwrap();
    let current = leave_gen::current_generation(&mut conn, job, &config(&db, job).await)
        .await
        .unwrap();
    assert_eq!(current, Some(2));
    drop(conn);

    drop(state);
    scratch_root.assert_clean();
}

/// I-LEAVE-8: `rebuild_artifacts` reproduces every generation's bytes -- the
/// zeroed generation 0 and a folded generation 1 hash to what was recorded
/// when they were written. A lost object is rewritten with those bytes. And
/// the comparison is a real one: rows that drift after the close rebuild to a
/// different digest, reported as a mismatch and left alone. Whatever the
/// object holds, the check leaves workers sent its hash.
#[tokio::test]
#[ignore = "needs a real MAGPIE (MAGPIE_BIN) and MinIO (TEST_S3_ENDPOINT); tier 6"]
async fn a_rebuild_reproduces_every_generations_bytes() {
    let scratch_root = ScratchRoot::new();
    let db = TestDb::new().await;
    let (state, _bucket) = real_state(&db).await;
    let (job, distribution) = leave_job(&db, &state, 3, 2, None).await;
    fill_generation(&db, job, 1, 3).await;
    own_transition(&db, job, 1).await;
    let key = leave_gen::run_transition(&state, job, 1, &config(&db, job).await, &distribution)
        .await
        .unwrap();
    let rebuild = |force: bool| {
        leave_gen::rebuild_artifacts(
            &state.pool,
            &state.artifacts,
            &state.magpie,
            &state.builders,
            job,
            &distribution,
            force,
        )
    };
    let summary = |report: &[leave_gen::ArtifactRebuild]| -> Vec<(i32, bool, bool, bool, bool)> {
        report
            .iter()
            .map(|r| {
                (
                    r.generation,
                    r.matches,
                    r.same_builder,
                    r.object_present,
                    r.rewritten,
                )
            })
            .collect()
    };

    let report = rebuild(false).await.unwrap();
    assert_eq!(
        summary(&report),
        vec![(0, true, true, true, false), (1, true, true, true, false)]
    );
    assert!(report.iter().all(|r| r.rebuilt_sha256 == r.stored_sha256));

    // The object is lost; the rebuild puts the same bytes back.
    state.artifacts.delete(&key).await.unwrap();
    let report = rebuild(false).await.unwrap();
    assert_eq!(summary(&report)[1], (1, true, true, false, true));
    let restored = state.artifacts.get(&key).await.unwrap();
    assert_eq!(sha256(&restored), report[1].stored_sha256);

    // The rows drift after the close. The rebuild says so, and does not
    // overwrite the only copy of what was built.
    sqlx::query(
        "UPDATE leave_rack_progress SET occurrence_count = occurrence_count + 1
         WHERE job_id = $1 AND generation = 1
           AND rack = (SELECT MIN(rack) FROM leave_rack_progress WHERE job_id = $1 AND generation = 1)",
    )
    .bind(job)
    .execute(&db.pool)
    .await
    .unwrap();
    let report = rebuild(false).await.unwrap();
    assert_eq!(summary(&report)[1], (1, false, true, true, false));
    assert_ne!(report[1].rebuilt_sha256, report[1].stored_sha256);
    let kept = state.artifacts.get(&key).await.unwrap();
    assert_eq!(
        sha256(&kept),
        report[1].stored_sha256,
        "the stored KLV was left alone"
    );

    // Forced, it writes the new bytes -- and what workers are told to check
    // the object against follows them, while the first hash stays as evidence.
    // Left behind, every task of the next generation failed its check.
    let report = rebuild(true).await.unwrap();
    assert_eq!(summary(&report)[1], (1, false, true, true, true));
    let (first, served): (String, Option<String>) = sqlx::query_as(
        "SELECT sha256, served_sha256 FROM leave_generation_artifacts
         WHERE job_id = $1 AND generation = 1",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(first, report[1].stored_sha256);
    assert_eq!(served.as_deref(), Some(report[1].rebuilt_sha256.as_str()));
    assert_eq!(sha256(&state.artifacts.get(&key).await.unwrap()), report[1].rebuilt_sha256);
    assert!(report[0].rewritten);
    let unchanged: Option<String> = sqlx::query_scalar(
        "SELECT served_sha256 FROM leave_generation_artifacts WHERE job_id = $1 AND generation = 0",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(unchanged, None, "rewritten with the same bytes, it still serves the first hash");

    // An operator puts the older object version back (RUNBOOK §3). A check
    // that rewrites nothing still makes what workers are sent the object's
    // own hash: left at the forced bytes' hash, every worker declined it.
    state.artifacts.put(&key, kept).await.unwrap();
    let report = rebuild(false).await.unwrap();
    assert_eq!(summary(&report)[1], (1, false, true, true, false));
    assert_eq!(report[1].served_sha256, report[1].stored_sha256);
    let served: Option<String> = sqlx::query_scalar(
        "SELECT served_sha256 FROM leave_generation_artifacts WHERE job_id = $1 AND generation = 1",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(served, None, "the object holds the first bytes again");

    drop(state);
    scratch_root.assert_clean();
}

/// I-LEAVE-9, on real KLVs and through the worker API: a claim that finds
/// generation 1 complete starts its transition, which builds and uploads the
/// KLV; the job goes on to generation 2, whose tasks play that KLV; and the
/// transition a claim starts at the end of generation 2 completes the job.
#[tokio::test]
#[ignore = "needs a real MAGPIE (MAGPIE_BIN) and MinIO (TEST_S3_ENDPOINT); tier 6"]
async fn a_two_generation_job_runs_to_completion_on_real_klvs() {
    let scratch_root = ScratchRoot::new();
    let db = TestDb::new().await;
    let (state, bucket) = real_state(&db).await;
    let (job, _) = leave_job(&db, &state, 1, 2, None).await;
    let app = birdtest::app(state.clone());

    fill_generation(&db, job, 1, 1).await;
    let (status, body) = claim(&app).await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "the claim starts the transition: {body}"
    );
    assert!(
        wait_for(|| async { artifact(&db, job, 1).await.is_some() }).await,
        "generation 1 closed"
    );

    // Generation 2: its universe is seeded by a claim, and its tasks play
    // generation 1's KLV.
    let universe = || async {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM leave_rack_progress WHERE job_id = $1 AND generation = 2",
        )
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap()
    };
    let (status, _) = claim(&app).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(
        wait_for(|| async { universe().await == 149 }).await,
        "generation 2 was seeded"
    );
    let (status, task) = claim(&app).await;
    assert_eq!(status, StatusCode::OK, "{task}");
    assert_eq!(task["task_request"]["generation"], json!(2));
    assert_eq!(
        task["task_request"]["previous_artifact_key"],
        json!(leave_gen::artifact_key(job, 1))
    );
    let racks: Vec<String> = task["task_request"]["forced_racks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap().to_string())
        .collect();
    let result = json!({ "racks": racks.iter().map(|rack| json!({
        "rack": rack, "count": 1, "mean": 2.5,
    })).collect::<Vec<_>>() });
    let (status, accepted) = send(
        &app,
        post_json(
            "/api/worker/result",
            &[("x-worker-uuid", task["worker_uuid"].as_str().unwrap())],
            json!({ "claim_token": task["claim_token"], "result": result }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{accepted}");
    leave_gen::merge_staged(&db.pool, job, 2, true)
        .await
        .unwrap();

    // The rest of generation 2 reaches target; the next claim closes it, and
    // with it the job.
    sqlx::query(
        "UPDATE leave_rack_progress SET occurrence_count = 1
         WHERE job_id = $1 AND generation = 2 AND occurrence_count = 0",
    )
    .bind(job)
    .execute(&db.pool)
    .await
    .unwrap();
    let (status, body) = claim(&app).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let completed = || async {
        sqlx::query_scalar::<_, String>("SELECT status::text FROM jobs WHERE id = $1")
            .bind(job)
            .fetch_one(&db.pool)
            .await
            .unwrap()
            == "completed"
    };
    assert!(
        wait_for(completed).await,
        "the last generation's transition completes the job"
    );

    // Three KLVs, each stored as recorded.
    let mut keys = bucket.keys().await;
    keys.sort();
    let mut expected: Vec<String> = (0..3).map(|g| leave_gen::artifact_key(job, g)).collect();
    expected.sort();
    assert_eq!(keys, expected);
    for generation in 0..3 {
        let (key, recorded, _) = artifact(&db, job, generation).await.unwrap();
        assert_eq!(
            sha256(&state.artifacts.get(&key).await.unwrap()),
            recorded,
            "generation {generation}"
        );
    }
    let (status, body) = claim(&app).await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "a completed job hands out nothing: {body}"
    );

    drop((app, state));
    scratch_root.assert_clean();
}

/// I-DATA-2: two distributions with one name produce two different derived
/// KLVs. Both jobs pin a row named `english`; one bag has an extra A. Were a
/// conversion to read the distribution from a path, a default, or anything
/// keyed by the name, the two would come out identical. Instead generation 0
/// and generation 1 each differ between the jobs, and each job's KLVs load
/// against its own distribution, holding exactly that distribution's leaves.
#[tokio::test]
#[ignore = "needs a real MAGPIE (MAGPIE_BIN) and MinIO (TEST_S3_ENDPOINT); tier 6"]
async fn two_distributions_with_one_name_build_two_different_klvs() {
    let scratch_root = ScratchRoot::new();
    let db = TestDb::new().await;
    let (state, _bucket) = real_state(&db).await;
    let bigger = std::str::from_utf8(TESTDIST)
        .unwrap()
        .replacen("A,a,3,", "A,a,4,", 1);
    assert_ne!(bigger.as_bytes(), TESTDIST);
    let ld_a = letterdist(&db, "english", TESTDIST).await;
    let ld_b = letterdist(&db, "english", bigger.as_bytes()).await;
    let (job_a, dist_a) = leave_job(&db, &state, 1, 2, Some(ld_a)).await;
    let (job_b, dist_b) = leave_job(&db, &state, 1, 2, Some(ld_b)).await;
    assert_eq!(dist_a.name, dist_b.name);

    // The same rule for both generations' totals: every rack once, worth 1.
    for job in [job_a, job_b] {
        sqlx::query(
            "UPDATE leave_rack_progress SET occurrence_count = 1, equity_sum = 1.0
             WHERE job_id = $1 AND generation = 1",
        )
        .bind(job)
        .execute(&db.pool)
        .await
        .unwrap();
    }
    for (job, distribution) in [(job_a, &dist_a), (job_b, &dist_b)] {
        own_transition(&db, job, 1).await;
        leave_gen::run_transition(&state, job, 1, &config(&db, job).await, distribution)
            .await
            .unwrap();
    }

    for generation in [0, 1] {
        let (a, b) = (
            artifact(&db, job_a, generation).await.unwrap(),
            artifact(&db, job_b, generation).await.unwrap(),
        );
        assert_ne!(a.1, b.1, "generation {generation}: one name, one KLV");
    }
    for (job, distribution) in [(job_a, &dist_a), (job_b, &dist_b)] {
        let mut expected = distribution.enumerate_leaves(6);
        expected.sort();
        for generation in [0, 1] {
            let (key, _, _) = artifact(&db, job, generation).await.unwrap();
            let bytes = state.artifacts.get(&key).await.unwrap();
            let mut leaves: Vec<String> = klv_values(&state.magpie, distribution, &bytes)
                .await
                .into_iter()
                .map(|(leave, _)| leave)
                .collect();
            leaves.sort();
            assert_eq!(
                leaves, expected,
                "generation {generation} is its own distribution's"
            );
        }
    }

    drop(state);
    scratch_root.assert_clean();
}

/// Writes `rows` as a rack-equity CSV in a fresh scratch directory and runs
/// `rackequity2klv` on it: the KLV, or MAGPIE's refusal.
async fn convert_rows(
    magpie: &Magpie,
    distribution: &LetterDistribution,
    rows: &[(String, i64, f64)],
) -> Result<Vec<u8>, String> {
    let scratch = ScratchData::empty().await.unwrap();
    scratch
        .write(
            "letterdistributions",
            &distribution.name,
            ".csv",
            &distribution.bytes,
        )
        .await
        .unwrap();
    let csv: String = rows
        .iter()
        .map(|(rack, count, sum)| format!("{rack},{count},{sum:.10}\n"))
        .collect();
    scratch
        .write("lexica", "gen", ".csv", csv.as_bytes())
        .await
        .unwrap();
    match magpie
        .convert(&scratch, "rackequity2klv", "gen", &distribution.name)
        .await
    {
        Ok(()) => Ok(tokio::fs::read(scratch.lexicon_path("gen", ".klv2"))
            .await
            .unwrap()),
        Err(err) => {
            assert!(
                !scratch.lexicon_path("gen", ".klv2").exists(),
                "MAGPIE refused and wrote a KLV anyway"
            );
            Err(err.message)
        }
    }
}

/// M-8: a rack the CSV names differently from MAGPIE -- in lower case (which
/// MAGPIE reads as blanked letters), with a letter the distribution does not
/// have, or with the blank spelled other than the distribution spells it -- is
/// refused, rather than the rack it stood for being valued at zero. A
/// different *order* of the same letters is the same rack and the same KLV.
/// And through the server: a transition whose rows name a rack MAGPIE does not
/// know, or miss one, fails without closing the generation or storing
/// anything.
#[tokio::test]
#[ignore = "needs a real MAGPIE (MAGPIE_BIN) and MinIO (TEST_S3_ENDPOINT); tier 6"]
async fn a_rack_named_differently_is_refused_rather_than_valued_at_zero() {
    let scratch_root = ScratchRoot::new();
    let magpie = Magpie::new(magpie_bin(), 1);
    let distribution = LetterDistribution::parse(TESTDIST, "testdist").unwrap();
    let index = RackIndex::new(&distribution, 7).unwrap();
    let rows: Vec<(String, i64, f64)> = (0..index.total())
        .map(|i| {
            (
                index.rack_at(i).unwrap(),
                3 + i as i64,
                (i % 11) as f64 - 5.0,
            )
        })
        .collect();
    let complete = convert_rows(&magpie, &distribution, &rows)
        .await
        .expect("the complete CSV");

    let with_blank = rows
        .iter()
        .position(|(rack, _, _)| rack.contains('?'))
        .unwrap();
    let with_letters = rows
        .iter()
        .position(|(rack, _, _)| !rack.contains('?'))
        .unwrap();
    let renamed = |at: usize, name: String| {
        let mut rows = rows.clone();
        rows[at].0 = name;
        rows
    };
    for (what, rows) in [
        (
            "lower case",
            renamed(with_letters, rows[with_letters].0.to_lowercase()),
        ),
        (
            "a letter not in the bag",
            renamed(with_letters, format!("{}Z", &rows[with_letters].0[..6])),
        ),
        (
            "the blank spelled `_`",
            renamed(with_blank, rows[with_blank].0.replace('?', "_")),
        ),
    ] {
        let refusal = convert_rows(&magpie, &distribution, &rows)
            .await
            .expect_err(&format!("{what}: a misnamed rack was accepted"));
        assert!(
            refusal.contains("full racks of 7 tiles drawable from this letter distribution"),
            "{what}: {refusal}"
        );
    }
    let reversed: String = rows[with_letters].0.chars().rev().collect();
    assert_ne!(reversed, rows[with_letters].0);
    let same = convert_rows(&magpie, &distribution, &renamed(with_letters, reversed))
        .await
        .unwrap();
    assert_eq!(
        same, complete,
        "the same letters in another order are the same rack"
    );

    // Through the server's own transition.
    let db = TestDb::new().await;
    let (state, bucket) = real_state(&db).await;
    let (job, job_distribution) = leave_job(&db, &state, 1, 2, None).await;
    fill_generation(&db, job, 1, 1).await;
    own_transition(&db, job, 1).await;
    let first: String = sqlx::query_scalar(
        "SELECT MIN(rack) FROM leave_rack_progress WHERE job_id = $1 AND generation = 1 AND rack !~ '[?]'",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let rename = |from: String, to: String| {
        sqlx::query(
            "UPDATE leave_rack_progress SET rack = $3 WHERE job_id = $1 AND generation = 1 AND rack = $2",
        )
        .bind(job)
        .bind(from)
        .bind(to)
        .execute(&db.pool)
    };
    rename(first.clone(), first.to_lowercase()).await.unwrap();
    let err = leave_gen::run_transition(&state, job, 1, &config(&db, job).await, &job_distribution)
        .await
        .expect_err("a misnamed rack closed the generation");
    assert!(
        err.message
            .contains("drawable from this letter distribution"),
        "{}",
        err.message
    );
    rename(first.to_lowercase(), first.clone()).await.unwrap();

    // A rack missing is refused by the server before MAGPIE is asked.
    sqlx::query(
        "DELETE FROM leave_rack_progress WHERE job_id = $1 AND generation = 1 AND rack = $2",
    )
    .bind(job)
    .bind(&first)
    .execute(&db.pool)
    .await
    .unwrap();
    let err = leave_gen::run_transition(&state, job, 1, &config(&db, job).await, &job_distribution)
        .await
        .expect_err("a missing rack closed the generation");
    assert!(err.message.contains("148 progress rows"), "{}", err.message);

    assert!(
        artifact(&db, job, 1).await.is_none(),
        "no generation-1 artifact was recorded"
    );
    assert_eq!(
        bucket.keys().await,
        vec![leave_gen::artifact_key(job, 0)],
        "nothing was stored"
    );
    let open: bool = sqlx::query_scalar(
        "SELECT completed_at IS NULL FROM leave_generation_transitions WHERE job_id = $1 AND generation = 1",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(open, "the generation is still open");

    drop(state);
    scratch_root.assert_clean();
}
