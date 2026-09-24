//! The server's handling of the fake worker's adversarial modes, where that
//! needs the database (U-FAKE-5). The submissions are the captured ones
//! `jobs::plausibility::fixture_tests` checks at tier 1; `--mode malformed` is
//! refused by validation alone and is proven there. To regenerate a fixture see
//! `backend/src/jobs/testdata/README.md`.

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};

/// `python3 worker/fake_worker.py --mode stale --emit-fixture
/// contract-fixtures/assignment-games.json`: a valid ten-game result under a
/// claim token the server never issued.
const STALE: &str = include_str!("../src/jobs/testdata/fake_worker_stale.json");

/// U-FAKE-5, `--mode stale`: posted by a worker that holds a live claim on a
/// task the result would fit, the captured body is `accepted: false` and
/// changes nothing. The same result under the token that was issued is then
/// accepted, so it is the token alone the server refused -- the stale mode
/// cannot silently become a valid submission.
#[tokio::test]
async fn a_stale_mode_submission_is_not_accepted_and_changes_nothing() {
    let db = TestDb::new().await;
    // Ten games a batch: the size the fixture's result reports.
    let job = db.games_job(1, 10).await;
    let app = birdtest::app(db.state().await);

    let (status, assignment) =
        send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{assignment}");
    let uuid = assignment["worker_uuid"].as_str().unwrap().to_string();
    let stale: Value = serde_json::from_str(STALE).unwrap();
    assert_ne!(stale["claim_token"], assignment["claim_token"]);

    let (status, body) =
        send(&app, post_json("/api/worker/result", &[("x-worker-uuid", &uuid)], stale.clone()))
            .await;
    assert_eq!(status, StatusCode::OK, "a stale token is not an error: {body}");
    assert_eq!(body, json!({ "accepted": false }));

    let (claim_state, accepted, results): (String, i32, i64) = sqlx::query_as(
        "SELECT c.state::text, t.accepted_count,
                (SELECT COUNT(*) FROM game_results WHERE job_id = $1)
         FROM task_claims c JOIN tasks t ON t.id = c.task_id WHERE t.job_id = $1",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!((claim_state.as_str(), accepted, results), ("claimed", 0, 0));

    let issued = json!({ "claim_token": assignment["claim_token"], "result": stale["result"] });
    let (status, body) =
        send(&app, post_json("/api/worker/result", &[("x-worker-uuid", &uuid)], issued)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({ "accepted": true }));
}
