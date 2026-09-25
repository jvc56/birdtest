//! Tier 6: admin routes whose work is running the server's own MAGPIE --
//! creating a leave-generation job (its generation-0 KLV) and rebuilding a
//! job's artifacts -- through the real router, against a real database and a
//! real object store.
//!
//! **Opt-in, then fail loudly**, like `magpie_smoke.rs`: `#[ignore]` by
//! default, and when asked for they fail rather than skip if MAGPIE is missing.
//! They also need `TEST_DATABASE_URL` and `TEST_S3_ENDPOINT`.
//!
//!     MAGPIE_BIN=../MAGPIE/bin/magpie cargo nextest run --test magpie_routes --run-ignored all

mod common;

use birdtest::jobs::leave_gen::artifact_key;
use common::*;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

fn magpie_bin() -> String {
    let binary =
        std::env::var("MAGPIE_BIN").unwrap_or_else(|_| "../MAGPIE/bin/magpie".to_string());
    assert!(
        std::path::Path::new(&binary).is_file(),
        "no MAGPIE at {binary}. Build one (`make magpie BUILD=portable_release` in a MAGPIE \
         checkout) and set MAGPIE_BIN. These tests fail rather than skip on purpose: a green \
         run must not mean nothing was exercised."
    );
    binary
}

/// An app whose object store is a fresh bucket and whose MAGPIE is the real
/// one. The harness builds every state with a MAGPIE path that is not a binary,
/// so a test below tier 6 cannot run one by accident; this is the one place
/// that swaps it in.
struct Stack {
    db: TestDb,
    state: birdtest::state::AppState,
    bucket: TestBucket,
    app: axum::Router,
    headers: Vec<(String, String)>,
}

impl Stack {
    async fn new() -> Self {
        let binary = magpie_bin();
        let db = TestDb::new().await;
        let (mut state, bucket) = db.state_with_object_store().await;
        state.magpie = birdtest::magpie::Magpie::new(&binary, 1);
        let admin = db.user("root", true).await;
        let headers = admin_headers(&state.cfg, admin);
        let app = birdtest::app(state.clone());
        Stack { db, state, bucket, app, headers }
    }

    async fn post(&self, path: &str, body: Value) -> (axum::http::StatusCode, Value) {
        let refs: Vec<(&str, &str)> = self.headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        send(&self.app, post_json(path, &refs, body)).await
    }

    /// A leave-generation job created through the API, over the test
    /// distribution.
    async fn leave_job(&self) -> Value {
        let body = json!({
            "job_type": "leave_generation", "variant": "classic",
            "letterdist_id": self.db.input_data("letterdist", "english").await,
            "layout_id": self.db.input_data("layout", "standard15").await,
            "kwg_id": self.db.input_data("kwg", "NWL23").await,
            "num_iterations": 100, "generation_count": 2,
            "target_rack_count": 10, "racks_per_task": 5,
        });
        let (status, created) = self.post("/api/admin/jobs", body).await;
        assert_eq!(status, axum::http::StatusCode::CREATED, "{created}");
        created
    }
}

/// A-ADMIN-2 (leave generation): creating a leave job answers `{job}`, inactive
/// and unallocated like every other type, and has already stored what its first
/// generation plays with -- the zeroed KLV MAGPIE built from the job's pinned
/// distribution, recorded under the key the artifact route will serve. Nothing
/// else is written up front: no task, no rack universe.
#[tokio::test]
#[ignore]
async fn a_leave_job_is_created_inactive_with_its_generation_zero_leaves_stored() {
    let stack = Stack::new().await;
    let created = stack.leave_job().await;

    let keys: Vec<&String> = created.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["job"], "{created}");
    let job = &created["job"];
    assert_eq!((&job["job_type"], &job["status"], &job["allocation"]), (&json!("leave_generation"), &json!("inactive"), &Value::Null));
    let id: Uuid = job["id"].as_str().unwrap().parse().unwrap();

    let (key, sha256, builder): (String, String, String) = sqlx::query_as(
        "SELECT artifact_key, sha256, builder FROM leave_generation_artifacts
         WHERE job_id = $1 AND generation = 0",
    )
    .bind(id)
    .fetch_one(&stack.db.pool)
    .await
    .unwrap();
    assert_eq!(key, artifact_key(id, 0));
    assert_eq!(builder, stack.state.builders.klv());
    assert_eq!(stack.bucket.keys().await, vec![key.clone()]);
    let stored = stack.state.artifacts.get(&key).await.unwrap();
    assert!(!stored.is_empty());
    assert_eq!(hex::encode(Sha256::digest(&stored)), sha256, "the recorded digest is of the stored bytes");

    let (tasks, universe): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM tasks WHERE job_id = $1),
                (SELECT COUNT(*) FROM leave_rack_progress WHERE job_id = $1)",
    )
    .bind(id)
    .fetch_one(&stack.db.pool)
    .await
    .unwrap();
    assert_eq!((tasks, universe), (0, 0));
}

/// A-ADMIN-11: `rebuild-artifacts` recomputes every generation's KLV and says,
/// per generation, what it found and whether it wrote: an object that is
/// present is left alone, a lost one is restored from the database, and asking
/// again changes nothing -- the same report, the same bytes. `force` rewrites
/// a present one.
#[tokio::test]
#[ignore]
async fn rebuilding_artifacts_restores_what_is_missing_and_is_idempotent() {
    let stack = Stack::new().await;
    let created = stack.leave_job().await;
    let id: Uuid = created["job"]["id"].as_str().unwrap().parse().unwrap();
    let path = format!("/api/admin/jobs/{id}/rebuild-artifacts");

    // A closed generation 1, from a fully counted universe, whose object was
    // never written: its digest on record is not what its rows rebuild to.
    let mut conn = stack.db.pool.acquire().await.unwrap();
    let job_data = birdtest::jobs::load_job_data(&mut conn, id).await.unwrap();
    birdtest::jobs::leave_gen::seed_generation(&mut conn, id, 1, &job_data.letterdist)
        .await
        .unwrap();
    drop(conn);
    sqlx::query(
        "UPDATE leave_rack_progress SET occurrence_count = 12, equity_sum = 30.0
         WHERE job_id = $1 AND generation = 1",
    )
    .bind(id)
    .execute(&stack.db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO leave_generation_artifacts (job_id, generation, artifact_key, sha256, builder)
         VALUES ($1, 1, $2, repeat('0', 64), $3)",
    )
    .bind(id)
    .bind(artifact_key(id, 1))
    .bind(stack.state.builders.klv())
    .execute(&stack.db.pool)
    .await
    .unwrap();

    let (status, first) = stack.post(&path, json!({})).await;
    assert_eq!(status, axum::http::StatusCode::OK, "{first}");
    let report = first.as_array().expect("one entry per generation");
    assert_eq!(report.len(), 2, "{first}");
    let gen0 = &report[0];
    assert_eq!(
        (&gen0["generation"], &gen0["artifact_key"], &gen0["object_present"], &gen0["rewritten"], &gen0["matches"], &gen0["same_builder"]),
        (&json!(0), &json!(artifact_key(id, 0)), &json!(true), &json!(false), &json!(true), &json!(true)),
        "a present, matching generation 0 is left alone: {first}"
    );
    assert_eq!(gen0["stored_sha256"], gen0["rebuilt_sha256"]);
    let gen1 = &report[1];
    assert_eq!(
        (&gen1["generation"], &gen1["object_present"], &gen1["rewritten"], &gen1["matches"]),
        (&json!(1), &json!(false), &json!(true), &json!(false)),
        "a missing generation 1 is restored: {first}"
    );
    let rebuilt = stack.state.artifacts.get(&artifact_key(id, 1)).await.unwrap();
    assert_eq!(json!(hex::encode(Sha256::digest(&rebuilt))), gen1["rebuilt_sha256"]);

    // Lose generation 0's object; the rebuild puts back the bytes on record.
    let gen0_key = artifact_key(id, 0);
    stack.state.artifacts.delete(&gen0_key).await.unwrap();
    let (status, second) = stack.post(&path, json!({})).await;
    assert_eq!(status, axum::http::StatusCode::OK, "{second}");
    assert_eq!(
        (&second[0]["object_present"], &second[0]["rewritten"], &second[0]["matches"]),
        (&json!(false), &json!(true), &json!(true)),
        "{second}"
    );
    let restored = stack.state.artifacts.get(&gen0_key).await.unwrap();
    assert_eq!(json!(hex::encode(Sha256::digest(&restored))), gen0["stored_sha256"]);

    // Again, with everything present: nothing is written, and the report is
    // the first one's -- the same digests from the same rows.
    let (status, third) = stack.post(&path, json!({})).await;
    assert_eq!(status, axum::http::StatusCode::OK, "{third}");
    assert_eq!(third[0], first[0], "an idempotent rebuild reports what the first found");
    assert_eq!(
        (&third[1]["object_present"], &third[1]["rewritten"], &third[1]["rebuilt_sha256"]),
        (&json!(true), &json!(false), &gen1["rebuilt_sha256"]),
        "{third}"
    );
    let mut keys = stack.bucket.keys().await;
    keys.sort();
    assert_eq!(keys, vec![gen0_key.clone(), artifact_key(id, 1)]);

    // `force` rewrites what is present -- once the job is not dispatching:
    // a worker mid-task would refuse the rewritten bytes.
    sqlx::query("UPDATE jobs SET status = 'active' WHERE id = $1")
        .bind(id)
        .execute(&stack.state.pool)
        .await
        .unwrap();
    let (status, refused) = stack.post(&format!("{path}?force=true"), json!({})).await;
    assert_eq!(status, axum::http::StatusCode::CONFLICT, "{refused}");
    sqlx::query("UPDATE jobs SET status = 'inactive' WHERE id = $1")
        .bind(id)
        .execute(&stack.state.pool)
        .await
        .unwrap();
    let (status, forced) = stack.post(&format!("{path}?force=true"), json!({})).await;
    assert_eq!(status, axum::http::StatusCode::OK, "{forced}");
    assert!(forced.as_array().unwrap().iter().all(|g| g["rewritten"] == json!(true)), "{forced}");

    let audited: Vec<String> = sqlx::query_scalar(
        "SELECT reason FROM audit_log WHERE action = 'job.artifacts_rebuilt' AND job_id = $1 ORDER BY id",
    )
    .bind(id)
    .fetch_all(&stack.db.pool)
    .await
    .unwrap();
    assert_eq!(
        audited,
        [
            "generations=2 rewritten=1 mismatched=1 force=false",
            "generations=2 rewritten=1 mismatched=1 force=false",
            "generations=2 rewritten=0 mismatched=1 force=false",
            "generations=2 rewritten=2 mismatched=1 force=true",
        ]
    );
}
