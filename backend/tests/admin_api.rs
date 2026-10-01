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
        { "game_index": 0, "turn_number": 0, "analysis": "sim", "rack": "AEINRST", "position": "cgp-0",
          "num_moves": 40,
          "moves": [{ "move": "8D RETAINS", "score": 74, "equity": 81.2,
                      "win_percentage": 55.0, "blended_utility": 0.6,
                      "plies": [{ "ply": 0, "bingo_percentage": 0.0, "average_score": 24.0 }] }] },
        // A capturing job's result has positions from every game of its batch.
        { "game_index": 1, "turn_number": 0, "analysis": "static", "rack": "AEINRST", "position": "cgp-1",
          "num_moves": 1, "moves": [{ "move": "8D RETAINS", "score": 74, "equity": 81.2 }] },
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
    assert_eq!(counts().await, (2, 2, 1));

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

/// A running job exports a snapshot: it has claims in flight -- an active job
/// always does -- and is exported all the same, where it was refused while it
/// was not completed. A completed job with a claim still out is refused (the
/// admin wants its final corpus, which is not yet fixed).
/// A-ADMIN-15b: the audit row is written with the export's own row, so a
/// refused one logs nothing (logged before it, every refusal read as a start).
#[tokio::test]
async fn a_running_job_exports_a_snapshot() {
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

    let start = || {
        let borrowed: Vec<(&str, &str)> =
            headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        post_json(&format!("/api/admin/jobs/{job}/export"), &borrowed, serde_json::json!({}))
    };
    let logged = || async {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM audit_log WHERE job_id = $1 AND action = 'job.export_started'",
        )
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap()
    };

    // Active, with a claim out.
    let (status, claim) =
        send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{claim}");
    let (status, body) = send(&app, start()).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert!(body["id"].is_string(), "{body}");

    // The row exists from the moment the task is spawned. Its outcome depends
    // on an object store the test config points at a closed port, so the state
    // is whatever the spawned task reached; what is pinned here is that the
    // export was accepted and recorded, not that the upload succeeded
    // (`exports.rs` builds one against a real store).
    let recorded: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM job_exports WHERE job_id = $1")
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(recorded, 1);
    assert_eq!(logged().await, 1, "one begun export, one row");

    // Completed with that claim still out: refused, and not logged.
    sqlx::query("UPDATE jobs SET status = 'completed' WHERE id = $1")
        .bind(job)
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, body) = send(&app, start()).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["message"].as_str().unwrap().contains("in flight"), "{body}");
    assert_eq!(logged().await, 1, "a refused export is not logged as started");
}

/// An opening-rack job whose static player records only the best move cannot
/// produce a ranked list, and nothing downstream would say so.
///
/// `-r best` is MOVE_RECORD_BEST: move generation keeps the top play and
/// discards the rest, so every rack comes back with exactly one move whatever
/// `num_plays_recorded` says. Verified against MAGPIE, which reports "1 of 1
/// plays" under `-r1 best` and 100 under `-r1 all`. The results look perfectly
/// well-formed, so the misconfiguration is refused where it is made. A
/// simulating player ranks every play up to `num_plays` whatever its recorder,
/// so a `best` simmer is accepted.
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

    let winpct = db.input_data("winpct", "winpct").await;
    let make_player = |name: &'static str, recorder: &'static str, recorded: i32, sim: bool| {
        let headers = headers.clone();
        let app = app.clone();
        async move {
            let mut config = json!({
                "name": name, "recorder_type": recorder, "sort_strategy": "equity",
                "kwg_id": kwg, "klv_id": klv, "num_plays_recorded": recorded,
            });
            if sim {
                config["num_plies"] = json!(2);
                config["num_plays"] = json!(12);
                config["max_iterations"] = json!(100);
                config["time_limit_secs"] = json!(0);
                config["winpct_id"] = json!(winpct);
            }
            let (status, body) = send(
                &app,
                post_json(
                    "/api/admin/player-configs",
                    &headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect::<Vec<_>>(),
                    config,
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
    let contradictory = make_player("best-ten", "best", 10, false).await;
    let (status, body) = create_job(contradictory).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["fields"][0]["field"], "player_config_id", "{body}");

    // "The best opening play for every rack" is a real job, and `best` is
    // exactly the right recorder for it.
    let single = make_player("best-one", "best", 1, false).await;
    let (status, body) = create_job(single).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // And a recorder that keeps candidates may rank as many as it likes.
    let ranking = make_player("all-ten", "all", 10, false).await;
    let (status, body) = create_job(ranking).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // A simulating player ranks every play up to num_plays, whatever its
    // recorder: a `best` simmer asking for ten is a working config.
    let simmer = make_player("best-ten-sim", "best", 10, true).await;
    let (status, body) = create_job(simmer).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // No player, static or simulating, reports more moves than it generates:
    // twenty recorded from twelve candidates would store twelve.
    let short = make_player("all-twenty-sim", "all", 20, true).await;
    let (status, body) = create_job(short).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["fields"][0]["message"].as_str().unwrap().contains("12 plays"), "{body}");
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

        // As if the claim had been held a minute: its contributor credited
        // with the time, which is what the give-back reads off the claim.
        sqlx::query(
            "WITH held AS (
                 UPDATE task_claims SET claimed_at = claimed_at - interval '1 minute'
                 WHERE job_id = $1 RETURNING claimed_by_anon_uuid
             )
             UPDATE anonymous_workers SET compute_ms = compute_ms + 60000
             WHERE uuid IN (SELECT claimed_by_anon_uuid FROM held)",
        )
        .bind(job)
        .execute(&db.pool)
        .await
        .unwrap();
        let contributed = || async {
            sqlx::query_as::<_, (i64, i64, i64, i64)>(
                "SELECT COALESCE(SUM(tasks_completed), 0)::bigint,
                        COALESCE(SUM(compute_ms), 0)::bigint,
                        COALESCE(SUM(games_played), 0)::bigint,
                        COALESCE(SUM(racks_analyzed), 0)::bigint
                 FROM anonymous_workers",
            )
            .fetch_one(&db.pool)
            .await
            .unwrap()
        };
        let (tasks, compute_ms, games, racks) = contributed().await;
        assert_eq!((tasks, games, racks), (1, 2, 0), "the worker's task is on its counters");
        assert!(compute_ms >= 60_000, "and the minute it was held: {compute_ms} ms");

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
            (0, 0, 0, 0),
            "{destroy}: the claims are gone, so every counter of the contribution must be too"
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
/// was short, and every later download of the job was redirected to it; it
/// would now be a snapshot, but the admin exporting a completed job wants the
/// final corpus.
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
    // A member, as a pool's anchor always is: history carries members only.
    sqlx::query("INSERT INTO rating_pool_members (pool_id, player_config_id) VALUES ($1, $2)")
        .bind(pool)
        .bind(anchor)
        .execute(&db.pool)
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
                "max_games": 10,
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
        // A simmer's candidates are the top plays by equity in a games job
        // whatever it says, so `score` would mean two players (A-ADMIN-PC-2b).
        (
            "score-simmer",
            json!({ "max_iterations": 100, "time_limit_secs": 0, "sort_strategy": "score" }),
            StatusCode::BAD_REQUEST,
            Some("sort_strategy"),
        ),
        (
            "equity-simmer",
            json!({ "max_iterations": 100, "time_limit_secs": 0, "sort_strategy": "equity" }),
            StatusCode::CREATED,
            None,
        ),
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
            "max_iterations": 100, "time_limit_secs": 0, "num_plays_recorded": 20,
        })).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        simmers.push(created["id"].clone());
    }

    let create = |p2: &serde_json::Value, capture: bool| {
        post_json("/api/admin/jobs", &headers, json!({
            "job_type": "games", "variant": "classic",
            "letterdist_id": letterdist, "layout_id": layout,
            "player1_config_id": capturing["id"], "player2_config_id": p2,
            "max_games": 10, "capture_positions": capture,
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

/// A-ADMIN-26: how many plays and plies a captured position keeps is one
/// setting for the whole run in MAGPIE, read from player 1. So a capturing
/// games or pairs job whose players disagree on either is refused, naming
/// `capture_positions`; without capture neither is read, and they may differ.
/// Keeping only first divergences (`capture_first_divergence`) is a pairs
/// job's, and needs capture: on a games job, or without it, it is refused.
#[tokio::test]
async fn a_capture_job_refuses_players_that_record_differently() {
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
    let mut configs = Vec::new();
    for (name, plays, plies) in [("ten-two", 10, 2), ("ten-two-b", 10, 2), ("five-two", 5, 2), ("ten-four", 10, 4)] {
        let (status, created) = player_config(&app, &headers, json!({
            "name": name, "recorder_type": "all", "kwg_id": kwg, "klv_id": klv,
            "num_plays_recorded": plays, "num_plies_recorded": plies,
        })).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        configs.push(created["id"].clone());
    }
    let create = |job_type: &str, p2: &serde_json::Value, capture: bool| {
        let units = if job_type == "games" { "max_games" } else { "max_pairs" };
        let mut body = json!({
            "job_type": job_type, "variant": "classic",
            "letterdist_id": letterdist, "layout_id": layout,
            "player1_config_id": configs[0], "player2_config_id": p2,
            "capture_positions": capture,
        });
        body[units] = json!(10);
        post_json("/api/admin/jobs", &headers, body)
    };
    for job_type in ["games", "game_pairs"] {
        for differing in [&configs[2], &configs[3]] {
            let (status, body) = send(&app, create(job_type, differing, true)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{job_type}: {body}");
            assert_eq!(body["fields"][0]["field"], "capture_positions", "{body}");
            let (status, body) = send(&app, create(job_type, differing, false)).await;
            assert_eq!(status, StatusCode::CREATED, "{job_type} without capture: {body}");
        }
        let (status, body) = send(&app, create(job_type, &configs[1], true)).await;
        assert_eq!(status, StatusCode::CREATED, "{job_type}, agreeing: {body}");
    }

    // First divergences: a pairs job's, and only with capture.
    let with_divergence = |job_type: &str, capture: bool| {
        let units = if job_type == "games" { "max_games" } else { "max_pairs" };
        let mut body = json!({
            "job_type": job_type, "variant": "classic",
            "letterdist_id": letterdist, "layout_id": layout,
            "player1_config_id": configs[0], "player2_config_id": configs[1],
            "capture_positions": capture, "capture_first_divergence": true,
        });
        body[units] = json!(10);
        post_json("/api/admin/jobs", &headers, body)
    };
    for (job_type, capture) in [("games", true), ("game_pairs", false)] {
        let (status, body) = send(&app, with_divergence(job_type, capture)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{job_type}, capture {capture}: {body}");
        assert_eq!(body["fields"][0]["field"], "capture_first_divergence", "{body}");
    }
    let (status, body) = send(&app, with_divergence("game_pairs", true)).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let stored: bool = sqlx::query_scalar(
        "SELECT capture_first_divergence FROM job_game_pair_config WHERE job_id = $1",
    )
    .bind(body["job"]["id"].as_str().unwrap().parse::<uuid::Uuid>().unwrap())
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(stored);
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
        ("use_wordmap", json!(true)),
        ("use_rit", json!(true)),
        // Off unless asked, as MAGPIE has it.
        ("use_wit", json!(false)),
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
        "max_games": 10,
    }))).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["job"]["bingo_bonus"], json!(50), "{created}");
    assert_eq!(created["job"]["sim_cutoff"], json!(0.005), "{created}");

    // A-ADMIN-27: the two may be stated instead, and are kept as stated.
    let job = |extra: serde_json::Value| {
        let mut body = json!({
            "job_type": "games", "variant": "classic",
            "letterdist_id": letterdist, "layout_id": layout,
            "player1_config_id": static_player["id"], "player2_config_id": simmer["id"],
            "max_games": 10,
        });
        for (key, value) in extra.as_object().unwrap() {
            body[key] = value.clone();
        }
        post_json("/api/admin/jobs", &headers, body)
    };
    let (status, created) = send(&app, job(json!({ "bingo_bonus": 35, "sim_cutoff": 0.5 }))).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["job"]["bingo_bonus"], json!(35), "{created}");
    assert_eq!(created["job"]["sim_cutoff"], json!(0.5), "{created}");
    for (extra, field) in [
        (json!({ "bingo_bonus": -1 }), "bingo_bonus"),
        (json!({ "sim_cutoff": 100.5 }), "sim_cutoff"),
        (json!({ "sim_cutoff": -0.1 }), "sim_cutoff"),
    ] {
        let (status, body) = send(&app, job(extra.clone())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{extra}: {body}");
        assert_eq!(body["fields"][0]["field"], field, "{extra}: {body}");
    }
}

/// A-ADMIN-25: a player config solves the endgame and the pre-endgame
/// exactly as it states. `endgame_plies` 0 solves nothing, and the
/// pre-endgame is refused without it, since PEG scores its emptier scenarios
/// with endgame solves. A PEG setting without the pre-endgame on is refused,
/// as a simulation setting without plies is; what a PEG player leaves out is
/// MAGPIE's default, written into the row.
#[tokio::test]
async fn a_player_config_solves_the_end_of_the_game_only_as_it_states() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let admin = db.user("root", true).await;
    let headers = admin_headers(&state.cfg, admin);
    let headers: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let kwg = db.input_data("kwg", "NWL23").await;
    let klv = db.input_data("klv", "NWL23").await;
    let body = |name: &str, extra: serde_json::Value| {
        let mut body = json!({
            "name": name, "recorder_type": "best", "kwg_id": kwg, "klv_id": klv,
            "num_plays_recorded": 1,
        });
        for (key, value) in extra.as_object().unwrap() {
            body[key] = value.clone();
        }
        body
    };

    // Off unless asked for.
    let (status, plain) = player_config(&app, &headers, body("plain", json!({}))).await;
    assert_eq!(status, StatusCode::CREATED, "{plain}");
    assert_eq!(plain["endgame_plies"], json!(0), "{plain}");
    assert_eq!(plain["peg_max_bag"], json!(0), "{plain}");
    assert_eq!(plain["peg_stage_top_k"], json!(null), "{plain}");

    // The endgame alone states no PEG settings.
    let (status, endgamer) =
        player_config(&app, &headers, body("endgamer", json!({ "endgame_plies": 4 }))).await;
    assert_eq!(status, StatusCode::CREATED, "{endgamer}");
    assert_eq!(endgamer["endgame_plies"], json!(4), "{endgamer}");
    assert_eq!(endgamer["peg_nested"], json!(null), "{endgamer}");

    // The pre-endgame with only its switch takes MAGPIE's schedule.
    let (status, pegger) = player_config(&app, &headers, body("pegger", json!({
        "endgame_plies": 6, "peg_max_bag": 2,
    }))).await;
    assert_eq!(status, StatusCode::CREATED, "{pegger}");
    for (field, expected) in [
        ("peg_max_bag", json!(2)),
        ("peg_stage_top_k", json!([32, 16, 8, 4, 2])),
        ("peg_scenario_stride", json!(1)),
        ("peg_opp_model", json!("rational")),
        ("peg_nested", json!(true)),
        ("peg_nested_cand_caps", json!([8, 4, 2])),
        ("peg_nested_max_depth", json!(1)),
        ("peg_nested_strides", json!([1, 1, 5, 7])),
    ] {
        assert_eq!(pegger[field], expected, "pegger {field}: {pegger}");
    }

    // Without nested lookahead the nested settings are not stated.
    let (status, flat) = player_config(&app, &headers, body("flat", json!({
        "endgame_plies": 3, "peg_max_bag": 1, "peg_nested": false,
        "peg_stage_top_k": [8, 4], "peg_opp_model": "pessimistic",
    }))).await;
    assert_eq!(status, StatusCode::CREATED, "{flat}");
    assert_eq!(flat["peg_nested_cand_caps"], json!(null), "{flat}");
    assert_eq!(flat["peg_opp_model"], json!("pessimistic"), "{flat}");

    let refused = [
        ("peg-without-endgame", json!({ "peg_max_bag": 2 }), "peg_max_bag"),
        ("peg-with-endgame-off", json!({ "endgame_plies": 0, "peg_max_bag": 2 }), "peg_max_bag"),
        ("too-deep", json!({ "endgame_plies": 26 }), "endgame_plies"),
        ("bag-too-big", json!({ "endgame_plies": 6, "peg_max_bag": 5 }), "peg_max_bag"),
        ("stride-without-peg", json!({ "endgame_plies": 6, "peg_scenario_stride": 2 }), "peg_scenario_stride"),
        (
            "depth-without-nested",
            json!({ "endgame_plies": 6, "peg_max_bag": 2, "peg_nested": false, "peg_nested_max_depth": 2 }),
            "peg_nested_max_depth",
        ),
        ("one-play-stage", json!({ "endgame_plies": 6, "peg_max_bag": 2, "peg_stage_top_k": [4, 1] }), "peg_stage_top_k"),
        ("growing-stages", json!({ "endgame_plies": 6, "peg_max_bag": 2, "peg_stage_top_k": [4, 8] }), "peg_stage_top_k"),
        ("zero-stride", json!({ "endgame_plies": 6, "peg_max_bag": 2, "peg_scenario_stride": 0 }), "peg_scenario_stride"),
        ("unknown-opponent", json!({ "endgame_plies": 6, "peg_max_bag": 2, "peg_opp_model": "nice" }), "peg_opp_model"),
        ("three-strides", json!({ "endgame_plies": 6, "peg_max_bag": 2, "peg_nested_strides": [1, 1, 5] }), "peg_nested_strides"),
    ];
    for (name, extra, field) in refused {
        let (status, response) = player_config(&app, &headers, body(name, extra)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{name}: {response}");
        assert_eq!(response["fields"][0]["field"], field, "{name}: {response}");
    }
}

/// A config or job no worker can run is refused at creation. Each of these
/// used to be accepted and dispatched: every worker then failed every task,
/// and after five failures in a row `magpie contribute` stops — so a job built
/// on one, alone on offer, took the whole fleet down within seconds.
#[tokio::test]
async fn a_config_or_job_no_worker_can_run_is_refused() {
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

    // MAGPIE's own ceilings: a margin past its largest equity, and play
    // counts it would allocate gigabytes for (2e9 was a 16 GB malloc and a
    // core dump) or that a stored rank cannot hold.
    for (field, value) in [
        ("movegen_margin", json!(2_147_483.646)),
        ("inference_margin", json!(2_147_483.646)),
        ("num_plays", json!(200_001)),
        ("num_plays", json!(2_000_000_000)),
        ("num_plays_recorded", json!(32_768)),
    ] {
        let mut body = json!({
            "name": format!("too-big-{field}"), "recorder_type": "best", "kwg_id": kwg,
            "klv_id": klv, "num_plays_recorded": 1,
        });
        body[field] = value;
        let (status, refused) = player_config(&app, &headers, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{field}: {refused}");
        assert_eq!(refused["fields"][0]["field"], field, "{refused}");
    }
    // At the ceilings themselves: MAGPIE's largest equity is a margin it
    // takes, and a static player ranking every play of a blank-heavy opening
    // rack needs more than 32,767 (63,585 for ??EIRST in CSW24).
    let (status, at_ceilings) = player_config(&app, &headers, json!({
        "name": "ceilings", "recorder_type": "all", "kwg_id": kwg, "klv_id": klv,
        "movegen_margin": 2_147_483.645, "num_plays": 63_585, "num_plays_recorded": 32_767,
    })).await;
    assert_eq!(status, StatusCode::CREATED, "{at_ceilings}");
    let (status, at_ceilings) = player_config(&app, &headers, json!({
        "name": "ceilings-simmer", "recorder_type": "best", "kwg_id": kwg, "klv_id": klv,
        "winpct_id": db.input_data("winpct", "winpct").await, "num_plies": 2,
        "max_iterations": 100, "time_limit_secs": 0, "inference_margin": 2_147_483.645, "num_plays": 200_000, "num_plays_recorded": 1,
    })).await;
    assert_eq!(status, StatusCode::CREATED, "{at_ceilings}");
    let (status, player) = player_config(&app, &headers, json!({
        "name": "ok", "recorder_type": "best", "kwg_id": kwg, "klv_id": klv,
        "num_plays_recorded": 1,
    })).await;
    assert_eq!(status, StatusCode::CREATED, "{player}");

    // A 21x21 board: it ships in the data release beside the super-board
    // lexica, and no MAGPIE the fleet runs loads it.
    let mut super_board = b"10, 10\n".to_vec();
    for _ in 0..21 {
        super_board.extend_from_slice(&[b' '; 21]);
        super_board.push(b'\n');
    }
    let super21: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO input_data (path, role, name, sha256, bytes, tarball_date, content)
         VALUES ('layouts/standard21.txt', 'layout', 'standard21', repeat('2', 64), $1, '20260101', $2)
         RETURNING id",
    )
    .bind(super_board.len() as i64)
    .bind(&super_board)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let job_with = |layout_id: uuid::Uuid, alpha: f64, beta: f64| {
        json!({
            "job_type": "games", "variant": "classic",
            "letterdist_id": letterdist, "layout_id": layout_id,
            "player1_config_id": player["id"], "player2_config_id": player["id"],
            "sprt_enabled": true, "min_games": 1, "max_games": 10,
            "sprt_alpha": alpha, "sprt_beta": beta,
        })
    };
    let job = |layout_id: uuid::Uuid, alpha: f64| job_with(layout_id, alpha, 0.05);
    let (status, body) = send(&app, post_json("/api/admin/jobs", &headers, job(super21, 0.05))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["fields"][0]["field"], "layout_id", "{body}");

    // A subnormal alpha made the SPRT's upper bound infinite, which the job
    // page could not print; beta keeps the same floor for symmetry.
    for (alpha, beta, field) in [
        (1e-309, 0.05, "sprt_alpha"),
        (0.000_000_9, 0.05, "sprt_alpha"),
        (0.05, 0.000_000_9, "sprt_beta"),
    ] {
        let (status, body) =
            send(&app, post_json("/api/admin/jobs", &headers, job_with(layout, alpha, beta))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{alpha} {beta}: {body}");
        assert_eq!(body["fields"][0]["field"], field, "{body}");
    }
    let (status, body) =
        send(&app, post_json("/api/admin/jobs", &headers, job_with(layout, 0.000_001, 0.000_001))).await;
    assert_eq!(status, StatusCode::CREATED, "at the floor: {body}");

    let (status, body) = send(&app, post_json("/api/admin/jobs", &headers, job(layout, 0.05))).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
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

    // An identity that does not exist is not "banned": a typo, or a UUID
    // pasted into the wrong kind, would otherwise read as done.
    for body in [
        json!({ "user_id": Uuid::new_v4(), "reason": "typo" }),
        json!({ "anon_uuid": target, "reason": "an account id sent as an anonymous UUID" }),
    ] {
        let (status, response) =
            send(&app, post_json("/api/admin/workers/ban", &header_refs, body.clone())).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}: {response}");
    }
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

    // The check observed five claims; `purged` is the second witness.
    let complete = |db: &TestDb, purged: bool| {
        let pool = db.pool.clone();
        async move {
            let finish = birdtest::jobs::Finish::ReachedTarget;
            birdtest::jobs::complete_unless_purged(&pool, job, 5, finish, || purged).await.unwrap()
        }
    };

    // The check observed five claims; a purge then reset the counter.
    sqlx::query("UPDATE jobs SET claims_issued = 0 WHERE id = $1")
        .bind(job)
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(!complete(&db, false).await);
    assert_eq!(status(&db).await, "active");

    // The counter has grown past what was observed -- but a purge came and
    // went (the second witness): the completion is rolled back.
    sqlx::query("UPDATE jobs SET claims_issued = 6 WHERE id = $1")
        .bind(job)
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(!complete(&db, true).await);
    assert_eq!(status(&db).await, "active");

    // With no purge in between the counter has only grown, and it completes.
    assert!(complete(&db, false).await);
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

    // Absent means yes, as the form has it: a config that does not say gets a
    // table, and says so in what is stored.
    let (status, response) = player_config(&app, &headers, json!({
        "name": "static-rit-absent", "recorder_type": "best",
        "kwg_id": kwg, "klv_id": klv, "num_plays_recorded": 1,
    }))
    .await;
    assert_eq!(status, StatusCode::CREATED, "{response}");
    assert_eq!(response["use_rit"], json!(true), "{response}");

    // A word info table is stored as asked, and is off when nothing says.
    assert_eq!(response["use_wit"], json!(false), "{response}");
    let (status, response) = player_config(&app, &headers, json!({
        "name": "static-wit", "recorder_type": "best",
        "kwg_id": kwg, "klv_id": klv, "num_plays_recorded": 1, "use_wit": true,
    }))
    .await;
    assert_eq!(status, StatusCode::CREATED, "{response}");
    assert_eq!(response["use_wit"], json!(true), "{response}");

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

/// Who gets each of `n` claims, as a count per job, claiming as one fresh
/// anonymous worker after another so no per-identity rule interferes.
async fn claims_by_job(app: &axum::Router, n: usize) -> std::collections::HashMap<String, usize> {
    let mut by_job = std::collections::HashMap::new();
    for _ in 0..n {
        let (status, body) =
            send(app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        *by_job.entry(body["job_id"].as_str().unwrap().to_string()).or_insert(0) += 1;
    }
    by_job
}

/// Bug: the scheduler's deficit was `claims_issued / allocation` over a job's
/// whole life, so a job activated beside one with a long history had a ratio
/// of zero and took *every* claim until it had issued as many -- the older job,
/// at the same 50%, got nothing for as long as that took. A job now joins level
/// with the lowest of the jobs being served (`scheduler::join_at_parity`) and
/// takes its share from then on.
#[tokio::test]
async fn a_newly_activated_job_joins_at_parity_instead_of_taking_everything() {
    let db = TestDb::new().await;
    let cfg = db.config();
    let admin = db.user("root", true).await;
    let veteran = db.games_job(1, 2).await;
    sqlx::query("UPDATE jobs SET claims_issued = 100000 WHERE id = $1")
        .bind(veteran)
        .execute(&db.pool)
        .await
        .unwrap();
    let newcomer = db.games_job(1, 2).await;
    sqlx::query("UPDATE jobs SET status = 'inactive', allocation = NULL WHERE id = $1")
        .bind(newcomer)
        .execute(&db.pool)
        .await
        .unwrap();
    let app = birdtest::app(db.state().await);

    let headers = admin_headers(&cfg, admin);
    let borrowed: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let (status, body) = send(
        &app,
        post_json(&format!("/api/admin/jobs/{newcomer}/activate"), &borrowed, json!({ "allocation": 50 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Twelve at a time: a worker with no identity yet is limited by address,
    // with a burst of thirty.
    let shares = claims_by_job(&app, 12).await;
    assert_eq!(shares.get(&veteran.to_string()), Some(&6), "{shares:?}");
    assert_eq!(shares.get(&newcomer.to_string()), Some(&6), "{shares:?}");

    // Changing a share is an activation too, and the new share holds from
    // there: 75/25 over the next twelve claims, not a lurch to make up for
    // the claims issued under the old one.
    let (status, body) = send(
        &app,
        post_json(&format!("/api/admin/jobs/{veteran}/activate"), &borrowed, json!({ "allocation": 25 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = send(
        &app,
        post_json(&format!("/api/admin/jobs/{newcomer}/activate"), &borrowed, json!({ "allocation": 75 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let shares = claims_by_job(&app, 12).await;
    assert_eq!(shares.get(&veteran.to_string()), Some(&3), "{shares:?}");
    assert_eq!(shares.get(&newcomer.to_string()), Some(&9), "{shares:?}");
}

/// The same for a purge, which zeroes `claims_issued`: the purged job restarts
/// level with the others rather than owed every claim it ever had.
#[tokio::test]
async fn a_purged_job_rejoins_at_parity() {
    let db = TestDb::new().await;
    let cfg = db.config();
    let admin = db.user("root", true).await;
    let steady = db.games_job(1, 2).await;
    let purged = db.games_job(1, 2).await;
    sqlx::query("UPDATE jobs SET claims_issued = 100000 WHERE id = ANY($1)")
        .bind(vec![steady, purged])
        .execute(&db.pool)
        .await
        .unwrap();
    let app = birdtest::app(db.state().await);

    let (status, body) = send(
        &app,
        request("POST", &format!("/api/admin/jobs/{purged}/purge"), &admin_headers(&cfg, admin)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let shares = claims_by_job(&app, 20).await;
    assert_eq!(shares.get(&steady.to_string()), Some(&10), "{shares:?}");
    assert_eq!(shares.get(&purged.to_string()), Some(&10), "{shares:?}");
}

/// Parity is with the jobs *being served*, not with every job on offer. A job
/// can be active and above 0% and still stand still -- its derived files are
/// building, the fleet cannot run it yet -- and its ratio does not move while
/// the others' climb. Put level with that job, a newcomer was first in every
/// candidate list until it had caught up with the jobs that were running: of
/// the next twelve claims it took twelve, and the veteran none. A job counts as
/// served when it issued a claim within the heartbeat timeout
/// (`jobs.last_claimed_at`).
#[tokio::test]
async fn a_job_nobody_is_being_served_from_does_not_set_a_newcomers_parity() {
    let db = TestDb::new().await;
    let cfg = db.config();
    let admin = db.user("root", true).await;
    let veteran = db.games_job(1, 2).await;
    sqlx::query(
        "UPDATE jobs SET claims_issued = 100000, allocation = 40, last_claimed_at = now()
         WHERE id = $1",
    )
    .bind(veteran)
    .execute(&db.pool)
    .await
    .unwrap();
    // On offer, and out of this fleet's reach: a floor above what the workers
    // run. It has issued nothing, so its ratio is zero and stays there.
    let lagging = db.games_job(1, 2).await;
    sqlx::query("UPDATE jobs SET allocation = 20, min_magpie_major = 9 WHERE id = $1")
        .bind(lagging)
        .execute(&db.pool)
        .await
        .unwrap();
    let newcomer = db.games_job(1, 2).await;
    sqlx::query("UPDATE jobs SET status = 'inactive', allocation = NULL WHERE id = $1")
        .bind(newcomer)
        .execute(&db.pool)
        .await
        .unwrap();
    let app = birdtest::app(db.state().await);

    let headers = admin_headers(&cfg, admin);
    let borrowed: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let (status, body) = send(
        &app,
        post_json(&format!("/api/admin/jobs/{newcomer}/activate"), &borrowed, json!({ "allocation": 40 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let shares = claims_by_job(&app, 12).await;
    assert_eq!(shares.get(&veteran.to_string()), Some(&6), "{shares:?}");
    assert_eq!(shares.get(&newcomer.to_string()), Some(&6), "{shares:?}");

    // And a claim is what marks a job as served.
    let stamped: bool = sqlx::query_scalar(
        "SELECT last_claimed_at > now() - interval '1 minute' FROM jobs WHERE id = $1",
    )
    .bind(newcomer)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(stamped, "issuing a claim stamps jobs.last_claimed_at");
}

/// A-ADMIN-15: purging a completed job returns it to inactive and clears the
/// verdict it was completed on. Left completed, it was an empty job nothing
/// could ever run again -- activation refuses a completed job -- where a purge
/// is meant to start it over.
#[tokio::test]
async fn purging_a_completed_job_returns_it_to_inactive() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 2).await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    with_history(&app).await;
    sqlx::query(
        "UPDATE jobs SET status = 'completed', sprt_decided_status = 'passed',
                         sprt_decided_llr = 3.1, sprt_decided_units = 200
         WHERE id = $1",
    )
    .bind(job)
    .execute(&db.pool)
    .await
    .unwrap();
    let admin = db.user("root", true).await;
    let headers = admin_headers(&state.cfg, admin);

    let (status, body) =
        send(&app, request("POST", &format!("/api/admin/jobs/{job}/purge"), &headers)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (job_status, verdict): (String, Option<String>) =
        sqlx::query_as("SELECT status::text, sprt_decided_status FROM jobs WHERE id = $1")
            .bind(job)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!((job_status.as_str(), verdict), ("inactive", None));

    let borrowed: Vec<(&str, &str)> =
        headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let (status, body) = send(
        &app,
        post_json(&format!("/api/admin/jobs/{job}/activate"), &borrowed, json!({ "allocation": 50 })),
    )
    .await;
    assert!(status.is_success(), "and it can run again: {body}");
}

/// A-ADMIN-16: while a purge or delete holds a job's claims, a submission for
/// one of them is answered `503` at once rather than waiting out its lock
/// timeout on a pool connection; and a hold that ends without committing --
/// the request dropped, a deadlock -- spares the job's claims from
/// reclamation for a heartbeat timeout, since their heartbeats were skipped
/// while it held them, not missed.
#[tokio::test]
async fn a_purge_in_progress_neither_parks_submissions_nor_costs_its_claims() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 2).await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());

    let (status, claim) =
        send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{claim}");
    let token = claim["claim_token"].as_str().unwrap().to_string();
    let uuid = claim["worker_uuid"].as_str().unwrap().to_string();

    let hold = state.dispatch_holds.hold(
        job,
        birdtest::jobs::HoldKind::Claims,
        std::time::Duration::from_secs(300),
    );
    let started = std::time::Instant::now();
    let (status, body) = send(
        &app,
        post_json(
            "/api/worker/result",
            &[("x-worker-uuid", uuid.as_str())],
            json!({ "claim_token": token, "result": games_result(2, 1) }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert!(started.elapsed() < std::time::Duration::from_secs(2), "answered at once");

    // The hold ends without a commit, and the claim looks long lapsed.
    drop(hold);
    sqlx::query(
        "UPDATE task_claims SET last_heartbeat_at = now() - interval '1 hour',
                                claimed_at = now() - interval '1 hour'
         WHERE claim_token = $1::uuid",
    )
    .bind(&token)
    .execute(&db.pool)
    .await
    .unwrap();
    assert_eq!(birdtest::scheduler::reclaim_lapsed(&state, &[job]).await.unwrap(), 0);
    let (status, body) = send(
        &app,
        post_json(
            "/api/worker/result",
            &[("x-worker-uuid", uuid.as_str())],
            json!({ "claim_token": token, "result": games_result(2, 1) }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["accepted"], true, "the claim survived the failed purge: {body}");
}

/// A-ADMIN-19: a purge or delete clicked again while one is running is refused
/// with 409, and so are activating, deactivating and completing the job. Each ran to completion on a task of its own, so a re-click
/// stacked a second behind the first's locks, and the first to finish ended
/// the hold the other still relied on.
#[tokio::test]
async fn a_second_purge_or_delete_is_refused_while_one_runs() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 2).await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let admin = db.user("root", true).await;
    let headers = admin_headers(&state.cfg, admin);
    let refs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();

    let hold = state.dispatch_holds.hold(
        job,
        birdtest::jobs::HoldKind::Claims,
        std::time::Duration::from_secs(300),
    );
    let (status, body) =
        send(&app, post_json(&format!("/api/admin/jobs/{job}/purge"), &refs, json!({}))).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let (status, body) =
        send(&app, request("DELETE", &format!("/api/admin/jobs/{job}"), &headers)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    // Nor does anything else wait out the purge on the job's row -- and a
    // completion would finish a job the purge had just emptied.
    for (path, body) in [
        ("activate", json!({ "allocation": 50 })),
        ("deactivate", json!({})),
        ("complete", json!({})),
    ] {
        let (status, response) =
            send(&app, post_json(&format!("/api/admin/jobs/{job}/{path}"), &refs, body)).await;
        assert_eq!(status, StatusCode::CONFLICT, "{path}: {response}");
    }

    drop(hold);
    let (status, body) =
        send(&app, post_json(&format!("/api/admin/jobs/{job}/purge"), &refs, json!({}))).await;
    assert!(status.is_success(), "once it has finished: {body}");
}

/// A-ADMIN-20: a purge that waits for a rating fit (it marks every pool for a
/// refit under their fit locks) does not hold its contributors' rows while it
/// waits. It gave their counters back first, and every submission of theirs,
/// for any job, waited on the purge -- holding a pool connection -- for as
/// long as the fit ran.
#[tokio::test]
async fn a_purge_waiting_on_a_rating_fit_holds_up_no_submissions() {
    let db = TestDb::new().await;
    let purged = db.games_job(1, 2).await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let admin = db.user("root", true).await;
    let headers = admin_headers(&state.cfg, admin);
    let refs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();

    // One anonymous worker completes a task of the job to be purged...
    let (status, claim) = send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{claim}");
    let uuid = claim["worker_uuid"].as_str().unwrap().to_string();
    let (status, body) = send(
        &app,
        post_json(
            "/api/worker/result",
            &[("x-worker-uuid", uuid.as_str())],
            json!({ "claim_token": claim["claim_token"], "result": games_result(2, 1) }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // ...and holds a claim on another job.
    sqlx::query("UPDATE jobs SET status = 'inactive' WHERE id = $1").bind(purged).execute(&db.pool).await.unwrap();
    db.games_job(1, 2).await;
    let (status, other) = send(
        &app,
        post_json("/api/worker/task", &[("x-worker-uuid", uuid.as_str())], claim_body("1.0.0", &[])),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{other}");

    // A fit is running on a pool: its lock is held on a side connection.
    let pool_id: Uuid = sqlx::query_scalar(
        "INSERT INTO rating_pools (name, variant, letterdist_id, layout_id, anchor_player_config_id)
         SELECT 'pool', j.variant, j.letterdist_id, j.layout_id, c.player1_config_id
         FROM jobs j JOIN job_game_config c ON c.job_id = j.id WHERE j.id = $1
         RETURNING id",
    )
    .bind(purged)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let mut fit = db.pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(2, hashtext($1::text))")
        .bind(pool_id)
        .execute(&mut *fit)
        .await
        .unwrap();

    let purge = {
        let app = app.clone();
        let refs: Vec<(String, String)> = refs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        tokio::spawn(async move {
            let refs: Vec<(&str, &str)> = refs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            send(&app, post_json(&format!("/api/admin/jobs/{purged}/purge"), &refs, json!({}))).await
        })
    };
    // The purge is waiting on the fit's lock.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let waiting: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_stat_activity
                            WHERE wait_event_type = 'Lock' AND wait_event = 'advisory')",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        if waiting {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "the purge never reached the fit's lock");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    // The worker's submission for the other job goes through meanwhile.
    let started = std::time::Instant::now();
    let (status, body) = send(
        &app,
        post_json(
            "/api/worker/result",
            &[("x-worker-uuid", uuid.as_str())],
            json!({ "claim_token": other["claim_token"], "result": games_result(2, 1) }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(started.elapsed() < std::time::Duration::from_secs(3), "the submission waited on the purge");

    fit.commit().await.unwrap();
    let (status, body) = purge.await.unwrap();
    assert!(status.is_success(), "{body}");
}

/// A-ADMIN-17: a worker request's `last_seen_at` touch does not wait on its
/// identity's row. A purge gives back what a job's contributors earned, and a
/// submission bumps its contributor's counter; a touch that waited on either
/// held a pool connection for as long -- for a purge, minutes, for every
/// anonymous contributor to the job.
#[tokio::test]
async fn a_locked_identity_row_does_not_hold_up_its_requests() {
    let db = TestDb::new().await;
    db.games_job(1, 2).await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let (status, claim) =
        send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{claim}");
    let token = claim["claim_token"].as_str().unwrap().to_string();
    let uuid: Uuid = claim["worker_uuid"].as_str().unwrap().parse().unwrap();
    sqlx::query("UPDATE anonymous_workers SET last_seen_at = now() - interval '1 hour' WHERE uuid = $1")
        .bind(uuid)
        .execute(&db.pool)
        .await
        .unwrap();

    let mut holder = db.pool.begin().await.unwrap();
    sqlx::query("SELECT 1 FROM anonymous_workers WHERE uuid = $1 FOR UPDATE")
        .bind(uuid)
        .execute(&mut *holder)
        .await
        .unwrap();
    let uuid_text = uuid.to_string();
    let heartbeat = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        send(
            &app,
            post_json(
                "/api/worker/heartbeat",
                &[("x-worker-uuid", uuid_text.as_str())],
                json!({ "claim_token": token }),
            ),
        ),
    )
    .await
    .expect("the heartbeat waited on its identity's row");
    assert_eq!(heartbeat.0, StatusCode::NO_CONTENT, "{}", heartbeat.1);
    holder.rollback().await.unwrap();
}

/// A-ADMIN-18: a deleted account's tombstone address cannot be squatted. It
/// was `<id>@deleted.invalid`, and ids are public: anyone who registered that
/// address first made the account impossible to delete (the address is
/// unique). And the bans in force are listed with the id lifting one takes.
#[tokio::test]
async fn a_deleted_accounts_tombstone_cannot_be_squatted_and_bans_are_listed() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let target = db.user("target", false).await;
    sqlx::query(
        "INSERT INTO users (username, email, password_hash) VALUES ('squatter', $1, 'x')",
    )
    .bind(format!("{target}@deleted.invalid"))
    .execute(&db.pool)
    .await
    .unwrap();
    let admin = db.user("root", true).await;
    let headers = admin_headers(&state.cfg, admin);

    let (status, body) =
        send(&app, request("DELETE", &format!("/api/admin/users/{target}"), &headers)).await;
    assert!(status.is_success(), "{body}");

    let borrowed: Vec<(&str, &str)> =
        headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let (status, ban) = send(
        &app,
        post_json("/api/admin/workers/ban", &borrowed, json!({ "user_id": admin, "reason": "test" })),
    )
    .await;
    assert!(status.is_success(), "{ban}");
    let (status, bans) = send(&app, get_request("/api/admin/workers/bans", &headers)).await;
    assert_eq!(status, StatusCode::OK, "{bans}");
    assert_eq!(bans[0]["id"], ban["id"], "{bans}");
    assert_eq!(bans[0]["username"], "root", "{bans}");
}

/// A-ADMIN-20: a job cannot pin two different files under one role and name.
/// MAGPIE finds a file by its name, so two players on `NWL23.klv2` from two
/// data releases made a job every worker declined as missing data, whichever
/// release it held (thirty-first audit). The same file on both sides, and two
/// differently named files, are still a job.
#[tokio::test]
async fn a_job_cannot_pin_two_files_under_one_name() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let admin = db.user("root", true).await;
    let headers = admin_headers(&state.cfg, admin);
    let headers: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let letterdist = db.input_data("letterdist", "english").await;
    let layout = db.input_data("layout", "standard15").await;
    let kwg = db.input_data("kwg", "NWL23").await;
    let old_klv = db.input_data("klv", "NWL23").await;
    let new_klv = db.input_data("klv", "NWL23").await;
    sqlx::query("UPDATE input_data SET tarball_date = '20260101' WHERE id = $1")
        .bind(new_klv)
        .execute(&db.pool)
        .await
        .unwrap();
    let other_klv = db.input_data("klv", "CSW21").await;

    let mut players = Vec::new();
    for (name, klv) in [("old", old_klv), ("new", new_klv), ("other", other_klv), ("old-again", old_klv)] {
        let (status, created) = player_config(&app, &headers, json!({
            "name": name, "recorder_type": "best", "sort_strategy": "equity",
            "kwg_id": kwg, "klv_id": klv, "num_plays_recorded": 1,
        })).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        players.push(created["id"].clone());
    }
    let create = |p1: &serde_json::Value, p2: &serde_json::Value| {
        post_json("/api/admin/jobs", &headers, json!({
            "job_type": "games", "variant": "classic",
            "letterdist_id": letterdist, "layout_id": layout,
            "player1_config_id": p1, "player2_config_id": p2,
            "max_games": 10,
        }))
    };

    let (status, body) = send(&app, create(&players[0], &players[1])).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["message"].as_str().unwrap().contains("NWL23"), "{body}");
    let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs").fetch_one(&db.pool).await.unwrap();
    assert_eq!(jobs, 0, "nothing of the refused job is left");

    let (status, body) = send(&app, create(&players[0], &players[3])).await;
    assert_eq!(status, StatusCode::CREATED, "one file on both sides: {body}");
    let (status, body) = send(&app, create(&players[0], &players[2])).await;
    assert_eq!(status, StatusCode::CREATED, "two names, two files: {body}");
}

/// A-ADMIN-22: deleting an account locks the account before its own rows, the
/// order a password reset and an email confirmation take them in
/// (`auth_routes` pins theirs). Rows first, a delete deadlocked with either.
#[tokio::test]
async fn deleting_an_account_locks_it_before_its_rows() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let doomed = db.user("doomed", false).await;
    sqlx::query(
        "INSERT INTO password_reset_tokens (user_id, token_hash, expires_at)
         VALUES ($1, 'one', now() + interval '1 hour'), ($1, 'two', now() + interval '1 hour')",
    )
    .bind(doomed)
    .execute(&db.pool)
    .await
    .unwrap();
    let admin = db.user("root", true).await;

    let mut holder = db.pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM users WHERE id = $1 FOR UPDATE")
        .bind(doomed)
        .execute(&mut *holder)
        .await
        .unwrap();
    let delete = {
        let (app, headers) = (app.clone(), admin_headers(&state.cfg, admin));
        tokio::spawn(async move {
            send(&app, request("DELETE", &format!("/api/admin/users/{doomed}"), &headers)).await
        })
    };
    let mut waited = false;
    for _ in 0..400 {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity
             WHERE datname = current_database() AND wait_event_type = 'Lock'",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        if waiting > 0 {
            waited = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(waited, "the delete never waited for the account");
    let mut probe = db.pool.begin().await.unwrap();
    let free = sqlx::query("SELECT 1 FROM password_reset_tokens WHERE user_id = $1 FOR UPDATE NOWAIT")
        .bind(doomed)
        .fetch_all(&mut *probe)
        .await;
    assert!(free.is_ok(), "the delete took the account's reset links first: {free:?}");
    probe.rollback().await.unwrap();
    holder.rollback().await.unwrap();
    let (status, body) = delete.await.unwrap();
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
}

/// A-ADMIN-23: MAGPIE builds a wordmap, and so a rack info table, for at most
/// two blanks, and aborts on more. A job whose players use either, on a
/// distribution with three blanks, was created, its build failed three
/// times, and it never dispatched, with nothing on its page to say why. It is
/// refused; the same job without a wordmap is not.
#[tokio::test]
async fn a_wordmap_on_a_distribution_with_more_than_two_blanks_is_refused() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let admin = db.user("root", true).await;
    let headers = admin_headers(&state.cfg, admin);
    let headers: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let letterdist = db.input_data("letterdist", "english").await;
    let three_blanks: Uuid = sqlx::query_scalar(
        "INSERT INTO input_data (path, role, name, sha256, bytes, tarball_date, content)
         SELECT 'letterdistributions/english_super.csv', 'letterdist', 'english_super',
                repeat('4', 64), bytes, tarball_date,
                convert_to(replace(convert_from(content, 'UTF8'), '?,?,1,', '?,?,3,'), 'UTF8')
         FROM input_data WHERE id = $1
         RETURNING id",
    )
    .bind(letterdist)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let layout = db.input_data("layout", "standard15").await;
    let kwg = db.input_data("kwg", "NWL23").await;
    let klv = db.input_data("klv", "NWL23").await;

    let mut players = Vec::new();
    for (name, use_wordmap) in [("with-wordmap", true), ("without", false)] {
        let (status, player) = player_config(&app, &headers, json!({
            "name": name, "recorder_type": "best", "kwg_id": kwg, "klv_id": klv,
            // A table is built from a wordmap, so "without" goes without both.
            "use_wordmap": use_wordmap, "use_rit": use_wordmap, "num_plays_recorded": 1,
        })).await;
        assert_eq!(status, StatusCode::CREATED, "{player}");
        players.push(player["id"].clone());
    }
    let job = |player: &serde_json::Value| json!({
        "job_type": "games", "variant": "classic",
        "letterdist_id": three_blanks, "layout_id": layout,
        "player1_config_id": player, "player2_config_id": player,
        "max_games": 10,
    });
    let (status, body) = send(&app, post_json("/api/admin/jobs", &headers, job(&players[0]))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["fields"][0]["field"], "letterdist_id", "{body}");
    assert!(body["fields"][0]["message"].as_str().unwrap().contains("3 blanks"), "{body}");
    let jobs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs").fetch_one(&db.pool).await.unwrap();
    let queued: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM derived_data").fetch_one(&db.pool).await.unwrap();
    assert_eq!((jobs, queued), (0, 0), "nothing is created or queued");

    let (status, body) = send(&app, post_json("/api/admin/jobs", &headers, job(&players[1]))).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

/// A-ADMIN-24: Retry resets the one failed build it names. Rows can share a
/// role and a name -- under another builder, or another copy of a lexicon --
/// and it reset every failed one of them, other builders' included, which no
/// builder of this MAGPIE then takes.
#[tokio::test]
async fn a_retry_resets_only_the_build_it_names() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let admin = db.user("root", true).await;
    let owned = admin_headers(&state.cfg, admin);
    let headers: Vec<(&str, &str)> = owned.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let ld = db.input_data("letterdist", "english").await;
    let kwg = db.input_data("kwg", "NWL23").await;
    for builder in ["wmp-1", "wmp-0"] {
        sqlx::query(
            "INSERT INTO derived_data (role, name, builder, kwg_id, letterdist_id, state, attempts, error)
             VALUES ('wmp', 'NWL23', $1, $2, $3, 'failed', 3, 'gone')",
        )
        .bind(builder)
        .bind(kwg)
        .bind(ld)
        .execute(&db.pool)
        .await
        .unwrap();
    }
    let (status, list) = send(&app, get_request("/api/admin/derived-data", &owned)).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let row = list.as_array().unwrap().iter().find(|r| r["builder"] == "wmp-1").unwrap().clone();
    assert_eq!(row["kwg_id"], json!(kwg), "{row}");
    assert_eq!(row["buildable"], json!(true), "{row}");
    assert!(row["made_from"].as_str().unwrap().contains("NWL23"), "{row}");
    let other = list.as_array().unwrap().iter().find(|r| r["builder"] == "wmp-0").unwrap();
    assert_eq!(other["buildable"], json!(false), "no builder of this MAGPIE takes it: {other}");
    // Some of a row's ids but not all, or none, is a mistake, said so: by
    // role and name alone it reset both rows.
    for partial in [
        json!({ "role": "wmp", "name": "NWL23", "builder": "wmp-1", "kwg_id": kwg }),
        json!({ "role": "wmp", "name": "NWL23" }),
    ] {
        let (status, body) =
            send(&app, post_json("/api/admin/derived-data/retry", &headers, partial)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }
    let (status, body) = send(&app, post_json("/api/admin/derived-data/retry", &headers, json!({
        "role": row["role"], "name": row["name"], "builder": row["builder"],
        "kwg_id": row["kwg_id"], "klv_id": row["klv_id"], "letterdist_id": row["letterdist_id"],
    }))).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let states: Vec<(String, String)> =
        sqlx::query_as("SELECT builder, state FROM derived_data ORDER BY builder")
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(states, vec![("wmp-0".into(), "failed".into()), ("wmp-1".into(), "pending".into())]);

    // A row for a builder this version lacks is not retried: nothing would
    // ever take it.
    let old = list.as_array().unwrap().iter().find(|r| r["builder"] == "wmp-0").unwrap().clone();
    let (status, body) = send(&app, post_json("/api/admin/derived-data/retry", &headers, json!({
        "role": old["role"], "name": old["name"], "builder": old["builder"],
        "kwg_id": old["kwg_id"], "klv_id": old["klv_id"], "letterdist_id": old["letterdist_id"],
    }))).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

/// I-SCHED-3c: a job with nothing to hand out does not set a newcomer's
/// parity. A games job at its cap -- its one task in flight -- is served (it
/// issued a claim a moment ago) and its ratio stands still; a newcomer put
/// level with it took the next 500 claims in a row, the veteran none
/// (thirty-second audit, pass 19). Each claim that passes it over now lifts it
/// level with the job claimed (`scheduler::lift_passed_over`), so the newcomer,
/// joining at the lowest ratio among the jobs served, joins level with the
/// veteran.
#[tokio::test]
async fn a_newcomer_is_not_put_level_with_a_job_that_has_run_out_of_work() {
    let db = TestDb::new().await;
    let cfg = db.config();
    let admin = db.user("root", true).await;
    let veteran = db.games_job(1, 2).await;
    let capped = db.games_job(1, 2).await;
    // The capped job's whole space is one task (max 2 games).
    sqlx::query("UPDATE job_game_config SET max_games = 2, min_games = 2 WHERE job_id = $1")
        .bind(capped)
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE jobs SET allocation = 40 WHERE id = ANY($1)")
        .bind(vec![veteran, capped])
        .execute(&db.pool)
        .await
        .unwrap();
    let state = db.state().await;
    let any = split_caps("1.0.0");
    let claim_one = |state: birdtest::state::AppState| async move {
        let w = birdtest::auth::WorkerIdentity::Unregistered { uuid: Uuid::new_v4(), client_ip: std::net::IpAddr::from([127, 0, 0, 1]) };
        match birdtest::scheduler::claim(&state, &w, &split_caps("1.0.0")).await.unwrap() {
            birdtest::scheduler::ClaimOutcome::Task(t) => t.job_id,
            _ => panic!("expected a task"),
        }
    };
    // The two run level until the capped job's one task is out; after that
    // the veteran issues 200 more claims, each passing the capped job over.
    let mut capped_claims = 0;
    for _ in 0..202 {
        if claim_one(state.clone()).await == capped {
            capped_claims += 1;
        }
    }
    assert_eq!(capped_claims, 1, "the capped job has one task");
    let newcomer = db.games_job(1, 2).await;
    sqlx::query("UPDATE jobs SET status = 'inactive', allocation = NULL WHERE id = $1")
        .bind(newcomer).execute(&db.pool).await.unwrap();
    let app = birdtest::app(state.clone());
    let headers = admin_headers(&cfg, admin);
    let borrowed: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let (status, body) = send(
        &app,
        post_json(&format!("/api/admin/jobs/{newcomer}/activate"), &borrowed, json!({ "allocation": 20 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let mut by_job: std::collections::HashMap<Uuid, usize> = Default::default();
    let mut first_veteran = None;
    for i in 0..900 {
        let w = birdtest::auth::WorkerIdentity::Unregistered { uuid: Uuid::new_v4(), client_ip: std::net::IpAddr::from([127, 0, 0, 1]) };
        match birdtest::scheduler::claim(&state, &w, &any).await.unwrap() {
            birdtest::scheduler::ClaimOutcome::Task(t) => {
                *by_job.entry(t.job_id).or_default() += 1;
                if t.job_id == veteran && first_veteran.is_none() {
                    first_veteran = Some(i);
                }
            }
            _ => panic!("expected a task"),
        }
    }
    // Fair: veteran 40 : newcomer 20 over the next 900 claims = 600 : 300.
    assert!(first_veteran.is_some_and(|i| i < 12), "the veteran starved: first claim at {first_veteran:?}");
    let veteran_share = by_job.get(&veteran).copied().unwrap_or(0);
    assert!((550..=650).contains(&veteran_share), "veteran {veteran_share} of 900, not about 600");
}

/// I-SCHED-3d: the same veteran and unservable job as
/// `a_job_nobody_is_being_served_from_does_not_set_a_newcomers_parity`, but
/// the fleet has been quiet for ten minutes (a deployment gap, a quiet night)
/// when the newcomer is activated: it still joins level with the veteran, not
/// the job nobody can run -- it took twelve claims of twelve (pass 19).
#[tokio::test]
async fn after_a_quiet_spell_a_newcomer_still_joins_level_with_the_jobs_served() {
    let db = TestDb::new().await;
    let cfg = db.config();
    let admin = db.user("root", true).await;
    let veteran = db.games_job(1, 2).await;
    sqlx::query(
        "UPDATE jobs SET claims_issued = 100000, allocation = 40,
                         last_claimed_at = now() - interval '10 minutes'
         WHERE id = $1",
    )
    .bind(veteran).execute(&db.pool).await.unwrap();
    let lagging = db.games_job(1, 2).await;
    sqlx::query("UPDATE jobs SET allocation = 20, min_magpie_major = 9 WHERE id = $1")
        .bind(lagging).execute(&db.pool).await.unwrap();
    let newcomer = db.games_job(1, 2).await;
    sqlx::query("UPDATE jobs SET status = 'inactive', allocation = NULL WHERE id = $1")
        .bind(newcomer).execute(&db.pool).await.unwrap();
    let app = birdtest::app(db.state().await);
    let headers = admin_headers(&cfg, admin);
    let borrowed: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let (status, body) = send(
        &app,
        post_json(&format!("/api/admin/jobs/{newcomer}/activate"), &borrowed, json!({ "allocation": 40 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let shares = claims_by_job(&app, 12).await;
    assert_eq!(shares.get(&veteran.to_string()), Some(&6), "{shares:?}");
}

fn split_caps(v: &str) -> birdtest::scheduler::WorkerCapabilities {
    birdtest::scheduler::WorkerCapabilities {
        magpie_version: birdtest::version::Version::parse_or_zero(v),
        unsupported_jobs: vec![],
    }
}

/// Who gets each of `n` claims, claimed one after another by fresh anonymous
/// workers: those for which `old(i)` holds run MAGPIE 1.0.0, the rest 3.0.0.
async fn split_run(
    state: &birdtest::state::AppState,
    n: usize,
    old: impl Fn(usize) -> bool,
) -> std::collections::HashMap<Uuid, usize> {
    let mut by_job: std::collections::HashMap<Uuid, usize> = Default::default();
    let (v1, v3) = (split_caps("1.0.0"), split_caps("3.0.0"));
    for i in 0..n {
        let c = if old(i) { &v1 } else { &v3 };
        let w = birdtest::auth::WorkerIdentity::Unregistered { uuid: Uuid::new_v4(), client_ip: std::net::IpAddr::from([127, 0, 0, 1]) };
        match birdtest::scheduler::claim(state, &w, c).await.unwrap() {
            birdtest::scheduler::ClaimOutcome::Task(t) => *by_job.entry(t.job_id).or_default() += 1,
            _ => panic!("expected a task at {i}"),
        }
    }
    by_job
}

async fn split_activate(db: &TestDb, state: &birdtest::state::AppState, admin: Uuid, job: Uuid, alloc: i32) {
    let cfg = db.config();
    let app = birdtest::app(state.clone());
    let headers = admin_headers(&cfg, admin);
    let borrowed: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let (status, body) = send(
        &app,
        post_json(&format!("/api/admin/jobs/{job}/activate"), &borrowed, json!({ "allocation": alloc })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// I-SCHED-3e: in a split fleet a newcomer still gets its share. A job some
/// workers can only run (an old MAGPIE floor while a release rolls out)
/// climbs past its share and leads; the job the rest run lags it without
/// limit. Joined at the leader, a newcomer got none of the next 1,000 claims
/// (the audit's pass 19); joined at the lowest ratio among the jobs served,
/// it shares.
#[tokio::test]
async fn in_a_split_fleet_a_newcomer_is_not_starved_behind_a_lagging_job() {
    let db = TestDb::new().await;
    let admin = db.user("root", true).await;
    let t = db.games_job(1, 2).await; // runnable by everyone (floor 0.1.0)
    let a = db.games_job(1, 2).await; // needs MAGPIE 2
    sqlx::query("UPDATE jobs SET allocation = 10 WHERE id = $1").bind(t).execute(&db.pool).await.unwrap();
    sqlx::query("UPDATE jobs SET allocation = 50, min_magpie_major = 2 WHERE id = $1").bind(a).execute(&db.pool).await.unwrap();
    let state = db.state().await;
    // 30% of claims from workers still on MAGPIE 1.
    split_run(&state, 1000, |i| i % 10 < 3).await;
    let n = db.games_job(1, 2).await;
    sqlx::query("UPDATE jobs SET status = 'inactive', allocation = NULL, min_magpie_major = 2 WHERE id = $1")
        .bind(n).execute(&db.pool).await.unwrap();
    split_activate(&db, &state, admin, n, 40).await;
    let p2 = split_run(&state, 1000, |i| i % 10 < 3).await;
    // Fair among the 700 claims from MAGPIE-2 workers is A:N = 50:40, N about
    // 311.
    let got = p2.get(&n).copied().unwrap_or(0);
    assert!((200..=350).contains(&got), "newcomer at 40% got {got} of the next 1000 claims");
}

/// I-SCHED-3f: a job only a minority can run lags while served; a newcomer the
/// same minority can run is not starved behind it (joined at the leader, it
/// got none of 1,000 claims).
#[tokio::test]
async fn a_minority_newcomer_is_not_starved_behind_a_lagging_minority_job() {
    let db = TestDb::new().await;
    let admin = db.user("root", true).await;
    let a = db.games_job(1, 2).await; // everyone
    let b = db.games_job(1, 2).await; // MAGPIE 2 only
    sqlx::query("UPDATE jobs SET allocation = 40 WHERE id = $1").bind(a).execute(&db.pool).await.unwrap();
    sqlx::query("UPDATE jobs SET allocation = 30, min_magpie_major = 2 WHERE id = $1").bind(b).execute(&db.pool).await.unwrap();
    let state = db.state().await;
    // 80% of claims from MAGPIE-1 workers.
    split_run(&state, 1000, |i| i % 10 < 8).await;
    let n = db.games_job(1, 2).await;
    sqlx::query("UPDATE jobs SET status = 'inactive', allocation = NULL, min_magpie_major = 2 WHERE id = $1")
        .bind(n).execute(&db.pool).await.unwrap();
    split_activate(&db, &state, admin, n, 30).await;
    let p2 = split_run(&state, 1000, |i| i % 10 < 8).await;
    // Fair among the 200 MAGPIE-2 claims is B:N = 30:30, N about 100.
    let got = p2.get(&n).copied().unwrap_or(0);
    assert!((50..=120).contains(&got), "newcomer at 30% got {got} of the next 1000 claims");
}

/// I-SCHED-3g: a job nobody could run does not bank the claims it missed. Its
/// ratio stands still while the others climb; once the fleet can run it --
/// its data out, its MAGPIE floor reached -- it took every claim of the
/// workers that could run it until it had caught up (with each lag bounded,
/// all of the next 40; unbounded, as many as the fleet had issued meanwhile). It
/// rejoins at parity on its first claim back (`scheduler::issue_claim`).
#[tokio::test]
async fn a_job_nobody_could_run_rejoins_at_parity_when_the_fleet_can() {
    let db = TestDb::new().await;
    let veteran = db.games_job(1, 2).await;
    let waiting = db.games_job(1, 2).await;
    sqlx::query("UPDATE jobs SET allocation = 40 WHERE id = $1").bind(veteran).execute(&db.pool).await.unwrap();
    // Activated an hour ago; nobody has had MAGPIE 9 since.
    sqlx::query("UPDATE jobs SET allocation = 40, min_magpie_major = 9, activated_at = now() - interval '1 hour' WHERE id = $1")
        .bind(waiting).execute(&db.pool).await.unwrap();
    let state = db.state().await;
    let all = split_run(&state, 400, |_| true).await;
    assert_eq!(all.get(&veteran), Some(&400));
    // The release is out.
    sqlx::query("UPDATE jobs SET min_magpie_major = 0 WHERE id = $1").bind(waiting).execute(&db.pool).await.unwrap();
    let next = split_run(&state, 40, |_| true).await;
    let got = next.get(&veteran).copied().unwrap_or(0);
    assert!((19..=21).contains(&got), "veteran got {got} of the 40 claims after the other job became runnable, not about 20");
}

/// I-SCHED-3h: jobs that lag together keep their order. Beside a job at 10%
/// that half the fleet can only run (and so leads), two jobs at 45% the other
/// half runs split that half evenly. With every job's lag bounded against the
/// job just claimed, each claim of the leader set both to the same floor and
/// the older one took every tie: 978 : 22 (the audit's pass 20).
#[tokio::test]
async fn jobs_lagging_together_keep_their_shares() {
    let db = TestDb::new().await;
    let t = db.games_job(1, 2).await;
    let b = db.games_job(1, 2).await;
    let c = db.games_job(1, 2).await;
    sqlx::query("UPDATE jobs SET allocation = 10 WHERE id = $1").bind(t).execute(&db.pool).await.unwrap();
    sqlx::query("UPDATE jobs SET allocation = 45, min_magpie_major = 2, created_at = now() - interval '1 hour' WHERE id = $1")
        .bind(b).execute(&db.pool).await.unwrap();
    sqlx::query("UPDATE jobs SET allocation = 45, min_magpie_major = 2 WHERE id = $1").bind(c).execute(&db.pool).await.unwrap();
    let state = db.state().await;
    let by = split_run(&state, 2000, |i| i % 2 == 0).await;
    let (gb, gc) = (by.get(&b).copied().unwrap_or(0), by.get(&c).copied().unwrap_or(0));
    assert_eq!(gb + gc, 1000);
    assert!((490..=510).contains(&gb), "B {gb} : C {gc} of the MAGPIE-2 claims, not 500 : 500");
}

/// I-SCHED-3i: a burst a small job gets from claims made at the same moment is
/// paid back. Thirty-two workers claiming at once all see the 1% job lowest and
/// take it; its lead is then worked off as the other job catches up. With
/// every job's lag bounded, the other job's catching up was forgiven: 64 to 87
/// claims of 3,000 where 30 is fair (the audit's pass 20).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_concurrent_burst_to_a_small_job_is_paid_back() {
    let db = TestDb::new().await;
    let a = db.games_job(1, 2).await;
    let b = db.games_job(1, 2).await;
    sqlx::query("UPDATE jobs SET allocation = 1 WHERE id = $1").bind(a).execute(&db.pool).await.unwrap();
    sqlx::query("UPDATE jobs SET allocation = 99 WHERE id = $1").bind(b).execute(&db.pool).await.unwrap();
    let state = db.state().await;
    let mut workers = Vec::new();
    for _ in 0..32 {
        let state = state.clone();
        workers.push(tokio::spawn(async move {
            let mut got = std::collections::HashMap::<Uuid, usize>::new();
            for _ in 0..(3000 / 32) {
                let w = birdtest::auth::WorkerIdentity::Unregistered { uuid: Uuid::new_v4(), client_ip: std::net::IpAddr::from([127, 0, 0, 1]) };
                if let birdtest::scheduler::ClaimOutcome::Task(t) = birdtest::scheduler::claim(&state, &w, &split_caps("3.0.0")).await.unwrap() {
                    *got.entry(t.job_id).or_default() += 1;
                }
            }
            got
        }));
    }
    let mut total = std::collections::HashMap::<Uuid, usize>::new();
    for w in workers {
        for (k, v) in w.await.unwrap() {
            *total.entry(k).or_default() += v;
        }
    }
    let (ga, gb) = (total.get(&a).copied().unwrap_or(0), total.get(&b).copied().unwrap_or(0));
    // Fair is 30 of 2,976; forgiven, it was 64 to 87.
    assert!(ga <= 45, "the 1% job got {ga} of {} claims", ga + gb);
}

/// The MAGPIE-1 workers' claims (every tenth claim but two) as a sequence.
async fn majority_claims(state: &birdtest::state::AppState, n: usize) -> Vec<Uuid> {
    let (v1, v3) = (split_caps("1.0.0"), split_caps("3.0.0"));
    let mut majority = Vec::new();
    for i in 0..n {
        let old = i % 10 >= 2;
        let w = birdtest::auth::WorkerIdentity::Unregistered { uuid: Uuid::new_v4(), client_ip: std::net::IpAddr::from([127, 0, 0, 1]) };
        match birdtest::scheduler::claim(state, &w, if old { &v1 } else { &v3 }).await.unwrap() {
            birdtest::scheduler::ClaimOutcome::Task(t) if old => majority.push(t.job_id),
            birdtest::scheduler::ClaimOutcome::Task(_) => {}
            _ => panic!("expected a task at {i}"),
        }
    }
    majority
}

/// A at 50% that everyone runs, L at 40% that only the 20% of claims from
/// MAGPIE 2 can run -- L lags, served, for as long as the split lasts.
async fn minority_split(db: &TestDb) -> (birdtest::state::AppState, Uuid, Uuid) {
    let a = db.games_job(1, 2).await;
    let l = db.games_job(1, 2).await;
    sqlx::query("UPDATE jobs SET allocation = 50 WHERE id = $1").bind(a).execute(&db.pool).await.unwrap();
    sqlx::query("UPDATE jobs SET allocation = 40, min_magpie_major = 2 WHERE id = $1").bind(l).execute(&db.pool).await.unwrap();
    let state = db.state().await;
    majority_claims(&state, 3000).await;
    (state, a, l)
}

/// I-SCHED-3j: a newcomer everyone can run is not put level with a job only a
/// minority can run. Joined at the lowest ratio served -- the minority job's,
/// which lags for as long as the split lasts -- it took the majority's claims
/// until it had caught the majority's job: A's first at the 331st, 391 of 800
/// where 667 is fair, and worse the longer the split had lasted (the audit's
/// pass 20). Each claim within an hour of joining settles it level with the
/// lowest of the claiming worker's other candidates.
#[tokio::test]
async fn a_newcomer_everyone_can_run_is_not_put_level_with_a_minority_job() {
    let db = TestDb::new().await;
    let admin = db.user("root", true).await;
    let (state, a, _) = minority_split(&db).await;
    let n = db.games_job(1, 2).await;
    sqlx::query("UPDATE jobs SET status = 'inactive', allocation = NULL WHERE id = $1").bind(n).execute(&db.pool).await.unwrap();
    split_activate(&db, &state, admin, n, 10).await;
    let majority = majority_claims(&state, 1000).await;
    let got = majority.iter().filter(|j| **j == a).count();
    let first = majority.iter().position(|j| *j == a);
    // Fair among the 800: A 50 : N 10.
    assert!(first.is_some_and(|i| i < 12), "A's first claim at {first:?}");
    assert!((640..=690).contains(&got), "A got {got} of the majority's 800, not about 667");
}

/// I-SCHED-3k: an allocation changed in a split fleet does not hand the
/// changed job the majority. Activation is a join, so B -- 20% to 19% --
/// rejoined level with the minority's job and took the majority's claims: A's
/// first at the 476th, 220 of 800 where 542 is fair (the audit's pass 20).
#[tokio::test]
async fn an_allocation_changed_in_a_split_fleet_takes_nothing_over() {
    let db = TestDb::new().await;
    let admin = db.user("root", true).await;
    let a = db.games_job(1, 2).await;
    let b = db.games_job(1, 2).await;
    let l = db.games_job(1, 2).await;
    sqlx::query("UPDATE jobs SET allocation = 40 WHERE id = $1").bind(a).execute(&db.pool).await.unwrap();
    sqlx::query("UPDATE jobs SET allocation = 20 WHERE id = $1").bind(b).execute(&db.pool).await.unwrap();
    sqlx::query("UPDATE jobs SET allocation = 40, min_magpie_major = 2 WHERE id = $1").bind(l).execute(&db.pool).await.unwrap();
    let state = db.state().await;
    majority_claims(&state, 3000).await;
    split_activate(&db, &state, admin, b, 19).await;
    let majority = majority_claims(&state, 1000).await;
    let got = majority.iter().filter(|j| **j == a).count();
    let first = majority.iter().position(|j| *j == a);
    // Fair among the 800: A 40 : B 19.
    assert!(first.is_some_and(|i| i < 12), "A's first claim at {first:?}");
    assert!((515..=570).contains(&got), "A got {got} of the majority's 800, not about 542");
}

/// I-SCHED-3l: a job nobody could run, returning in a split fleet, is settled
/// as a newcomer is. It rejoined at the lowest ratio served, the minority
/// job's, and took the majority's claims: A's first at the 331st (the audit's
/// pass 20).
#[tokio::test]
async fn a_returning_job_in_a_split_fleet_takes_nothing_over() {
    let db = TestDb::new().await;
    let x = db.games_job(1, 2).await;
    sqlx::query("UPDATE jobs SET allocation = 10, min_magpie_major = 9, activated_at = now() - interval '1 hour' WHERE id = $1")
        .bind(x).execute(&db.pool).await.unwrap();
    let (state, a, _) = minority_split(&db).await;
    sqlx::query("UPDATE jobs SET min_magpie_major = 0 WHERE id = $1").bind(x).execute(&db.pool).await.unwrap();
    let majority = majority_claims(&state, 1000).await;
    let got = majority.iter().filter(|j| **j == a).count();
    let first = majority.iter().position(|j| *j == a);
    assert!(first.is_some_and(|i| i < 12), "A's first claim at {first:?}");
    assert!((640..=690).contains(&got), "A got {got} of the majority's 800, not about 667");
}

/// I-SCHED-3m: a job passed over for a moment is lifted to where the job
/// claimed stood before its claim, not after. Lifted past it, a job at 50%
/// passed over while a 1% job was claimed was a whole ratio unit ahead of the
/// rest and waited 49 claims when its work came back (the audit's pass 20).
#[tokio::test]
async fn a_job_passed_over_for_a_moment_waits_for_nothing() {
    let db = TestDb::new().await;
    let p = db.games_job(1, 2).await;
    let c = db.games_job(1, 2).await;
    let d = db.games_job(1, 2).await;
    for (job, alloc, age) in [(p, 50, 3), (c, 1, 2), (d, 49, 1)] {
        sqlx::query("UPDATE jobs SET allocation = $2, created_at = now() - make_interval(hours => $3) WHERE id = $1")
            .bind(job).bind(alloc).bind(age).execute(&db.pool).await.unwrap();
    }
    let state = db.state().await;
    split_run(&state, 200, |_| false).await;
    // P has nothing to hand out (a dispatch hold) until the 1% job is claimed.
    let hold = state.dispatch_holds.hold(p, birdtest::jobs::HoldKind::DispatchOnly, std::time::Duration::ZERO);
    let mut gap = 0;
    loop {
        gap += 1;
        assert!(gap < 500, "the 1% job was never claimed");
        if split_run(&state, 1, |_| false).await.contains_key(&c) {
            break;
        }
    }
    drop(hold);
    let v3 = split_caps("3.0.0");
    let mut first = None;
    let mut got = 0;
    for i in 0..300 {
        let w = birdtest::auth::WorkerIdentity::Unregistered { uuid: Uuid::new_v4(), client_ip: std::net::IpAddr::from([127, 0, 0, 1]) };
        if let birdtest::scheduler::ClaimOutcome::Task(t) = birdtest::scheduler::claim(&state, &w, &v3).await.unwrap() {
            if t.job_id == p {
                got += 1;
                first.get_or_insert(i);
            }
        }
    }
    assert!(first.is_some_and(|i| i < 3), "P's first claim after its gap at {first:?}");
    assert!((140..=160).contains(&got), "P got {got} of 300, not about 150");
}

/// I-SCHED-3n: a job with nothing to hand out banks no debt. Held (a seeding,
/// a generation being built) for 200 claims beside a job at the same share,
/// it came back that many claims behind and took the next hundred in a row;
/// passed over, it is lifted level with the job claimed instead
/// (`scheduler::lift_passed_over`).
#[tokio::test]
async fn a_job_with_nothing_to_hand_out_banks_no_debt() {
    let db = TestDb::new().await;
    let p = db.games_job(1, 2).await;
    let d = db.games_job(1, 2).await;
    sqlx::query("UPDATE jobs SET allocation = 50 WHERE id = ANY($1)").bind(vec![p, d]).execute(&db.pool).await.unwrap();
    let state = db.state().await;
    split_run(&state, 100, |_| false).await;
    let hold = state.dispatch_holds.hold(p, birdtest::jobs::HoldKind::DispatchOnly, std::time::Duration::ZERO);
    assert_eq!(split_run(&state, 200, |_| false).await.get(&d), Some(&200));
    drop(hold);
    let v3 = split_caps("3.0.0");
    let mut first_d = None;
    for i in 0..20 {
        let w = birdtest::auth::WorkerIdentity::Unregistered { uuid: Uuid::new_v4(), client_ip: std::net::IpAddr::from([127, 0, 0, 1]) };
        if let birdtest::scheduler::ClaimOutcome::Task(t) = birdtest::scheduler::claim(&state, &w, &v3).await.unwrap() {
            if t.job_id == d {
                first_d.get_or_insert(i);
            }
        }
    }
    assert!(first_d.is_some_and(|i| i < 3), "the other job's first claim after the hold at {first_d:?}");
}

/// Who gets each claim from the minority (MAGPIE 3, every tenth claim but
/// eight) of I-SCHED-3f's fleet.
async fn minority_claims(state: &birdtest::state::AppState, n: usize) -> std::collections::HashMap<Uuid, usize> {
    let (v1, v3) = (split_caps("1.0.0"), split_caps("3.0.0"));
    let mut got = std::collections::HashMap::new();
    for i in 0..n {
        let major = i % 10 < 8;
        let w = birdtest::auth::WorkerIdentity::Unregistered { uuid: Uuid::new_v4(), client_ip: std::net::IpAddr::from([127, 0, 0, 1]) };
        match birdtest::scheduler::claim(state, &w, if major { &v1 } else { &v3 }).await.unwrap() {
            birdtest::scheduler::ClaimOutcome::Task(t) if !major => *got.entry(t.job_id).or_default() += 1,
            birdtest::scheduler::ClaimOutcome::Task(_) => {}
            _ => panic!("expected a task at {i}"),
        }
    }
    got
}

/// I-SCHED-3o: a burst is paid back in a job's first hour too. The jobs of
/// I-SCHED-3i activated as an admin activates them, so both are settling: a
/// burst to the 1% job put the 99% job more than a claim behind it, and
/// settling forgave the difference -- 307 to 324 claims where 30 is fair (the
/// audit's pass 20). A claim that finds its job moved past a rival on its way
/// now goes to the rival instead, so there is no burst to forgive.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_burst_in_the_first_hour_is_paid_back() {
    let db = TestDb::new().await;
    let admin = db.user("root", true).await;
    let a = db.games_job(1, 2).await;
    let b = db.games_job(1, 2).await;
    sqlx::query("UPDATE jobs SET status = 'inactive', allocation = NULL WHERE id = ANY($1)")
        .bind(vec![a, b]).execute(&db.pool).await.unwrap();
    let state = db.state().await;
    split_activate(&db, &state, admin, a, 1).await;
    split_activate(&db, &state, admin, b, 99).await;
    let mut workers = Vec::new();
    for _ in 0..32 {
        let state = state.clone();
        workers.push(tokio::spawn(async move {
            let mut got = std::collections::HashMap::<Uuid, usize>::new();
            for _ in 0..(3000 / 32) {
                let w = birdtest::auth::WorkerIdentity::Unregistered { uuid: Uuid::new_v4(), client_ip: std::net::IpAddr::from([127, 0, 0, 1]) };
                if let birdtest::scheduler::ClaimOutcome::Task(t) = birdtest::scheduler::claim(&state, &w, &split_caps("3.0.0")).await.unwrap() {
                    *got.entry(t.job_id).or_default() += 1;
                }
            }
            got
        }));
    }
    let mut total = std::collections::HashMap::<Uuid, usize>::new();
    for w in workers {
        for (k, v) in w.await.unwrap() {
            *total.entry(k).or_default() += v;
        }
    }
    let (ga, gb) = (total.get(&a).copied().unwrap_or(0), total.get(&b).copied().unwrap_or(0));
    assert!(ga <= 40, "the 1% job got {ga} of {} claims", ga + gb);
}

/// I-SCHED-3p: a newcomer only the minority has the data for is not settled at
/// the majority's pace. The server cannot filter a data gap: each majority
/// worker is issued one claim of it and declines it `missing_data`, and that
/// claim settled it level with the majority's job, past the minority's own
/// lagging job -- and the minority never reached it: none of 400 claims (the
/// audit's pass 20). A decline that says the worker cannot run the job undoes
/// the settling its claim gave (`scheduler::unsettle`).
#[tokio::test]
async fn a_newcomer_the_majority_declines_is_not_settled_at_its_pace() {
    let db = TestDb::new().await;
    let admin = db.user("root", true).await;
    let a = db.games_job(1, 2).await;
    let l = db.games_job(1, 2).await;
    sqlx::query("UPDATE jobs SET allocation = 50 WHERE id = $1").bind(a).execute(&db.pool).await.unwrap();
    sqlx::query("UPDATE jobs SET allocation = 40, min_magpie_major = 2 WHERE id = $1").bind(l).execute(&db.pool).await.unwrap();
    let n = db.games_job(1, 2).await;
    sqlx::query("UPDATE jobs SET status = 'inactive', allocation = NULL WHERE id = $1").bind(n).execute(&db.pool).await.unwrap();
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    // Sixteen majority workers (MAGPIE 1) without N's data, four minority
    // workers (MAGPIE 3) with it; 80 : 20.
    let majority: Vec<Uuid> = (0..16).map(|_| Uuid::new_v4()).collect();
    let minority: Vec<Uuid> = (0..4).map(|_| Uuid::new_v4()).collect();
    let mut unsupported: std::collections::HashMap<Uuid, Vec<Uuid>> = Default::default();
    let mut minority_got: std::collections::HashMap<Uuid, usize> = Default::default();
    let mut declines = 0;
    for round in 0..2 {
        if round == 1 {
            split_activate(&db, &state, admin, n, 10).await;
            minority_got.clear();
        }
        for i in 0..[3000, 2000][round] {
            let slot = i % 10;
            let (w, major) = if slot < 8 { (majority[(i / 10 * 8 + slot) % 16], true) } else { (minority[(i / 10 * 2 + slot - 8) % 4], false) };
            let caps = birdtest::scheduler::WorkerCapabilities {
                magpie_version: birdtest::version::Version::parse_or_zero(if major { "1.0.0" } else { "3.0.0" }),
                unsupported_jobs: unsupported.get(&w).cloned().unwrap_or_default(),
            };
            let id = birdtest::auth::WorkerIdentity::Unregistered { uuid: w, client_ip: std::net::IpAddr::from([127, 0, 0, 1]) };
            let birdtest::scheduler::ClaimOutcome::Task(t) = birdtest::scheduler::claim(&state, &id, &caps).await.unwrap() else {
                panic!("expected a task at {i}");
            };
            if major && t.job_id == n {
                let (status, body) = send(
                    &app,
                    post_json(
                        "/api/worker/decline",
                        &[("x-worker-uuid", w.to_string().as_str())],
                        json!({ "claim_token": t.claim_token, "reason": "missing_data" }),
                    ),
                )
                .await;
                assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
                unsupported.entry(w).or_default().push(n);
                declines += 1;
            } else if !major {
                *minority_got.entry(t.job_id).or_default() += 1;
            }
        }
    }
    assert!(declines > 0, "no majority worker was issued the newcomer");
    // Fair among the minority's 400: L 40 : N 10, N about 80.
    let got = minority_got.get(&n).copied().unwrap_or(0);
    assert!((60..=100).contains(&got), "the newcomer got {got} of the minority's 400 claims");
}

/// I-SCHED-3q: a job settles against the lowest of the worker's other
/// candidates, the ones it just passed over included. With the minority's own
/// lagging job paused for one claim -- a seeding, a lock held -- the newcomer
/// beside it was settled against the next job in the list instead, the
/// majority's, and the minority never reached it again: none of 200 claims
/// (the audit's pass 20).
#[tokio::test]
async fn a_newcomer_is_not_settled_past_a_job_paused_for_a_moment() {
    let db = TestDb::new().await;
    let admin = db.user("root", true).await;
    let a = db.games_job(1, 2).await;
    let l = db.games_job(1, 2).await;
    sqlx::query("UPDATE jobs SET allocation = 40 WHERE id = $1").bind(a).execute(&db.pool).await.unwrap();
    sqlx::query("UPDATE jobs SET allocation = 30, min_magpie_major = 2 WHERE id = $1").bind(l).execute(&db.pool).await.unwrap();
    let state = db.state().await;
    minority_claims(&state, 1000).await;
    let n = db.games_job(1, 2).await;
    sqlx::query("UPDATE jobs SET status = 'inactive', allocation = NULL, min_magpie_major = 2 WHERE id = $1")
        .bind(n).execute(&db.pool).await.unwrap();
    split_activate(&db, &state, admin, n, 30).await;
    minority_claims(&state, 50).await;
    let hold = state.dispatch_holds.hold(l, birdtest::jobs::HoldKind::DispatchOnly, std::time::Duration::ZERO);
    let w = birdtest::auth::WorkerIdentity::Unregistered { uuid: Uuid::new_v4(), client_ip: std::net::IpAddr::from([127, 0, 0, 1]) };
    birdtest::scheduler::claim(&state, &w, &split_caps("3.0.0")).await.unwrap();
    drop(hold);
    // Fair among the minority's 200: L 30 : N 30.
    let got = minority_claims(&state, 1000).await.get(&n).copied().unwrap_or(0);
    assert!((80..=120).contains(&got), "the newcomer got {got} of the minority's 200 claims");
}

/// Thirty-two workers claiming at once, `per_worker` claims each, over
/// `state`'s jobs: who got what, and how many claims were answered without a
/// task.
async fn concurrent_claims(state: &birdtest::state::AppState, per_worker: usize) -> (std::collections::HashMap<Uuid, usize>, usize) {
    let mut workers = Vec::new();
    for _ in 0..32 {
        let state = state.clone();
        workers.push(tokio::spawn(async move {
            let mut got = std::collections::HashMap::<Uuid, usize>::new();
            let mut idle = 0;
            for _ in 0..per_worker {
                let w = birdtest::auth::WorkerIdentity::Unregistered { uuid: Uuid::new_v4(), client_ip: std::net::IpAddr::from([127, 0, 0, 1]) };
                match birdtest::scheduler::claim(&state, &w, &split_caps("3.0.0")).await.unwrap() {
                    birdtest::scheduler::ClaimOutcome::Task(t) => *got.entry(t.job_id).or_default() += 1,
                    _ => idle += 1,
                }
            }
            (got, idle)
        }));
    }
    let (mut total, mut idle) = (std::collections::HashMap::new(), 0);
    for w in workers {
        let (got, i) = w.await.unwrap();
        idle += i;
        for (k, v) in got {
            *total.entry(k).or_default() += v;
        }
    }
    (total, idle)
}

/// I-SCHED-3r: a claim is not told there is nothing while work exists. With
/// jobs at equal shares and 32 workers claiming together, each job kept being
/// found a claim past the other, and a claim that found both so for eight
/// rounds answered `204` -- 67 to 166 of 1,920 (the audit's pass 21). The last
/// round takes the first job with work without the turn check.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn equal_jobs_claimed_together_leave_no_worker_idle() {
    let db = TestDb::new().await;
    for _ in 0..3 {
        db.games_job(1, 2).await;
    }
    sqlx::query("UPDATE jobs SET allocation = 33").execute(&db.pool).await.unwrap();
    let state = db.state().await;
    let (got, idle) = concurrent_claims(&state, 60).await;
    assert_eq!(idle, 0, "{idle} of 1,920 claims got no task; shares {got:?}");
    assert!(got.values().all(|n| (600..=680).contains(n)), "shares {got:?}, not about 640 each");
}

/// I-SCHED-3s: a job whose dispatch lock is held past the bounded wait costs a
/// claim that wait once. Waited on again every round, and outrunning every
/// other job with its ratio frozen, it held each claim 16 s and ended it
/// `204` while the other job had work (the audit's pass 21). A busy job is
/// not tried again in the request (it stays a rival, with a ratio unit of
/// slack: I-SCHED-3t).
#[tokio::test]
async fn a_busy_job_costs_a_claim_one_wait() {
    let db = TestDb::new().await;
    let a = db.games_job(1, 2).await;
    let b = db.games_job(1, 2).await;
    let state = db.state().await;
    split_run(&state, 20, |_| false).await;
    // Another holder keeps A's dispatch lock.
    let mut holder = db.pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(1, hashtext($1::text))")
        .bind(a)
        .execute(&mut *holder)
        .await
        .unwrap();
    for i in 0..3 {
        let started = std::time::Instant::now();
        let w = birdtest::auth::WorkerIdentity::Unregistered { uuid: Uuid::new_v4(), client_ip: std::net::IpAddr::from([127, 0, 0, 1]) };
        match birdtest::scheduler::claim(&state, &w, &split_caps("3.0.0")).await.unwrap() {
            birdtest::scheduler::ClaimOutcome::Task(t) => assert_eq!(t.job_id, b, "claim {i}"),
            _ => panic!("claim {i} got no task while B had work"),
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "claim {i} took {:?}", started.elapsed());
    }
    holder.rollback().await.unwrap();
}

/// I-SCHED-3t: a small job beside a busy large one is not handed the large
/// job's claims. Left out as a rival while its lock was held, the 99% job let
/// the 1% job take every claim of the spell -- each worth 99 of its own -- and
/// its settling forgave them: 49 claims of 3,000 where 30 is fair, after a
/// spell of twenty (the audit's pass 21). A busy job stays a rival, with a ratio unit of slack.
#[tokio::test]
async fn a_busy_large_job_does_not_hand_a_small_one_its_claims() {
    let db = TestDb::new().await;
    let admin = db.user("root", true).await;
    let a = db.games_job(1, 2).await;
    let b = db.games_job(1, 2).await;
    sqlx::query("UPDATE jobs SET status = 'inactive', allocation = NULL WHERE id = ANY($1)")
        .bind(vec![a, b]).execute(&db.pool).await.unwrap();
    let state = db.state().await;
    // Activated as an admin does, so both are settling.
    split_activate(&db, &state, admin, a, 1).await;
    split_activate(&db, &state, admin, b, 99).await;
    let before = split_run(&state, 500, |_| false).await;
    // Another holder keeps B's dispatch lock for five claims.
    let mut holder = db.pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(1, hashtext($1::text))")
        .bind(b).execute(&mut *holder).await.unwrap();
    let mut during = 0;
    for _ in 0..5 {
        let w = birdtest::auth::WorkerIdentity::Unregistered { uuid: Uuid::new_v4(), client_ip: std::net::IpAddr::from([127, 0, 0, 1]) };
        if let birdtest::scheduler::ClaimOutcome::Task(t) = birdtest::scheduler::claim(&state, &w, &split_caps("3.0.0")).await.unwrap() {
            assert_eq!(t.job_id, a, "B's lock is held");
            during += 1;
        }
    }
    holder.rollback().await.unwrap();
    let after = split_run(&state, 2500, |_| false).await;
    let got = before.get(&a).copied().unwrap_or(0) + during + after.get(&a).copied().unwrap_or(0);
    assert!(during <= 2, "the 1% job took {during} of the 5 claims while B was busy");
    assert!(got <= 33, "the 1% job got {got} claims where 30 is fair");
}

/// I-SCHED-3u: repeated busy spells on a settling job are paid back. Each spell
/// let the job beside it run a ratio unit ahead, and the busy job's settling
/// forgave the lead once the spell ended, so every spell added another: a 10%
/// job beside a 90% one took 400 claims of 2,400 over twenty spells of ten,
/// where 240 is fair (the audit's pass 22). A job found busy is settled a
/// ratio unit short for ten minutes.
#[tokio::test]
async fn repeated_busy_spells_on_a_settling_job_are_paid_back() {
    let db = TestDb::new().await;
    let admin = db.user("root", true).await;
    let small = db.games_job(1, 2).await;
    let large = db.games_job(1, 2).await;
    sqlx::query("UPDATE jobs SET status = 'inactive', allocation = NULL WHERE id = ANY($1)")
        .bind(vec![small, large]).execute(&db.pool).await.unwrap();
    let state = db.state().await;
    split_activate(&db, &state, admin, small, 10).await;
    split_activate(&db, &state, admin, large, 90).await;
    let mut got = split_run(&state, 200, |_| false).await.get(&small).copied().unwrap_or(0);
    let mut total = 200;
    for _ in 0..3 {
        let mut holder = db.pool.begin().await.unwrap();
        sqlx::query("SELECT pg_advisory_xact_lock(1, hashtext($1::text))")
            .bind(large).execute(&mut *holder).await.unwrap();
        for _ in 0..3 {
            let w = birdtest::auth::WorkerIdentity::Unregistered { uuid: Uuid::new_v4(), client_ip: std::net::IpAddr::from([127, 0, 0, 1]) };
            if let birdtest::scheduler::ClaimOutcome::Task(t) = birdtest::scheduler::claim(&state, &w, &split_caps("3.0.0")).await.unwrap() {
                got += usize::from(t.job_id == small);
                total += 1;
            }
        }
        holder.rollback().await.unwrap();
        got += split_run(&state, 300, |_| false).await.get(&small).copied().unwrap_or(0);
        total += 300;
    }
    // Fair is a tenth; each spell's lead, forgiven, was about ten more.
    let fair = total / 10;
    assert!(got <= fair + 5, "the 10% job got {got} of {total} claims where {fair} is fair");
}

/// I-SCHED-3v: a newcomer found busy once is still settled. Not settled at all
/// for ten minutes after it was found busy, a newcomer everyone can run, in the
/// split of I-SCHED-3j, took the majority's claims until it had caught theirs:
/// the majority job's first came 331st (the audit's pass 22). It is settled a
/// ratio unit short instead -- ten claims of a 10% job.
#[tokio::test]
async fn a_newcomer_busy_once_is_still_settled() {
    let db = TestDb::new().await;
    let admin = db.user("root", true).await;
    let (state, a, _) = minority_split(&db).await;
    let n = db.games_job(1, 2).await;
    sqlx::query("UPDATE jobs SET status = 'inactive', allocation = NULL WHERE id = $1").bind(n).execute(&db.pool).await.unwrap();
    split_activate(&db, &state, admin, n, 10).await;
    // One claim while another holder keeps N's dispatch lock.
    let mut holder = db.pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(1, hashtext($1::text))")
        .bind(n).execute(&mut *holder).await.unwrap();
    let w = birdtest::auth::WorkerIdentity::Unregistered { uuid: Uuid::new_v4(), client_ip: std::net::IpAddr::from([127, 0, 0, 1]) };
    birdtest::scheduler::claim(&state, &w, &split_caps("1.0.0")).await.unwrap();
    holder.rollback().await.unwrap();
    let majority = majority_claims(&state, 1000).await;
    let got = majority.iter().filter(|j| **j == a).count();
    let first = majority.iter().position(|j| *j == a);
    assert!(first.is_some_and(|i| i < 16), "A's first claim at {first:?}");
    assert!((630..=690).contains(&got), "A got {got} of the majority's 800, not about 667");
}
