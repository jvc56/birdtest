//! The audit log against a real database (`I-AUDIT-*`): what each helper
//! writes, that a log shares its action's fate, and that every destructive
//! admin action leaves exactly its record.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use common::*;
use serde_json::{json, Value};
use uuid::Uuid;

fn request(method: &str, path: &str, headers: &[(String, String)], body: Option<Value>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(path);
    for (name, value) in headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    }
}

#[derive(Debug, PartialEq, sqlx::FromRow)]
struct AuditRow {
    action: String,
    actor_user_id: Option<Uuid>,
    actor_anon_uuid: Option<Uuid>,
    target_type: Option<String>,
    target_id: Option<String>,
    job_id: Option<Uuid>,
    reason: Option<String>,
    old_status: Option<String>,
    new_status: Option<String>,
}

/// Every audit row after `after`, oldest first.
async fn rows_after(db: &TestDb, after: i64) -> Vec<AuditRow> {
    sqlx::query_as(
        "SELECT action, actor_user_id, actor_anon_uuid, target_type, target_id, job_id,
                reason, old_status, new_status
         FROM audit_log WHERE id > $1 ORDER BY id",
    )
    .bind(after)
    .fetch_all(&db.pool)
    .await
    .unwrap()
}

async fn last_id(db: &TestDb) -> i64 {
    sqlx::query_scalar("SELECT COALESCE(MAX(id), 0) FROM audit_log")
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

fn row(action: &str) -> AuditRow {
    AuditRow {
        action: action.into(),
        actor_user_id: None,
        actor_anon_uuid: None,
        target_type: None,
        target_id: None,
        job_id: None,
        reason: None,
        old_status: None,
        new_status: None,
    }
}

/// I-AUDIT-1: each helper in `audit.rs` writes exactly the actor, target type,
/// target id and job id it was given -- and the reason or statuses it carries
/// -- into their own columns, with nothing else filled in.
#[tokio::test]
async fn each_audit_helper_records_who_did_what_to_which_target() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 1).await;
    let actor = db.user("auditor", true).await;
    let anon = Uuid::new_v4();
    let mut conn = db.pool.acquire().await.unwrap();

    birdtest::audit::log(
        &mut conn, "thing.logged", Some(actor), Some(anon), Some("thing"), Some("t-1".into()), Some(job),
    )
    .await
    .unwrap();
    birdtest::audit::log(&mut conn, "thing.bare", None, None, None, None, None).await.unwrap();
    birdtest::audit::log_status_change(&mut conn, "job.activated", actor, job, "inactive", "active")
        .await
        .unwrap();
    birdtest::audit::log_ban(&mut conn, actor, "w-1".into(), Some("spam".into())).await.unwrap();
    birdtest::audit::log_ban(&mut conn, actor, "w-2".into(), None).await.unwrap();
    birdtest::audit::log_detail(&mut conn, "job.census", actor, "job", job.to_string(), Some(job), "n=1".into())
        .await
        .unwrap();
    birdtest::audit::log_detail(&mut conn, "user.census", actor, "user", "u-1".into(), None, "n=2".into())
        .await
        .unwrap();
    birdtest::audit::log_worker_detail(
        &mut conn, "claim.declined", None, Some(anon), "claim", "c-1".into(), Some(job), "no kwg".into(),
    )
    .await
    .unwrap();
    birdtest::audit::log_worker_detail(
        &mut conn, "claim.declined", Some(actor), None, "claim", "c-2".into(), None, "no klv".into(),
    )
    .await
    .unwrap();
    drop(conn);

    let rows = rows_after(&db, 0).await;
    let expected = vec![
        AuditRow {
            actor_user_id: Some(actor),
            actor_anon_uuid: Some(anon),
            target_type: Some("thing".into()),
            target_id: Some("t-1".into()),
            job_id: Some(job),
            ..row("thing.logged")
        },
        row("thing.bare"),
        AuditRow {
            actor_user_id: Some(actor),
            target_type: Some("job".into()),
            target_id: Some(job.to_string()),
            job_id: Some(job),
            old_status: Some("inactive".into()),
            new_status: Some("active".into()),
            ..row("job.activated")
        },
        AuditRow {
            actor_user_id: Some(actor),
            target_type: Some("worker".into()),
            target_id: Some("w-1".into()),
            reason: Some("spam".into()),
            ..row("worker.banned")
        },
        AuditRow {
            actor_user_id: Some(actor),
            target_type: Some("worker".into()),
            target_id: Some("w-2".into()),
            ..row("worker.banned")
        },
        AuditRow {
            actor_user_id: Some(actor),
            target_type: Some("job".into()),
            target_id: Some(job.to_string()),
            job_id: Some(job),
            reason: Some("n=1".into()),
            ..row("job.census")
        },
        AuditRow {
            actor_user_id: Some(actor),
            target_type: Some("user".into()),
            target_id: Some("u-1".into()),
            reason: Some("n=2".into()),
            ..row("user.census")
        },
        AuditRow {
            actor_anon_uuid: Some(anon),
            target_type: Some("claim".into()),
            target_id: Some("c-1".into()),
            job_id: Some(job),
            reason: Some("no kwg".into()),
            ..row("claim.declined")
        },
        AuditRow {
            actor_user_id: Some(actor),
            target_type: Some("claim".into()),
            target_id: Some("c-2".into()),
            reason: Some("no klv".into()),
            ..row("claim.declined")
        },
    ];
    assert_eq!(rows, expected);
}

/// I-AUDIT-2: an audit row shares its caller's transaction, so one written
/// inside a transaction that rolls back does not persist -- directly, and
/// through the application: deleting a user who does not exist writes its
/// census before it finds that out, and the census goes with the refusal.
#[tokio::test]
async fn an_audit_row_in_a_rolled_back_transaction_does_not_persist() {
    let db = TestDb::new().await;
    let actor = db.user("auditor", true).await;

    let mut tx = db.pool.begin().await.unwrap();
    birdtest::audit::log(&mut tx, "never.happened", Some(actor), None, None, None, None)
        .await
        .unwrap();
    birdtest::audit::log_detail(&mut tx, "never.census", actor, "job", "x".into(), None, "n=1".into())
        .await
        .unwrap();
    let inside: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(inside, 2, "visible inside the transaction");
    tx.rollback().await.unwrap();
    assert!(rows_after(&db, 0).await.is_empty(), "and gone with it");

    // A dropped transaction -- an early return with `?` -- is a rollback too.
    let state = db.state().await;
    let headers = admin_headers(&state.cfg, actor);
    let app = birdtest::app(state);
    let ghost = Uuid::new_v4();
    let (status, body) =
        send(&app, request("DELETE", &format!("/api/admin/users/{ghost}"), &headers, None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(
        rows_after(&db, 0).await.is_empty(),
        "the census of a deletion that did not happen does not survive it"
    );
}

/// A rating pool anchored on one config with a second member, over plain SQL.
async fn pool_with_member(db: &TestDb, admin: Uuid) -> (Uuid, Uuid) {
    let anchor = db.static_player("anchor", admin).await;
    let member = db.static_player("member", admin).await;
    let ld = db.input_data("letterdist", "english").await;
    let layout = db.input_data("layout", "standard15").await;
    let pool: Uuid = sqlx::query_scalar(
        "INSERT INTO rating_pools (name, variant, letterdist_id, layout_id, anchor_player_config_id)
         VALUES ('pool', 'classic', $1, $2, $3) RETURNING id",
    )
    .bind(ld)
    .bind(layout)
    .bind(anchor)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    for config in [anchor, member] {
        sqlx::query("INSERT INTO rating_pool_members (pool_id, player_config_id) VALUES ($1, $2)")
            .bind(pool)
            .bind(config)
            .execute(&db.pool)
            .await
            .unwrap();
    }
    (pool, member)
}

/// Claims and completes one task, so a job has something to destroy.
async fn with_history(app: &Router) {
    let (status, assignment) =
        send(app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{assignment}");
    let (_, body) = send(
        app,
        post_json(
            "/api/worker/result",
            &[("x-worker-uuid", assignment["worker_uuid"].as_str().unwrap())],
            json!({ "claim_token": assignment["claim_token"], "result": games_result(2, 1) }),
        ),
    )
    .await;
    assert_eq!(body, json!({ "accepted": true }));
}

/// I-AUDIT-3: every destructive admin action -- deactivating, completing,
/// purging and deleting a job, deleting a user, banning and unbanning a
/// worker, deleting an input file, deleting a player config, removing a
/// rating-pool member -- writes exactly one row naming what it did, who did
/// it and to what. The three that destroy recorded work (purge, job delete,
/// user delete) also write their census (I-JOB-10), so they write exactly that
/// pair and nothing more.
///
/// Bug: deleting an input file and deleting a player config wrote nothing, so
/// the log could not say who removed a file a restore would need, or when.
#[tokio::test]
async fn every_destructive_admin_action_writes_exactly_its_record() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let admin = db.user("root", true).await;
    let headers = admin_headers(&state.cfg, admin);
    let app = birdtest::app(state);

    let purged = db.games_job(1, 2).await;
    with_history(&app).await;
    sqlx::query("UPDATE jobs SET status = 'inactive' WHERE id = $1")
        .bind(purged)
        .execute(&db.pool)
        .await
        .unwrap();
    let deleted = db.games_job(1, 2).await;
    with_history(&app).await;
    let lifecycle = db.games_job(1, 2).await;
    let victim = db.user("victim", false).await;
    let banned = db.user("banned", false).await;
    let unused_file = db.input_data("winpct", "unused").await;
    let unused_config = db.static_player("unused", admin).await;
    let (pool, member) = pool_with_member(&db, admin).await;

    let admin_row = |action: &str, target_type: &str, target: String| AuditRow {
        actor_user_id: Some(admin),
        target_type: Some(target_type.into()),
        target_id: Some(target),
        ..row(action)
    };
    let status_row = |action: &str, job: Uuid, from: &str, to: &str| AuditRow {
        job_id: Some(job),
        old_status: Some(from.into()),
        new_status: Some(to.into()),
        ..admin_row(action, "job", job.to_string())
    };
    let census = |action: &str, target_type: &str, target: String, job: Option<Uuid>, reason: &str| AuditRow {
        job_id: job,
        reason: Some(reason.into()),
        ..admin_row(action, target_type, target)
    };

    let actions: Vec<(&str, String, Option<Value>, Vec<AuditRow>)> = vec![
        (
            "POST",
            format!("/api/admin/jobs/{lifecycle}/deactivate"),
            None,
            vec![status_row("job.deactivated", lifecycle, "active", "inactive")],
        ),
        (
            "POST",
            format!("/api/admin/jobs/{lifecycle}/complete"),
            None,
            vec![status_row("job.completed", lifecycle, "inactive", "completed")],
        ),
        (
            "POST",
            format!("/api/admin/jobs/{purged}/purge"),
            None,
            vec![
                census(
                    "job.purged.census", "job", purged.to_string(), Some(purged),
                    "tasks=1 claims=1 game_results=1 leave_records=0 positions=0 rack_progress=0 \
                     staged_results=0 artifacts=0",
                ),
                AuditRow { job_id: Some(purged), ..admin_row("job.purged", "job", purged.to_string()) },
            ],
        ),
        (
            "DELETE",
            format!("/api/admin/jobs/{deleted}"),
            None,
            vec![
                admin_row("job.deleted", "job", deleted.to_string()),
                census(
                    "job.deleted.census", "job", deleted.to_string(), None,
                    "tasks=1 claims=1 game_results=1 leave_records=0 positions=0 rack_progress=0 \
                     staged_results=0 artifacts=0",
                ),
            ],
        ),
        (
            "DELETE",
            format!("/api/admin/users/{victim}"),
            None,
            vec![
                census("user.deleted.census", "user", victim.to_string(), None, "claims=0 accepted=0 api_keys=0"),
                admin_row("user.deleted", "user", victim.to_string()),
            ],
        ),
        (
            "DELETE",
            format!("/api/admin/input-data/{unused_file}"),
            None,
            vec![admin_row("input_data.deleted", "input_data", unused_file.to_string())],
        ),
        (
            "DELETE",
            format!("/api/admin/player-configs/{unused_config}"),
            None,
            vec![admin_row("player_config.deleted", "player_config", unused_config.to_string())],
        ),
        (
            "DELETE",
            format!("/api/admin/rating-pools/{pool}/members/{member}"),
            None,
            vec![admin_row("rating_pool.member_removed", "player_config", member.to_string())],
        ),
    ];

    for (method, path, body, expected) in actions {
        let before = last_id(&db).await;
        let (status, response) = send(&app, request(method, &path, &headers, body)).await;
        assert!(status.is_success(), "{method} {path}: {status} {response}");
        assert_eq!(rows_after(&db, before).await, expected, "{method} {path}");
    }

    // Ban, then unban by the ban's id: each is one row naming the identity.
    let before = last_id(&db).await;
    let (status, response) = send(
        &app,
        request("POST", "/api/admin/workers/ban", &headers, Some(json!({ "user_id": banned, "reason": "spam" }))),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{response}");
    let ban_id: Uuid = response["id"].as_str().unwrap().parse().unwrap();
    assert_eq!(
        rows_after(&db, before).await,
        vec![AuditRow { reason: Some("spam".into()), ..admin_row("worker.banned", "worker", banned.to_string()) }]
    );
    let before = last_id(&db).await;
    let (status, response) =
        send(&app, request("DELETE", &format!("/api/admin/workers/ban/{ban_id}"), &headers, None)).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{response}");
    assert_eq!(
        rows_after(&db, before).await,
        vec![admin_row("worker.unbanned", "worker", banned.to_string())]
    );
}
