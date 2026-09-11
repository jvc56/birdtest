//! Leave generation against a real database: the full-rack universe, forced
//! racks that are full racks, submissions folding into it, and claim-time
//! selection skipping racks already out (AUDIT_FINDINGS.md F1, F10).

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::json;
use std::collections::HashSet;
use uuid::Uuid;

/// A leave-generation job over the test distribution (13 tiles, so a small
/// full-rack universe), with its generation-1 universe seeded and a
/// generation-0 artifact row so it can dispatch.
async fn leave_job(db: &TestDb, racks_per_task: i32) -> (Uuid, i64) {
    let admin = db.user(&format!("admin{}", Uuid::new_v4().simple()), true).await;
    let job = db.bare_job("leave_generation", 1, admin).await;
    let kwg = db.input_data("kwg", "NWL23").await;
    sqlx::query(
        "INSERT INTO job_leave_config
             (job_id, kwg_id, num_iterations, generation_count, target_rack_count, racks_per_task)
         VALUES ($1, $2, 100, 2, 1000, $3)",
    )
    .bind(job)
    .bind(kwg)
    .bind(racks_per_task)
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO leave_generation_artifacts (job_id, generation, artifact_key, sha256)
         VALUES ($1, 0, 'leaves/test/generation-0.klv2', repeat('0', 64))",
    )
    .bind(job)
    .execute(&db.pool)
    .await
    .unwrap();

    let row = sqlx::query_as::<_, birdtest::models::job::Job>("SELECT * FROM jobs WHERE id = $1")
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let mut conn = db.pool.acquire().await.unwrap();
    let seeded = birdtest::jobs::registry::initialize_job_state(&mut conn, &row).await.unwrap();
    (job, seeded)
}

fn forced_racks(assignment: &serde_json::Value) -> Vec<String> {
    assignment["task_request"]["forced_racks"]
        .as_array()
        .expect("a leave assignment carries forced_racks")
        .iter()
        .map(|r| r.as_str().unwrap().to_string())
        .collect()
}

/// Bug: the server enumerated leaves of 1..6 tiles and forced them, and MAGPIE
/// refused every task ("forced racks must all be full racks of 7 tiles").
#[tokio::test]
async fn the_universe_and_the_forced_racks_are_full_racks() {
    let db = TestDb::new().await;
    let (job, seeded) = leave_job(&db, 5).await;

    let rows: Vec<String> =
        sqlx::query_scalar("SELECT rack FROM leave_rack_progress WHERE job_id = $1 AND generation = 1")
            .bind(job)
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(rows.len() as i64, seeded);
    assert!(rows.iter().all(|r| r.chars().count() == 7), "every row is a full rack");
    let ld = birdtest::jobs::racks::LetterDistribution::parse(
        include_bytes!("../src/jobs/testdata/testdist.csv"),
        "testdist",
    )
    .unwrap();
    assert_eq!(seeded as usize, ld.enumerate_racks(7).len());

    let app = birdtest::app(db.state().await);
    let (status, body) =
        send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let racks = forced_racks(&body);
    assert_eq!(racks.len(), 5);
    assert!(racks.iter().all(|r| r.chars().count() == 7), "{racks:?}");
}

/// F10: two workers claiming at once were both handed the same lowest-count
/// racks.
#[tokio::test]
async fn racks_out_with_an_open_claim_are_not_handed_out_again() {
    let db = TestDb::new().await;
    let (_, seeded) = leave_job(&db, 4).await;
    let app = birdtest::app(db.state().await);

    let mut seen = HashSet::new();
    let claims = (seeded / 4).min(6);
    for _ in 0..claims {
        let (status, body) =
            send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        for rack in forced_racks(&body) {
            assert!(seen.insert(rack.clone()), "rack {rack} handed to two open claims");
        }
    }
}

/// A submission adds to its racks' rows, and a rack that is not a full rack of
/// the distribution neither counts nor creates a row.
#[tokio::test]
async fn a_result_folds_into_the_generation_and_creates_no_rows() {
    let db = TestDb::new().await;
    let (job, seeded) = leave_job(&db, 2).await;
    let app = birdtest::app(db.state().await);

    let (status, body) =
        send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let uuid = body["worker_uuid"].as_str().unwrap();
    let token = body["claim_token"].as_str().unwrap();
    let racks = forced_racks(&body);

    let result = json!({ "racks": [
        { "rack": racks[0], "count": 3, "mean": 10.0 },
        { "rack": racks[1], "count": 1, "mean": -2.0 },
        // Seven tiles, but ZZZZZZZ is not drawable from this distribution.
        { "rack": "ZZZZZZZ", "count": 5, "mean": 1.0 },
    ]});
    let (status, body) = send(
        &app,
        post_json(
            "/api/worker/result",
            &[("x-worker-uuid", uuid)],
            json!({ "claim_token": token, "result": result }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (count, sum): (i64, f64) = sqlx::query_as(
        "SELECT occurrence_count, equity_sum FROM leave_rack_progress
         WHERE job_id = $1 AND generation = 1 AND rack = $2",
    )
    .bind(job)
    .bind(&racks[0])
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!((count, sum), (3, 30.0));

    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM leave_rack_progress WHERE job_id = $1")
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(rows, seeded, "no row for a rack outside the universe");
}

/// A leave result naming anything but full racks is refused outright.
#[tokio::test]
async fn a_leave_result_of_partial_racks_is_rejected() {
    let db = TestDb::new().await;
    leave_job(&db, 2).await;
    let app = birdtest::app(db.state().await);

    let (_, body) = send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    let uuid = body["worker_uuid"].as_str().unwrap();
    let token = body["claim_token"].as_str().unwrap();
    let (status, _) = send(
        &app,
        post_json(
            "/api/worker/result",
            &[("x-worker-uuid", uuid)],
            json!({ "claim_token": token, "result": { "racks": [
                { "rack": "AB", "count": 3, "mean": 10.0 }
            ]}}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
