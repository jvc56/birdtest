//! Admin lifecycle endpoints against a real database: the destructive paths
//! that were broken outright, and the lifecycle rule that could be sidestepped.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::*;
use serde_json::json;
use uuid::Uuid;

fn request(method: &str, path: &str, headers: &[(String, String)]) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(path);
    for (name, value) in headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    builder.body(Body::empty()).unwrap()
}

/// Claims and completes one task, so the job has claims, results and audit
/// rows referencing it.
async fn with_history(app: &axum::Router) {
    let (status, assignment) =
        send(app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{assignment}");
    let uuid = assignment["worker_uuid"].as_str().unwrap();
    let (_, body) = send(
        app,
        post_json(
            "/api/worker/result",
            &[("x-worker-uuid", uuid)],
            json!({ "claim_token": assignment["claim_token"], "result": games_result(2, 1) }),
        ),
    )
    .await;
    assert_eq!(body, json!({ "accepted": true }));
}

/// Bug: `audit_log.job_id` referenced `jobs` with no ON DELETE clause, and
/// every job has a `job.created` or `task.claimed` row pointing at it, so
/// deleting any job that had ever done anything failed with a foreign-key
/// violation.
#[tokio::test]
async fn a_job_with_history_can_be_deleted_and_its_census_survives() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 2).await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    with_history(&app).await;
    let admin = db.user("root", true).await;

    let headers = admin_headers(&state.cfg, admin);
    let (status, body) =
        send(&app, request("DELETE", &format!("/api/admin/jobs/{job}"), &headers)).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    let census: String = sqlx::query_scalar(
        "SELECT reason FROM audit_log WHERE action = 'job.deleted.census' AND target_id = $1",
    )
    .bind(job.to_string())
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(census.contains("claims=1"), "{census}");
    let claimed_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM audit_log WHERE job_id = $1 AND action = 'task.claimed'")
            .bind(job)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(claimed_rows, 1, "the job's history stays in the log after the job is gone");
}

/// Bug: the purge census and the purge itself both queried
/// `player_config_ratings.job_id`, a column that does not exist -- ratings are
/// pool-scoped -- so every purge failed.
#[tokio::test]
async fn a_job_can_be_purged_and_its_dispatch_counter_resets() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 2).await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    with_history(&app).await;
    let admin = db.user("root", true).await;

    let headers = admin_headers(&state.cfg, admin);
    let (status, body) =
        send(&app, request("POST", &format!("/api/admin/jobs/{job}/purge"), &headers)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["tasks_reset"], 1);

    let (tasks, issued): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM tasks WHERE job_id = $1), claims_issued FROM jobs WHERE id = $1",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!((tasks, issued), (0, 0));
}

/// Bug: `audit_log.actor_user_id`, `player_configs.created_by` and
/// `worker_bans.banned_by` all referenced `users` with no ON DELETE clause, so
/// a user who had registered (and so has a `user.registered` row), created a
/// config or issued a ban could not be deleted.
#[tokio::test]
async fn a_user_with_history_can_be_deleted() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let doomed = db.user("doomed", true).await;
    let config = db.static_player("theirs", doomed).await;
    sqlx::query(
        "INSERT INTO audit_log (action, actor_user_id, target_type, target_id)
         VALUES ('user.registered', $1, 'user', $1::text)",
    )
    .bind(doomed)
    .execute(&db.pool)
    .await
    .unwrap();
    let banned = db.user("banned", false).await;
    sqlx::query("INSERT INTO worker_bans (user_id, banned_by) VALUES ($1, $2)")
        .bind(banned)
        .bind(doomed)
        .execute(&db.pool)
        .await
        .unwrap();
    let admin = db.user("root", true).await;

    let headers = admin_headers(&state.cfg, admin);
    let (status, body) =
        send(&app, request("DELETE", &format!("/api/admin/users/{doomed}"), &headers)).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    let created_by: Option<Uuid> =
        sqlx::query_scalar("SELECT created_by FROM player_configs WHERE id = $1")
            .bind(config)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(created_by, None, "the config outlives its creator");
    let still_banned: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM worker_bans WHERE user_id = $1")
        .bind(banned)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(still_banned, 1, "deleting the admin who issued a ban does not lift it");
}

/// Bug: deactivation was unconditional, and activation only refuses a job that
/// is *currently* completed -- so deactivate-then-activate restarted a
/// completed job.
#[tokio::test]
async fn a_completed_job_cannot_be_deactivated() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 2).await;
    sqlx::query("UPDATE jobs SET status = 'completed' WHERE id = $1")
        .bind(job)
        .execute(&db.pool)
        .await
        .unwrap();
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let admin = db.user("root", true).await;

    let headers = admin_headers(&state.cfg, admin);
    let (status, body) =
        send(&app, request("POST", &format!("/api/admin/jobs/{job}/deactivate"), &headers)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["message"].as_str().unwrap().contains("completed"), "{body}");

    let status_text: String = sqlx::query_scalar("SELECT status::text FROM jobs WHERE id = $1")
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(status_text, "completed");
}
