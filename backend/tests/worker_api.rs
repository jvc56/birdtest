//! Worker API against a real database: the claim/submit races, identity
//! minting, and the scheduler decisions fixed in the audit. Each test names the
//! bug it would have caught.

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::json;
use uuid::Uuid;

/// Claims once with no identity, returning the assignment and the UUID the
/// server minted for it.
async fn first_claim(app: &axum::Router) -> (serde_json::Value, String) {
    let (status, body) =
        send(app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let uuid = body["worker_uuid"].as_str().expect("a minted worker_uuid").to_string();
    (body, uuid)
}

async fn claim_as(app: &axum::Router, uuid: &str) -> (StatusCode, serde_json::Value) {
    send(app, post_json("/api/worker/task", &[("x-worker-uuid", uuid)], claim_body("1.0.0", &[])))
        .await
}

async fn submit_as(
    app: &axum::Router,
    uuid: &str,
    token: &str,
    result: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    send(
        app,
        post_json(
            "/api/worker/result",
            &[("x-worker-uuid", uuid)],
            json!({ "claim_token": token, "result": result }),
        ),
    )
    .await
}

async fn anonymous_worker_count(db: &TestDb) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM anonymous_workers")
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

/// Bug: every request with no identity header inserted an `anonymous_workers`
/// row before anything else happened. A new contributor polling a quiet server
/// got a 204 with no body to carry the UUID in, so it minted a fresh row on
/// every poll, forever.
#[tokio::test]
async fn a_worker_with_no_identity_is_persisted_only_when_given_a_task() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);

    for _ in 0..3 {
        let (status, _) =
            send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }
    assert_eq!(anonymous_worker_count(&db).await, 0, "idle polls must not mint identities");

    db.games_job(1, 2).await;
    let (_, uuid) = first_claim(&app).await;
    assert_eq!(anonymous_worker_count(&db).await, 1);

    // And the minted identity is usable from then on.
    let (status, _) = claim_as(&app, &uuid).await;
    assert_eq!(status, StatusCode::OK);
}

/// Bug: omitting the identity header minted an identity on any worker
/// endpoint, which also put each such request in a rate-limit bucket of its
/// own. Only a claim can create an identity.
#[tokio::test]
async fn requests_other_than_a_claim_require_an_identity() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);

    let (status, body) = send(
        &app,
        post_json("/api/worker/heartbeat", &[], json!({ "claim_token": Uuid::new_v4() })),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert!(body["message"].as_str().unwrap().contains("worker identity"), "{body}");
    assert_eq!(anonymous_worker_count(&db).await, 0);
}

/// Bug: with redundancy above 1, a task stays `available` after its first
/// claim, and `next_available` offered it straight back to the worker holding
/// it. The per-identity unique index refused the slot, the claim retried three
/// times against the same task, and the worker got nothing at all.
#[tokio::test]
async fn redundancy_does_not_starve_a_worker_holding_a_slot() {
    let db = TestDb::new().await;
    let job = db.games_job(2, 2).await;
    let app = birdtest::app(db.state().await);

    let (first, uuid) = first_claim(&app).await;
    let (status, second) = claim_as(&app, &uuid).await;
    assert_eq!(status, StatusCode::OK, "the second claim must still find work: {second}");
    assert_eq!(second["job_id"], json!(job.to_string()));
    assert_ne!(
        first["task_request"]["seed"], second["task_request"]["seed"],
        "a worker must never hold two slots on one task"
    );

    // A second, independent worker is the one that fills the first task's
    // other slot.
    let (other, _) = first_claim(&app).await;
    assert_eq!(other["task_request"]["seed"], first["task_request"]["seed"]);
}

/// Bug: the submit path read the claim outside its transaction and marked it
/// completed unconditionally. A timeout reclaiming it in between left the
/// claim both abandoned and completed and the task's live count decremented
/// twice. The sequential version of that race: a result for a reclaimed claim
/// is refused and leaves the counters alone, and a retried submission of an
/// accepted result is an `accepted: false`, not a 500.
#[tokio::test]
async fn submissions_for_reclaimed_or_already_accepted_claims_change_nothing() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 2).await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());

    let (assignment, uuid) = first_claim(&app).await;
    let token = assignment["claim_token"].as_str().unwrap().to_string();

    sqlx::query("UPDATE task_claims SET claimed_at = now() - interval '1 hour'")
        .execute(&db.pool)
        .await
        .unwrap();
    let reclaimed = birdtest::scheduler::reclaim_expired(&db.pool, job, 300.0).await.unwrap();
    assert_eq!(reclaimed, 1);

    let (status, body) = submit_as(&app, &uuid, &token, games_result(2, 1)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "accepted": false }));

    let (active, accepted, state_text): (i32, i32, String) = sqlx::query_as(
        "SELECT active_claim_count, accepted_count, state::text FROM tasks WHERE job_id = $1",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!((active, accepted, state_text.as_str()), (0, 0, "available"));

    // The same worker may take the reopened task again, and this time finish.
    let (status, again) = claim_as(&app, &uuid).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(again["task_request"]["seed"], assignment["task_request"]["seed"]);
    let token = again["claim_token"].as_str().unwrap().to_string();
    let (_, body) = submit_as(&app, &uuid, &token, games_result(2, 1)).await;
    assert_eq!(body, json!({ "accepted": true }));

    let (status, body) = submit_as(&app, &uuid, &token, games_result(2, 1)).await;
    assert_eq!(status, StatusCode::OK, "a retried submission must not be a server error");
    assert_eq!(body, json!({ "accepted": false }));
}

/// Bug: SPRT, progress and ratings summed every `game_results` row. With
/// redundancy 2 each seeded batch is played twice with identical outcomes, so
/// every game counted twice and SPRT saw double the evidence it had.
#[tokio::test]
async fn redundant_results_for_one_task_count_once() {
    let db = TestDb::new().await;
    let job = db.games_job(2, 2).await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());

    let (a, uuid_a) = first_claim(&app).await;
    let (b, uuid_b) = first_claim(&app).await;
    assert_eq!(a["task_request"]["seed"], b["task_request"]["seed"], "same task, two slots");

    for (assignment, uuid) in [(&a, &uuid_a), (&b, &uuid_b)] {
        let token = assignment["claim_token"].as_str().unwrap();
        let (_, body) = submit_as(&app, uuid, token, games_result(2, 2)).await;
        assert_eq!(body, json!({ "accepted": true }));
    }

    let job_row = birdtest::jobstats::load_job(&db.pool, job).await.unwrap();
    let games = birdtest::jobstats::game_stats(&db.pool, &job_row).await.unwrap().unwrap();
    assert_eq!(games.units_completed, 2, "two games were played, not four");
    assert_eq!(games.wins, 2);
}

/// Bug: any error claiming from one job failed the whole claim, so a single
/// job that could not dispatch -- here, one whose config row is missing --
/// answered every worker with a 500 for as long as it led the tier.
#[tokio::test]
async fn a_job_that_cannot_dispatch_does_not_block_the_others() {
    let db = TestDb::new().await;
    let admin = db.user("admin", true).await;
    // Created first, so it wins the deficit tie-break and is tried first.
    let broken = db.bare_job("games", 1, admin).await;
    let healthy = db.games_job(1, 2).await;
    let app = birdtest::app(db.state().await);

    let (status, body) =
        send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["job_id"], json!(healthy.to_string()));
    assert_ne!(body["job_id"], json!(broken.to_string()));
}

/// Bug: the shutdown reason treated any non-empty unsupported set as a data
/// problem, so a worker too old for every active job, which happened to list a
/// long-finished job as unsupported, was told to update its data too.
#[tokio::test]
async fn a_stale_unsupported_entry_does_not_change_the_shutdown_reason() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 2).await;
    sqlx::query(
        "UPDATE jobs SET min_magpie_major = 2, min_magpie_minor = 0, min_magpie_patch = 0
         WHERE id = $1",
    )
    .bind(job)
    .execute(&db.pool)
    .await
    .unwrap();
    let admin = db.user("admin", true).await;
    let finished = db.bare_job("games", 1, admin).await;
    sqlx::query("UPDATE jobs SET status = 'completed' WHERE id = $1")
        .bind(finished)
        .execute(&db.pool)
        .await
        .unwrap();
    let app = birdtest::app(db.state().await);

    let (status, body) =
        send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[finished]))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["shutdown"]["reason"], "magpie_too_old", "{body}");
    assert_eq!(body["shutdown"]["required_magpie_version"], "2.0.0");
}

/// The deficit the scheduler orders on is a counter now; it must move with
/// every claim, declined ones included.
#[tokio::test]
async fn every_claim_advances_the_dispatch_counter() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 2).await;
    let app = birdtest::app(db.state().await);

    let (assignment, uuid) = first_claim(&app).await;
    let (status, _) = send(
        &app,
        post_json(
            "/api/worker/decline",
            &[("x-worker-uuid", &uuid)],
            json!({ "claim_token": assignment["claim_token"], "reason": "missing_data" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = claim_as(&app, &uuid).await;
    assert_eq!(status, StatusCode::OK);

    let issued: i64 = sqlx::query_scalar("SELECT claims_issued FROM jobs WHERE id = $1")
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(issued, 2);

    // Declining twice must not release the claim twice.
    let (status, _) = send(
        &app,
        post_json(
            "/api/worker/decline",
            &[("x-worker-uuid", &uuid)],
            json!({ "claim_token": assignment["claim_token"], "reason": "missing_data" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let active: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(active_claim_count), 0)::bigint FROM tasks WHERE job_id = $1",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(active, 1);
}

/// F13: a claim token worked for any registered identity, so a banned worker
/// could hand its tokens to another, and a result was audit-logged under
/// whoever submitted it. The token is now bound to the identity it was issued
/// to; any other identity is treated as holding an unknown token.
#[tokio::test]
async fn a_claim_token_works_only_for_the_identity_it_was_issued_to() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    db.games_job(1, 2).await;

    let (owner_claim, owner) = first_claim(&app).await;
    let (_, other) = first_claim(&app).await;
    assert_ne!(owner, other);
    let token = owner_claim["claim_token"].as_str().unwrap();

    let heartbeat_at = || async {
        sqlx::query_scalar::<_, Option<chrono::DateTime<chrono::Utc>>>(
            "SELECT last_heartbeat_at FROM task_claims WHERE claim_token = $1::uuid",
        )
        .bind(token)
        .fetch_one(&db.pool)
        .await
        .unwrap()
    };
    let before = heartbeat_at().await;
    let (status, _) = send(
        &app,
        post_json("/api/worker/heartbeat", &[("x-worker-uuid", &other)], json!({ "claim_token": token })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(heartbeat_at().await, before, "another identity's heartbeat keeps nothing alive");

    let (status, _) = send(
        &app,
        post_json(
            "/api/worker/decline",
            &[("x-worker-uuid", &other)],
            json!({ "claim_token": token, "reason": "missing_data" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "another identity cannot decline it");

    let (status, body) = submit_as(&app, &other, token, games_result(2, 1)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["accepted"], false, "another identity cannot submit for it");

    let (status, body) = submit_as(&app, &owner, token, games_result(2, 1)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["accepted"], true, "the owner still can: {body}");
}

/// F5: public endpoints published anonymous workers' UUIDs, which are their
/// only credential. They now publish a derived pseudonym, and only the admin
/// listing carries the UUID.
#[tokio::test]
async fn public_endpoints_name_anonymous_workers_by_pseudonym_only() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let job = db.games_job(1, 2).await;

    let (claim, uuid) = first_claim(&app).await;
    let token = claim["claim_token"].as_str().unwrap();
    let (_, body) = submit_as(&app, &uuid, token, games_result(2, 1)).await;
    assert_eq!(body["accepted"], true, "{body}");
    let expected = birdtest::auth::public_anon_id(uuid.parse().unwrap());

    let public = [
        "/api/workers".to_string(),
        format!("/api/jobs/{job}"),
        format!("/api/jobs/{job}/results"),
        format!("/api/jobs/{job}/results?worker={expected}"),
    ];
    for path in &public {
        let (status, body) = send(&app, get_request(path, &[])).await;
        assert_eq!(status, StatusCode::OK, "{path}: {body}");
        let text = body.to_string();
        assert!(!text.contains(&uuid), "{path} publishes the worker's UUID: {text}");
        assert!(text.contains(&expected), "{path} does not name the worker by pseudonym: {text}");
    }

    let admin = db.user("root", true).await;
    let (status, body) =
        send(&app, get_request("/api/admin/workers", &admin_headers(&state.cfg, admin))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["items"][0]["anon_uuid"], uuid, "the admin listing carries the UUID to ban");
    let (status, _) = send(&app, get_request("/api/admin/workers", &[])).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
