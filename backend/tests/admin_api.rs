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
    let deleted_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_log WHERE action = 'job.deleted' AND target_id = $1",
    )
    .bind(job.to_string())
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(deleted_rows, 1, "the job's deletion stays in the log after the job is gone");

    // Claims and submissions write no audit rows: `task_claims` already records
    // who claimed and completed what, and when.
    let worker_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_log WHERE action IN ('task.claimed', 'result.submitted')",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(worker_rows, 0);
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

    // The progress totals describe results this purge deleted, so they go
    // back to zero with the dispatch counter.
    let (tasks, issued, games, racks): (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM tasks WHERE job_id = $1),
                claims_issued, games_completed, racks_analyzed
         FROM jobs WHERE id = $1",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!((tasks, issued, games, racks), (0, 0, 0, 0));
}

/// Captured positions, their moves and their plies go with the job when it is
/// purged, through the record: `position_analysis_moves` used to carry its own
/// `task_id` with a cascade of its own, and that column had no index, so every
/// task a purge deleted scanned the whole moves table -- the largest in the
/// schema -- to find the rows the record cascade was about to delete anyway.
#[tokio::test]
async fn purging_a_job_removes_its_captured_positions_through_the_record() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 2).await;
    sqlx::query("UPDATE job_game_config SET capture_positions = true WHERE job_id = $1")
        .bind(job)
        .execute(&db.pool)
        .await
        .unwrap();
    let state = db.state().await;
    let app = birdtest::app(state.clone());

    let (status, assignment) =
        send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{assignment}");
    let uuid = assignment["worker_uuid"].as_str().unwrap();
    let mut result = games_result(2, 1);
    result["positions"] = json!([
        { "game_index": 0, "turn_number": 0, "rack": "AEINRST", "position": "cgp-0",
          "num_moves": 40,
          "moves": [{ "move": "8D RETAINS", "score": 74, "equity": 81.2,
                      "win_percentage": 55.0, "blended_utility": 0.6,
                      "plies": [{ "ply": 0, "bingo_percentage": 0.0, "average_score": 24.0 }] }] },
    ]);
    let (_, body) = send(
        &app,
        post_json(
            "/api/worker/result",
            &[("x-worker-uuid", uuid)],
            json!({ "claim_token": assignment["claim_token"], "result": result }),
        ),
    )
    .await;
    assert_eq!(body, json!({ "accepted": true }));

    let counts = || async {
        sqlx::query_as::<_, (i64, i64, i64)>(
            "SELECT (SELECT COUNT(*) FROM position_analysis_records),
                    (SELECT COUNT(*) FROM position_analysis_moves),
                    (SELECT COUNT(*) FROM position_analysis_plies)",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap()
    };
    assert_eq!(counts().await, (1, 1, 1));

    let admin = db.user("root", true).await;
    let headers = admin_headers(&state.cfg, admin);
    let (status, body) =
        send(&app, request("POST", &format!("/api/admin/jobs/{job}/purge"), &headers)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(counts().await, (0, 0, 0), "record, moves and plies all cascade from the task");
}

/// A staged import the admin never confirmed is a proposal about a vocabulary
/// that has since moved on, holding the bytes of every distribution and layout
/// it staged. After a day it is expired: its rows go, and the import says why,
/// rather than vanishing from the page.
#[tokio::test]
async fn an_unconfirmed_import_expires_after_a_day_and_says_so() {
    let db = TestDb::new().await;
    let admin = db.user("root", true).await;
    let insert = |state: &'static str, age: &'static str| {
        let pool = db.pool.clone();
        async move {
            let id: Uuid = sqlx::query_scalar(
                "INSERT INTO input_data_imports
                     (tarball_date, commit_sha, state, requested_by, requested_at)
                 VALUES ('20251004', repeat('a', 40), $1, $2, now() - $3::interval)
                 RETURNING id",
            )
            .bind(state)
            .bind(admin)
            .bind(age)
            .fetch_one(&pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO input_data_import_rows
                     (import_id, path, role, name, sha256, bytes, disposition, content)
                 VALUES ($1, 'letterdistributions/english.csv', 'letterdist', 'english',
                         repeat('b', 64), 5, 'new', 'bytes'::bytea)",
            )
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
            id
        }
    };
    let stale = insert("staged", "25 hours").await;
    let fresh = insert("staged", "23 hours").await;
    let running = insert("running", "25 hours").await;
    let confirmed = insert("confirmed", "25 hours").await;

    assert_eq!(birdtest::inputdata::expire_unconfirmed_imports(&db.pool).await.unwrap(), 1);

    let state_and_rows = |id: Uuid| {
        let pool = db.pool.clone();
        async move {
            sqlx::query_as::<_, (String, Option<String>, i64)>(
                "SELECT state, error,
                        (SELECT COUNT(*) FROM input_data_import_rows r WHERE r.import_id = i.id)
                 FROM input_data_imports i WHERE i.id = $1",
            )
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    let (state, error, rows) = state_and_rows(stale).await;
    assert_eq!((state.as_str(), rows), ("cancelled", 0));
    assert!(error.unwrap_or_default().contains("24 hours"));
    // Younger than a day, still running, or already confirmed: untouched.
    for (id, expected) in [(fresh, "staged"), (running, "running"), (confirmed, "confirmed")] {
        let (state, _, rows) = state_and_rows(id).await;
        assert_eq!((state.as_str(), rows), (expected, 1));
    }
    // And a second sweep finds nothing left to expire.
    assert_eq!(birdtest::inputdata::expire_unconfirmed_imports(&db.pool).await.unwrap(), 0);
}

/// Bug: `audit_log.actor_user_id`, `player_configs.created_by` and
/// `worker_bans.banned_by` all referenced `users` with no ON DELETE clause, so
/// a user who had registered (and so has a `user.registered` row), created a
/// config or issued a ban could not be deleted.
///
/// Deletion anonymizes rather than removing. Personal data and credentials go;
/// the row,
/// and with it every claim and result, stays.
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
    sqlx::query("INSERT INTO api_keys (user_id, key_hash) VALUES ($1, 'hash')")
        .bind(doomed)
        .execute(&db.pool)
        .await
        .unwrap();
    let doomed_session = admin_headers(&state.cfg, doomed);
    let admin = db.user("root", true).await;

    let headers = admin_headers(&state.cfg, admin);
    let (status, body) =
        send(&app, request("DELETE", &format!("/api/admin/users/{doomed}"), &headers)).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    let (username, email, is_admin, deleted): (String, String, bool, bool) = sqlx::query_as(
        "SELECT username, email, is_admin, deleted_at IS NOT NULL FROM users WHERE id = $1",
    )
    .bind(doomed)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(deleted, "the row stays, marked deleted");
    assert!(!username.contains("doomed") && !email.contains("doomed"), "{username} {email}");
    assert!(!is_admin);
    let keys: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM api_keys WHERE user_id = $1")
        .bind(doomed)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(keys, 0, "API keys are credentials and go");
    let (status, _) = send(&app, get_request("/api/me", &doomed_session)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "the deleted account's sessions are revoked");
    let (_, users) = send(&app, get_request("/api/users", &[])).await;
    assert!(!users.to_string().contains(&doomed.to_string()), "hidden from the public list");
    let (status, _) =
        send(&app, request("DELETE", &format!("/api/admin/users/{doomed}"), &headers)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "deleting twice finds nothing");

    let created_by: Option<Uuid> =
        sqlx::query_scalar("SELECT created_by FROM player_configs WHERE id = $1")
            .bind(config)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(created_by, Some(doomed), "the config outlives its creator, credited to the tombstone");
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

/// Bulk reads of a job's results are admin operations.
///
/// The stream was public, unpaginated and unmetered, so one HTTP request was
/// enough to start a scan of a job's whole result table — and what it consumes
/// is a pool connection held for as long as the caller keeps reading, out of
/// twenty. The public keeps the paginated endpoint.
#[tokio::test]
async fn the_results_stream_is_admin_only_and_the_paginated_one_is_not() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 10).await;
    let cfg = db.config();
    let admin = db.user("streamadmin", true).await;
    let plain = db.user("plainuser", false).await;
    let app = birdtest::app(db.state().await);

    // The old public path is gone entirely.
    let (status, _) = send(&app, get_request(&format!("/api/jobs/{job}/results/stream"), &[])).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // The admin path needs an admin: anonymous, then a signed-in non-admin.
    let path = format!("/api/admin/jobs/{job}/results/stream");
    let (status, _) = send(&app, get_request(&path, &[])).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = send(&app, get_request(&path, &admin_headers(&cfg, plain))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _) = send(&app, get_request(&path, &admin_headers(&cfg, admin))).await;
    assert_eq!(status, StatusCode::OK);

    // Browsing stays public.
    let (status, _) = send(&app, get_request(&format!("/api/jobs/{job}/results"), &[])).await;
    assert_eq!(status, StatusCode::OK);
}

/// Only a completed job can be exported: an export of a job still taking
/// results would be stale before anyone downloaded it, and nothing would say so.
#[tokio::test]
async fn only_a_completed_job_can_be_exported() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 10).await;
    let cfg = db.config();
    let admin = db.user("exportadmin", true).await;
    let app = birdtest::app(db.state().await);
    let headers = admin_headers(&cfg, admin);

    // Nothing exported yet.
    let (status, _) =
        send(&app, get_request(&format!("/api/admin/jobs/{job}/export"), &headers)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let start = |headers: Vec<(String, String)>| {
        let borrowed: Vec<(&str, &str)> =
            headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        post_json(&format!("/api/admin/jobs/{job}/export"), &borrowed, serde_json::json!({}))
    };

    let (status, body) = send(&app, start(headers.clone())).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["message"].as_str().unwrap().contains("completed"), "{body}");

    sqlx::query("UPDATE jobs SET status = 'completed' WHERE id = $1")
        .bind(job)
        .execute(&db.pool)
        .await
        .unwrap();

    let (status, body) = send(&app, start(headers)).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert!(body["id"].is_string(), "{body}");

    // The row exists from the moment the task is spawned. Its outcome depends
    // on an object store the test config points at a closed port, so the state
    // is whatever the spawned task reached; what is pinned here is that the
    // export was accepted and recorded, not that the upload succeeded.
    let recorded: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM job_exports WHERE job_id = $1")
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(recorded, 1);
}

/// An opening-rack job whose player records only the best move cannot produce
/// a ranked list, and nothing downstream would say so.
///
/// `-r best` is MOVE_RECORD_BEST: move generation keeps the top play and
/// discards the rest, so every rack comes back with exactly one move whatever
/// `num_plays_recorded` says -- and for a simming player there is nothing left
/// for the simulation to choose between. Verified against MAGPIE, which
/// reports "1 of 1 plays" under `-r1 best` and 100 under `-r1 all`. The
/// results look perfectly well-formed, so the misconfiguration is refused
/// where it is made.
#[tokio::test]
async fn an_opening_rack_job_cannot_rank_moves_with_a_best_recorder() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let admin = db.user("root", true).await;
    let headers = admin_headers(&state.cfg, admin);

    let letterdist = db.input_data("letterdist", "english").await;
    let layout = db.input_data("layout", "standard15").await;
    let kwg = db.input_data("kwg", "NWL23").await;
    let klv = db.input_data("klv", "NWL23").await;

    let make_player = |name: &'static str, recorder: &'static str, recorded: i32| {
        let headers = headers.clone();
        let app = app.clone();
        async move {
            let (status, body) = send(
                &app,
                post_json(
                    "/api/admin/player-configs",
                    &headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect::<Vec<_>>(),
                    json!({
                        "name": name, "recorder_type": recorder, "sort_strategy": "equity",
                        "kwg_id": kwg, "klv_id": klv, "num_plays_recorded": recorded,
                    }),
                ),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED, "{body}");
            body["id"].as_str().unwrap().to_string()
        }
    };

    let create_job = |player: String| {
        let headers = headers.clone();
        let app = app.clone();
        async move {
            send(
                &app,
                post_json(
                    "/api/admin/jobs",
                    &headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect::<Vec<_>>(),
                    json!({
                        "job_type": "opening_rack", "variant": "classic",
                        "letterdist_id": letterdist, "layout_id": layout,
                        "player_config_id": player, "racks_per_batch": 10, "rack_size": 2,
                    }),
                ),
            )
            .await
        }
    };

    // Ten moves asked for, one move possible.
    let contradictory = make_player("best-ten", "best", 10).await;
    let (status, body) = create_job(contradictory).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["fields"][0]["field"], "player_config_id", "{body}");

    // "The best opening play for every rack" is a real job, and `best` is
    // exactly the right recorder for it.
    let single = make_player("best-one", "best", 1).await;
    let (status, body) = create_job(single).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // And a recorder that keeps candidates may rank as many as it likes.
    let ranking = make_player("all-ten", "all", 10).await;
    let (status, body) = create_job(ranking).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

/// An identity's contribution counter spans every job it ever worked on, so a
/// job whose claims are destroyed has to hand back exactly what it contributed.
///
/// This is the half of the counter design that the `jobs` counters do not have:
/// those belong to the job and a purge simply zeroes them. Left undone, a purge
/// or a delete leaves every contributor on that job reading permanently high on
/// the leaderboard, with nothing to say why.
#[tokio::test]
async fn purging_and_deleting_a_job_give_back_what_it_earned() {
    for destroy in ["purge", "delete"] {
        let db = TestDb::new().await;
        let job = db.games_job(1, 2).await;
        let state = db.state().await;
        let app = birdtest::app(state.clone());
        with_history(&app).await;
        let admin = db.user("root", true).await;
        let headers = admin_headers(&state.cfg, admin);

        let contributed = || async {
            sqlx::query_scalar::<_, i64>(
                "SELECT COALESCE(SUM(tasks_completed), 0)::bigint FROM anonymous_workers",
            )
            .fetch_one(&db.pool)
            .await
            .unwrap()
        };
        assert_eq!(contributed().await, 1, "the worker's task is on its counter");

        let (status, body) = match destroy {
            "purge" => {
                send(&app, post_json(
                    &format!("/api/admin/jobs/{job}/purge"),
                    &headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect::<Vec<_>>(),
                    json!({}),
                ))
                .await
            }
            _ => send(&app, request("DELETE", &format!("/api/admin/jobs/{job}"), &headers)).await,
        };
        assert!(status.is_success(), "{destroy}: {body}");
        assert_eq!(
            contributed().await,
            0,
            "{destroy}: the claims are gone, so the contribution must be too"
        );
    }
}

/// A purge must count what a job's contributors earned *after* every submission
/// in flight has landed, or the one that lands in between is never handed back.
///
/// The purge used to count contributions straight away and then delete the
/// claims, waiting on a submission's claim lock only at the delete. A
/// submission that committed in that gap credited its worker for a claim the
/// purge went on to destroy, so the leaderboard read high for good. Here the
/// "submission" is a transaction holding its claim, marked completed and
/// credited but not yet committed, while the purge runs.
#[tokio::test]
async fn a_purge_waits_for_a_submission_in_flight_before_counting_contributions() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 2).await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let admin = db.user("root", true).await;
    let headers = admin_headers(&state.cfg, admin);

    let (status, claim) =
        send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{claim}");
    let token: Uuid = claim["claim_token"].as_str().unwrap().parse().unwrap();
    let uuid: Uuid = claim["worker_uuid"].as_str().unwrap().parse().unwrap();

    let mut submission = db.pool.begin().await.unwrap();
    sqlx::query("SELECT 1 FROM task_claims WHERE claim_token = $1 FOR UPDATE")
        .bind(token)
        .execute(&mut *submission)
        .await
        .unwrap();
    sqlx::query("UPDATE task_claims SET state = 'completed', completed_at = now() WHERE claim_token = $1")
        .bind(token)
        .execute(&mut *submission)
        .await
        .unwrap();
    sqlx::query("UPDATE anonymous_workers SET tasks_completed = tasks_completed + 1 WHERE uuid = $1")
        .bind(uuid)
        .execute(&mut *submission)
        .await
        .unwrap();

    let purge = {
        let app = app.clone();
        let headers = headers.clone();
        tokio::spawn(async move {
            send(
                &app,
                post_json(
                    &format!("/api/admin/jobs/{job}/purge"),
                    &headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect::<Vec<_>>(),
                    json!({}),
                ),
            )
            .await
        })
    };
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    submission.commit().await.unwrap();

    let (status, body) = purge.await.unwrap();
    assert!(status.is_success(), "{body}");
    let contributed: i64 =
        sqlx::query_scalar("SELECT tasks_completed FROM anonymous_workers WHERE uuid = $1")
            .bind(uuid)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(contributed, 0, "the submission that landed mid-purge was handed back too");
}

/// A completed job is not exported while its last claims are still out.
///
/// Completion does not stop results arriving: a job that met its stopping rule
/// still accepts every claim already issued. An export built in that window
/// was short, and every later download of the job was redirected to it.
#[tokio::test]
async fn a_completed_job_is_not_exported_until_its_claims_have_landed() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 2).await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let admin = db.user("root", true).await;
    let headers = admin_headers(&state.cfg, admin);
    let headers: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();

    let (status, claim) =
        send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{claim}");
    sqlx::query("UPDATE jobs SET status = 'completed' WHERE id = $1")
        .bind(job)
        .execute(&db.pool)
        .await
        .unwrap();

    let export = || post_json(&format!("/api/admin/jobs/{job}/export"), &headers, json!({}));
    let (status, body) = send(&app, export()).await;
    assert_eq!(status, StatusCode::CONFLICT, "a claim is still in flight: {body}");

    // The claim lapses; nothing can be issued against a completed job, so the
    // results are now fixed.
    sqlx::query("UPDATE task_claims SET state = 'abandoned'")
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, body) = send(&app, export()).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
}

/// A pool's public rating history is thinned rather than returned whole: a pool
/// with an active job is refit every two minutes, forever.
#[tokio::test]
async fn a_long_rating_history_is_thinned_but_keeps_its_ends() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let admin = db.user("root", true).await;
    let anchor = db.static_player("anchor", admin).await;
    let letterdist = db.input_data("letterdist", "english").await;
    let layout = db.input_data("layout", "standard15").await;

    let pool: Uuid = sqlx::query_scalar(
        "INSERT INTO rating_pools (name, variant, letterdist_id, layout_id, anchor_player_config_id)
         VALUES ('pool', 'classic', $1, $2, $3) RETURNING id",
    )
    .bind(letterdist)
    .bind(layout)
    .bind(anchor)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    // Twelve hundred runs, two minutes apart: under two days of one active job.
    sqlx::query(
        "WITH runs AS (
             INSERT INTO rating_runs (pool_id, computed_at, trigger, iterations, converged,
                                      pairs_used, jobs_used)
             SELECT $1, timestamptz '2026-01-01' + g * interval '2 minutes', 'evidence', 1,
                    true, g, 1
             FROM generate_series(0, 1199) g
             RETURNING id
         )
         INSERT INTO player_config_ratings
             (run_id, player_config_id, rating, stderr, pairs_played, connected_to_anchor, is_anchor)
         SELECT id, $2, 2000, 0, 0, true, true FROM runs",
    )
    .bind(pool)
    .bind(anchor)
    .execute(&db.pool)
    .await
    .unwrap();

    let (status, body) =
        send(&app, get_request(&format!("/api/rating-pools/{pool}/history"), &[])).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let points = body.as_array().unwrap();
    assert!(points.len() <= 501, "thinned to the cap: {}", points.len());
    assert!(points.len() >= 400, "but not thinned to nothing: {}", points.len());
    assert_eq!(points.first().unwrap()["computed_at"], json!("2026-01-01T00:00:00Z"));
    assert_eq!(
        points.last().unwrap()["computed_at"],
        json!("2026-01-02T15:58:00Z"),
        "the newest run is always kept"
    );
}

/// Runs older than a month are thinned to the last of each day, so a pool's
/// tables are bounded while its history chart keeps its shape and its ends:
/// the first run, each old day's last and every recent run survive, and a
/// deleted run's ratings and residuals go with it.
#[tokio::test]
async fn old_rating_runs_are_thinned_to_the_last_of_each_day() {
    let db = TestDb::new().await;
    let admin = db.user("root", true).await;
    let anchor = db.static_player("anchor", admin).await;
    let rival = db.static_player("rival", admin).await;
    let letterdist = db.input_data("letterdist", "english").await;
    let layout = db.input_data("layout", "standard15").await;
    let pool: Uuid = sqlx::query_scalar(
        "INSERT INTO rating_pools (name, variant, letterdist_id, layout_id, anchor_player_config_id)
         VALUES ('pool', 'classic', $1, $2, $3) RETURNING id",
    )
    .bind(letterdist)
    .bind(layout)
    .bind(anchor)
    .fetch_one(&db.pool)
    .await
    .unwrap();

    // The pool's first run, alone on its day; two old days of three runs each;
    // and three runs from the last few minutes. Every run carries a rating and
    // a residual.
    sqlx::query(
        "WITH runs AS (
             INSERT INTO rating_runs (pool_id, computed_at, trigger, iterations, converged,
                                      pairs_used, jobs_used)
             SELECT $1, t, 'evidence', 1, true, 0, 1
             FROM unnest(ARRAY[
                 timestamptz '2025-12-01 00:00Z',
                 timestamptz '2026-01-01 10:00Z', timestamptz '2026-01-01 10:02Z',
                 timestamptz '2026-01-01 10:04Z',
                 timestamptz '2026-01-02 10:00Z', timestamptz '2026-01-02 10:02Z',
                 timestamptz '2026-01-02 10:04Z',
                 now() - interval '6 minutes', now() - interval '4 minutes',
                 now() - interval '2 minutes'
             ]) AS t
             RETURNING id
         ),
         rated AS (
             INSERT INTO player_config_ratings
                 (run_id, player_config_id, rating, stderr, pairs_played, connected_to_anchor,
                  is_anchor)
             SELECT id, $2, 2000, 0, 0, true, true FROM runs
         )
         INSERT INTO rating_run_residuals
             (run_id, row_player_config_id, col_player_config_id, pairs, actual, predicted)
         SELECT id, $2, $3, 1, 0.5, 0.5 FROM runs",
    )
    .bind(pool)
    .bind(anchor)
    .bind(rival)
    .execute(&db.pool)
    .await
    .unwrap();

    let deleted = birdtest::ratings::thin_old_runs(&db.pool).await.unwrap();
    assert_eq!(deleted, 4, "two of each old day's three runs");

    let old_kept: Vec<String> = sqlx::query_scalar(
        "SELECT to_char(computed_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI')
         FROM rating_runs
         WHERE pool_id = $1 AND computed_at < now() - interval '1 day'
         ORDER BY computed_at",
    )
    .bind(pool)
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        old_kept,
        ["2025-12-01 00:00", "2026-01-01 10:04", "2026-01-02 10:04"],
        "the first run and each old day's last"
    );
    let recent: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM rating_runs
         WHERE pool_id = $1 AND computed_at > now() - interval '1 day'",
    )
    .bind(pool)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(recent, 3, "runs inside the window are untouched");
    let ratings: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM player_config_ratings")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let residuals: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rating_run_residuals")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!((ratings, residuals), (6, 6), "a deleted run's ratings and residuals go with it");

    let again = birdtest::ratings::thin_old_runs(&db.pool).await.unwrap();
    assert_eq!(again, 0, "thinning is idempotent");
}

/// A games job may pit a static player against a simmer: PLAN.md promises "any
/// mix", and it is the configuration a strength comparison most often wants.
///
/// Job creation compared the two players' win% models with plain equality, and
/// a static player has none (a config that names one is refused), so every
/// such job failed as a "disagreement". Two simmers on different models are
/// still refused -- MAGPIE loads one model for the whole run.
#[tokio::test]
async fn a_games_job_may_pit_a_static_player_against_a_simmer() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let admin = db.user("root", true).await;
    let headers = admin_headers(&state.cfg, admin);
    let headers: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();

    let letterdist = db.input_data("letterdist", "english").await;
    let layout = db.input_data("layout", "standard15").await;
    let kwg = db.input_data("kwg", "NWL23").await;
    let klv = db.input_data("klv", "NWL23").await;
    let winpct = db.input_data("winpct", "winpct").await;
    let other_winpct = db.input_data("winpct", "winpct2").await;

    let mut configs = Vec::new();
    for (name, winpct_id) in [("static", None), ("simmer", Some(winpct)), ("simmer2", Some(other_winpct))] {
        let mut body = json!({
            "name": name, "recorder_type": "best", "sort_strategy": "equity",
            "kwg_id": kwg, "klv_id": klv, "num_plays_recorded": 1,
        });
        if let Some(winpct_id) = winpct_id {
            body["winpct_id"] = json!(winpct_id);
            body["num_plies"] = json!(2);
            body["num_plays"] = json!(10);
            body["max_iterations"] = json!(100);
            body["time_limit_secs"] = json!(0);
        }
        let (status, created) = send(&app, post_json("/api/admin/player-configs", &headers, body)).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        configs.push(created["id"].as_str().unwrap().to_string());
    }

    let create = |p1: &str, p2: &str| {
        post_json(
            "/api/admin/jobs",
            &headers,
            json!({
                "job_type": "games", "variant": "classic",
                "letterdist_id": letterdist, "layout_id": layout,
                "player1_config_id": p1, "player2_config_id": p2,
                "min_games": 1, "max_games": 10,
            }),
        )
    };

    for (p1, p2) in [(&configs[0], &configs[1]), (&configs[1], &configs[0])] {
        let (status, body) = send(&app, create(p1, p2)).await;
        assert_eq!(status, StatusCode::CREATED, "static against simmer, either seat: {body}");
    }
    let (status, body) = send(&app, create(&configs[1], &configs[2])).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "two simmers on different models: {body}");
}

/// Creates a player config through the API.
async fn player_config(
    app: &axum::Router,
    headers: &[(&str, &str)],
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    send(app, post_json("/api/admin/player-configs", headers, body)).await
}

/// A simmer is bounded by its iteration budget, never by a time limit: a limit
/// makes how far a simulation gets depend on the contributor's hardware, and a
/// null limit means MAGPIE's 60-second default.
#[tokio::test]
async fn a_simming_player_config_is_bounded_by_iterations_not_time() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let admin = db.user("root", true).await;
    let headers = admin_headers(&state.cfg, admin);
    let headers: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let kwg = db.input_data("kwg", "NWL23").await;
    let klv = db.input_data("klv", "NWL23").await;
    let winpct = db.input_data("winpct", "winpct").await;

    let cases = [
        ("no-limit-stated", json!({ "max_iterations": 100 }), StatusCode::BAD_REQUEST, Some("time_limit_secs")),
        ("a-limit", json!({ "max_iterations": 100, "time_limit_secs": 30 }), StatusCode::BAD_REQUEST, Some("time_limit_secs")),
        ("no-budget", json!({ "time_limit_secs": 0 }), StatusCode::BAD_REQUEST, Some("max_iterations")),
        ("bounded", json!({ "max_iterations": 100, "time_limit_secs": 0 }), StatusCode::CREATED, None),
    ];
    for (name, extra, expected, field) in cases {
        let mut body = json!({
            "name": name, "recorder_type": "best", "kwg_id": kwg, "klv_id": klv,
            "winpct_id": winpct, "num_plies": 2, "num_plays": 10, "num_plays_recorded": 1,
        });
        for (key, value) in extra.as_object().unwrap() {
            body[key] = value.clone();
        }
        let (status, response) = player_config(&app, &headers, body).await;
        assert_eq!(status, expected, "{name}: {response}");
        if let Some(field) = field {
            assert_eq!(response["fields"][0]["field"], field, "{name}: {response}");
        }
    }
}

/// With capture on, MAGPIE raises a simmer's candidate count to the capture cap,
/// so a capture job whose simmer considers fewer plays would play different
/// games from the same job without capture.
#[tokio::test]
async fn a_capture_job_refuses_simmers_that_capture_would_change() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let admin = db.user("root", true).await;
    let headers = admin_headers(&state.cfg, admin);
    let headers: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let letterdist = db.input_data("letterdist", "english").await;
    let layout = db.input_data("layout", "standard15").await;
    let kwg = db.input_data("kwg", "NWL23").await;
    let klv = db.input_data("klv", "NWL23").await;
    let winpct = db.input_data("winpct", "winpct").await;

    let (_, capturing) = player_config(&app, &headers, json!({
        "name": "static-capturing-20", "recorder_type": "best", "sort_strategy": "equity",
        "kwg_id": kwg, "klv_id": klv, "num_plays_recorded": 20,
    })).await;
    let mut simmers = Vec::new();
    for plays in [10, 20] {
        let (status, created) = player_config(&app, &headers, json!({
            "name": format!("simmer-{plays}"), "recorder_type": "best", "kwg_id": kwg,
            "klv_id": klv, "winpct_id": winpct, "num_plies": 2, "num_plays": plays,
            "max_iterations": 100, "time_limit_secs": 0, "num_plays_recorded": 1,
        })).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        simmers.push(created["id"].clone());
    }

    let create = |p2: &serde_json::Value, capture: bool| {
        post_json("/api/admin/jobs", &headers, json!({
            "job_type": "games", "variant": "classic",
            "letterdist_id": letterdist, "layout_id": layout,
            "player1_config_id": capturing["id"], "player2_config_id": p2,
            "min_games": 1, "max_games": 10, "capture_positions": capture,
        }))
    };
    let (status, body) = send(&app, create(&simmers[0], true)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["fields"][0]["field"], "capture_positions", "{body}");
    let (status, body) = send(&app, create(&simmers[0], false)).await;
    assert_eq!(status, StatusCode::CREATED, "without capture nothing is raised: {body}");
    let (status, body) = send(&app, create(&simmers[1], true)).await;
    assert_eq!(status, StatusCode::CREATED, "a simmer already at the cap: {body}");
}

/// A config states every setting a task needs, and a job every run-wide one:
/// what the body leaves out is written in from MAGPIE's defaults at creation.
/// A null used to mean "the worker's compile-time default", so a result
/// depended on which MAGPIE release ran it. A static player states no
/// simulation settings at all, and one that sets one is refused.
#[tokio::test]
async fn a_player_config_and_a_job_state_every_setting_a_task_needs() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let admin = db.user("root", true).await;
    let headers = admin_headers(&state.cfg, admin);
    let headers: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let letterdist = db.input_data("letterdist", "english").await;
    let layout = db.input_data("layout", "standard15").await;
    let kwg = db.input_data("kwg", "NWL23").await;
    let klv = db.input_data("klv", "NWL23").await;
    let winpct = db.input_data("winpct", "winpct").await;

    let (status, static_player) = player_config(&app, &headers, json!({
        "name": "static", "recorder_type": "best", "kwg_id": kwg, "klv_id": klv,
        "num_plays_recorded": 1,
    })).await;
    assert_eq!(status, StatusCode::CREATED, "{static_player}");
    for (field, expected) in [
        ("sort_strategy", json!("equity")),
        ("num_plies", json!(0)),
        ("num_plays", json!(100)),
        ("num_plies_recorded", json!(2)),
        ("movegen_margin", json!(5.0)),
        ("use_wordmap", json!(false)),
        ("use_rit", json!(false)),
        ("max_iterations", json!(null)),
        ("threshold", json!(null)),
        ("utility_w_spread", json!(null)),
    ] {
        assert_eq!(static_player[field], expected, "static {field}: {static_player}");
    }

    // A simmer keeps what it states and gets MAGPIE's value for the rest.
    let (status, simmer) = player_config(&app, &headers, json!({
        "name": "simmer", "recorder_type": "best", "kwg_id": kwg, "klv_id": klv,
        "winpct_id": winpct, "num_plies": 2, "max_iterations": 100, "time_limit_secs": 0,
        "threshold": "none", "num_plays_recorded": 1,
    })).await;
    assert_eq!(status, StatusCode::CREATED, "{simmer}");
    for (field, expected) in [
        ("threshold", json!("none")),
        ("sampling_rule", json!("top_two_ids")),
        ("stopping_pct", json!(99.0)),
        ("use_inference", json!(true)),
        ("min_play_iterations", json!(500)),
        ("inference_margin", json!(5.0)),
        ("utility_w_winpct", json!(1.0)),
        ("utility_w_spread", json!(0.5)),
        ("utility_spread_scale", json!(100.0)),
        ("num_plays", json!(100)),
    ] {
        assert_eq!(simmer[field], expected, "simmer {field}: {simmer}");
    }

    // A static player with a simulation setting would carry it on every
    // request for nothing to read.
    let (status, body) = player_config(&app, &headers, json!({
        "name": "static-with-threshold", "recorder_type": "best", "kwg_id": kwg,
        "klv_id": klv, "threshold": "gk16", "num_plays_recorded": 1,
    })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["fields"][0]["field"], "num_plies", "{body}");

    let (status, created) = send(&app, post_json("/api/admin/jobs", &headers, json!({
        "job_type": "games", "variant": "classic",
        "letterdist_id": letterdist, "layout_id": layout,
        "player1_config_id": static_player["id"], "player2_config_id": simmer["id"],
        "min_games": 1, "max_games": 10,
    }))).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["job"]["bingo_bonus"], json!(50), "{created}");
    assert_eq!(created["job"]["sim_cutoff"], json!(0.005), "{created}");
}

/// Banning is the only lever there is against a bad contributor — nothing bans
/// automatically — so unban has to mean what it says.
///
/// It deletes by row id, so a second ban row for the same identity used to
/// leave the identity banned after the admin had lifted the ban, with nothing
/// in the response to say so. A duplicate is refused instead.
#[tokio::test]
async fn an_identity_can_be_banned_once_and_unbanning_lifts_it() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let admin = db.user("root", true).await;
    let headers = admin_headers(&state.cfg, admin);
    let header_refs: Vec<(&str, &str)> =
        headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let target = db.user("nuisance", false).await;

    let ban = |reason: &'static str| {
        let app = app.clone();
        let refs = header_refs.clone();
        async move {
            send(
                &app,
                post_json("/api/admin/workers/ban", &refs, json!({ "user_id": target, "reason": reason })),
            )
            .await
        }
    };

    let (status, body) = ban("submitting garbage").await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let ban_id = body["id"].as_str().unwrap().to_string();

    let (status, body) = ban("and again").await;
    assert_eq!(status, StatusCode::CONFLICT, "a second ban of one identity: {body}");

    let (status, body) =
        send(&app, request("DELETE", &format!("/api/admin/workers/ban/{ban_id}"), &headers)).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    let still_banned: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM worker_bans WHERE user_id = $1")
            .bind(target)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(still_banned, 0, "unban lifted the ban rather than one row of it");

    // And the identity can be banned again afterwards, which is how "ban with a
    // different reason" is expressed.
    let (status, body) = ban("a new reason").await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

/// Bug: the finish check reads a job's results, then completes it. A purge in
/// between left the job `completed` with none of those results, and a completed
/// job cannot be reactivated. The purge zeroes `claims_issued`, which otherwise
/// only grows, so the completion is refused once the counter is below what the
/// check observed before it read.
#[tokio::test]
async fn a_finish_check_overtaken_by_a_purge_does_not_complete_the_job() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 2).await;
    let status = |db: &TestDb| {
        let pool = db.pool.clone();
        async move {
            sqlx::query_scalar::<_, String>("SELECT status::text FROM jobs WHERE id = $1")
                .bind(job)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };

    // The check observed five claims; a purge then reset the counter.
    sqlx::query("UPDATE jobs SET claims_issued = 0 WHERE id = $1")
        .bind(job)
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(!birdtest::jobs::complete_unless_purged(&db.pool, job, 5).await.unwrap());
    assert_eq!(status(&db).await, "active");

    // With no purge in between the counter has only grown, and it completes.
    sqlx::query("UPDATE jobs SET claims_issued = 6 WHERE id = $1")
        .bind(job)
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(birdtest::jobs::complete_unless_purged(&db.pool, job, 5).await.unwrap());
    assert_eq!(status(&db).await, "completed");
}

/// A rack info table carries precomputed leave values that move generation uses
/// in place of the leaves a job pins, which is why it was refused outright
/// until the server could build the table for a config's own (lexicon, leaves)
/// pair and pin its hash. Both values are accepted now; what stops a wrong
/// table being used is the hash and the dispatch gate, not this validator.
#[tokio::test]
async fn a_player_config_may_ask_for_a_rack_info_table() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let admin = db.user("root", true).await;
    let headers = admin_headers(&state.cfg, admin);
    let headers: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let kwg = db.input_data("kwg", "NWL23").await;
    let klv = db.input_data("klv", "NWL23").await;

    for use_rit in [json!(true), json!(false)] {
        let (status, response) = player_config(&app, &headers, json!({
            "name": format!("static-rit-{use_rit}"), "recorder_type": "best",
            "kwg_id": kwg, "klv_id": klv, "num_plays_recorded": 1, "use_rit": use_rit,
        }))
        .await;
        assert_eq!(status, StatusCode::CREATED, "use_rit {use_rit}: {response}");
        // Stored, not merely accepted. While the answer was always "no" the
        // insert bound a literal `false`, so lifting the refusal without this
        // would have produced configs that asked for a table and were written
        // as not wanting one -- silently, and only visible as a job that never
        // loaded the table it was created for.
        assert_eq!(response["use_rit"], use_rit, "{response}");
    }

    // Absent still means no. A table is 1.9 GB on every contributor's disk and
    // minutes of server time; nothing should get one by default.
    let (status, response) = player_config(&app, &headers, json!({
        "name": "static-rit-absent", "recorder_type": "best",
        "kwg_id": kwg, "klv_id": klv, "num_plays_recorded": 1,
    }))
    .await;
    assert_eq!(status, StatusCode::CREATED, "{response}");
    assert_eq!(response["use_rit"], json!(false), "{response}");

    // The table travels under the pair's name, not the lexicon's. That is what
    // keeps NWL23-with-CSW21-leaves -- a configuration birdtest accepts on
    // purpose -- from loading NWL23's own table and ranking every full rack on
    // leaves the job did not pin.
    assert_eq!(
        birdtest::derived::rack_info_table_name("NWL23", "CSW21"),
        "NWL23.CSW21"
    );
}

/// Bug: reclamation is lazy and runs when a worker asks for work from the
/// job's candidate list, which never happens for a completed job. A claim whose
/// worker died therefore stayed `claimed` for good, and the export refused
/// with "at most the heartbeat timeout" for good.
#[tokio::test]
async fn an_export_is_not_blocked_by_a_claim_whose_worker_vanished() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 2).await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let admin = db.user("root", true).await;
    let headers = admin_headers(&state.cfg, admin);
    let headers: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();

    let (status, claim) =
        send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{claim}");
    sqlx::query("UPDATE jobs SET status = 'completed' WHERE id = $1")
        .bind(job)
        .execute(&db.pool)
        .await
        .unwrap();

    let export = || post_json(&format!("/api/admin/jobs/{job}/export"), &headers, json!({}));
    let (status, body) = send(&app, export()).await;
    assert_eq!(status, StatusCode::CONFLICT, "a live claim is still in flight: {body}");

    // The worker vanished: its claim is past the heartbeat timeout, and no
    // claim request will ever reclaim it, since nothing claims from a
    // completed job.
    sqlx::query(
        "UPDATE task_claims SET claimed_at = now() - interval '1 hour', last_heartbeat_at = NULL",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let (status, body) = send(&app, export()).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");

    // Reclaimed through the same path dispatch uses, so the claim and its
    // task read as every other lapsed claim does.
    let (claim_state, active): (String, i32) = sqlx::query_as(
        "SELECT c.state::text, t.active_claim_count
         FROM task_claims c JOIN tasks t ON t.id = c.task_id
         WHERE c.claim_token = $1",
    )
    .bind(Uuid::parse_str(claim["claim_token"].as_str().unwrap()).unwrap())
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!((claim_state.as_str(), active), ("abandoned", 0));
}
