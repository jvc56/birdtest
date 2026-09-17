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

/// Bug: a claim lapses when no heartbeat has arrived for the timeout, and
/// nothing asked whether the server had been there to receive one. After an
/// outage longer than the timeout -- a deployment that went badly, a database
/// maintenance window -- every open claim in the fleet looked that old, however
/// alive its worker, and the first claim request after the restart abandoned
/// all of them: every task in flight was handed out again and every result
/// being computed came back `accepted: false`. A process reclaims nothing until
/// it has been up for the timeout itself; a live worker's next heartbeat
/// arrives well inside that, and a claim still silent afterwards is reclaimed
/// as before.
#[tokio::test]
async fn a_restarted_server_does_not_abandon_claims_it_could_not_have_heard_from() {
    let db = TestDb::new().await;
    db.games_job(1, 2).await;

    // The outage: a worker claimed, and the server was away for an hour.
    let before = birdtest::app(db.state().await);
    let (assignment, uuid) = first_claim(&before).await;
    let token = assignment["claim_token"].as_str().unwrap().to_string();
    sqlx::query("UPDATE task_claims SET claimed_at = now() - interval '1 hour'")
        .execute(&db.pool)
        .await
        .unwrap();

    // The process that comes back has heard from nobody yet.
    let mut restarted = db.state().await;
    restarted.reclaim_from = std::time::Instant::now() + restarted.cfg.heartbeat_timeout;
    let app = birdtest::app(restarted);

    let (other, _) = first_claim(&app).await;
    assert_ne!(
        other["task_request"]["seed"], assignment["task_request"]["seed"],
        "the first claim after a restart re-dispatched a task whose worker is still playing it"
    );
    let open: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM task_claims WHERE state = 'claimed'")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(open, 2, "both claims are still live");

    // The worker that played through the outage is heard from, and its result
    // lands.
    let (status, _) = send(
        &app,
        post_json("/api/worker/heartbeat", &[("x-worker-uuid", &uuid)], json!({ "claim_token": token })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, body) = submit_as(&app, &uuid, &token, games_result(2, 1)).await;
    assert_eq!(body, json!({ "accepted": true }), "the work done through the outage was thrown away");
}

/// The other half: the grace is a delay, not an amnesty. Once the process has
/// been up for the timeout, a claim that never spoke is reclaimed by the next
/// claim request, exactly as before.
#[tokio::test]
async fn a_claim_still_silent_after_the_grace_is_reclaimed() {
    let db = TestDb::new().await;
    db.games_job(1, 2).await;
    let mut state = db.state().await;
    state.reclaim_from = std::time::Instant::now() + std::time::Duration::from_millis(300);
    let app = birdtest::app(state);

    let (assignment, _) = first_claim(&app).await;
    sqlx::query("UPDATE task_claims SET claimed_at = now() - interval '1 hour'")
        .execute(&db.pool)
        .await
        .unwrap();

    let (during, _) = first_claim(&app).await;
    assert_ne!(during["task_request"]["seed"], assignment["task_request"]["seed"]);

    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    let (after, _) = first_claim(&app).await;
    assert_eq!(
        after["task_request"]["seed"], assignment["task_request"]["seed"],
        "a claim that stayed silent past the grace was never handed out again"
    );
}

/// Bug: a dashboard's SSE stream has no end of its own, and graceful shutdown
/// waits for every open response -- so with one job page open anywhere, SIGTERM
/// was followed by nothing until the container runtime's SIGKILL, thirty
/// seconds later, on a service whose old task has to be gone before the new
/// one starts. The stream ends when the process is told to stop.
#[tokio::test]
async fn a_live_stats_stream_ends_when_the_server_is_told_to_stop() {
    use tower::ServiceExt;

    let db = TestDb::new().await;
    let job = db.games_job(1, 2).await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());

    let response = app
        .oneshot(get_request(&format!("/api/jobs/{job}/stream"), &[]))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = tokio::spawn(axum::body::to_bytes(response.into_body(), 1024 * 1024));

    // Still open: nothing but a shutdown ends it.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(!body.is_finished(), "the stream ended by itself");

    state.shutdown.trigger();
    let bytes = tokio::time::timeout(std::time::Duration::from_secs(5), body)
        .await
        .expect("the stream outlived the shutdown signal")
        .unwrap()
        .unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("event: stats"), "the first payload was sent before the end: {text}");
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

    // The job list reads a running total instead of re-deriving this on
    // every page view, and it has to answer 2 for the same reason.
    assert_eq!(job_row.games_completed, 2, "the running total counts one result per task");
    let (_, list) = send(&app, get_request("/api/jobs", &[])).await;
    assert_eq!(list["items"][0]["units_completed"], json!(2));
}

/// Bug: the running total decided "first accepted result for this task" by
/// counting the task's result rows, before the task row was locked. Two
/// submissions for a redundancy-2 task's two slots, arriving together, each
/// counted only their own uncommitted rows, both concluded they were first, and
/// the job list showed every such batch twice. Submissions for one task now
/// serialize on the task row before anything is stored.
///
/// Deterministic rather than timing-dependent: an outside transaction holds the
/// job row, so both submissions get as far as they can and wait on a lock
/// before either commits -- the exact interleaving that double-counted.
#[tokio::test]
async fn concurrent_redundant_results_count_once() {
    let db = TestDb::new().await;
    let job = db.games_job(2, 2).await;
    let app = birdtest::app(db.state().await);

    let (a, uuid_a) = first_claim(&app).await;
    let (b, uuid_b) = first_claim(&app).await;
    assert_eq!(a["task_request"]["seed"], b["task_request"]["seed"], "same task, two slots");
    let token_a = a["claim_token"].as_str().unwrap();
    let token_b = b["claim_token"].as_str().unwrap();

    let mut blocker = db.pool.begin().await.unwrap();
    sqlx::query("SELECT 1 FROM jobs WHERE id = $1 FOR UPDATE")
        .bind(job)
        .execute(&mut *blocker)
        .await
        .unwrap();

    let release = async {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let waiting: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM pg_stat_activity
                 WHERE datname = current_database() AND wait_event_type = 'Lock'",
            )
            .fetch_one(&db.pool)
            .await
            .unwrap();
            if waiting >= 2 {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "both submissions should block");
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        blocker.commit().await.unwrap();
    };

    let ((status_a, body_a), (status_b, body_b), ()) = tokio::join!(
        submit_as(&app, &uuid_a, token_a, games_result(2, 2)),
        submit_as(&app, &uuid_b, token_b, games_result(2, 2)),
        release,
    );
    assert_eq!((status_a, status_b), (StatusCode::OK, StatusCode::OK), "{body_a} {body_b}");
    assert_eq!((&body_a["accepted"], &body_b["accepted"]), (&json!(true), &json!(true)));

    let job_row = birdtest::jobstats::load_job(&db.pool, job).await.unwrap();
    assert_eq!(job_row.games_completed, 2, "two games were played, not four");
}

/// The opening-rack detail page reads a running count of analysed racks
/// rather than counting distinct racks over every stored analysis, which at a
/// million racks took seconds on every view and every live push.
#[tokio::test]
async fn analysed_racks_are_counted_once_per_task_as_they_arrive() {
    let db = TestDb::new().await;
    let admin = db.user("admin", true).await;
    let player = db.static_player("solver", admin).await;
    let job = db.bare_job("opening_rack", 2, admin).await;
    sqlx::query(
        "INSERT INTO job_opening_rack_config
             (job_id, player_config_id, racks_per_batch, rack_size, total_racks)
         VALUES ($1, $2, 2, 7, 100)",
    )
    .bind(job)
    .bind(player)
    .execute(&db.pool)
    .await
    .unwrap();
    let app = birdtest::app(db.state().await);

    let (a, uuid_a) = first_claim(&app).await;
    let (b, uuid_b) = first_claim(&app).await;
    let racks: Vec<String> = a["task_request"]["racks"]
        .as_array()
        .expect("an opening-rack assignment carries racks")
        .iter()
        .map(|r| r.as_str().unwrap().to_string())
        .collect();
    assert_eq!(racks.len(), 2);

    let result = json!({ "racks": racks.iter().map(|rack| json!({
        "rack": rack,
        "num_moves": 1,
        "moves": [{ "move": "8G WUZ", "score": 30, "equity": 32.5 }],
    })).collect::<Vec<_>>() });

    // Both slots of the one task, so a second accepted result must not count
    // the same racks again.
    for uuid in [&uuid_a, &uuid_b] {
        let token = if uuid == &uuid_a {
            a["claim_token"].as_str().unwrap()
        } else {
            b["claim_token"].as_str().unwrap()
        };
        let (status, body) = submit_as(&app, uuid, token, result.clone()).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    let job_row = birdtest::jobstats::load_job(&db.pool, job).await.unwrap();
    assert_eq!(job_row.racks_analyzed, 2);
    let stats = birdtest::jobstats::compute(&db.pool, &job_row).await.unwrap();
    let racks = stats.opening_racks.expect("an opening-rack job reports rack stats");
    assert_eq!(racks.racks_analyzed, 2, "two racks were analysed, by two workers");
}

/// Bug: any error claiming from one job failed the whole claim, so a single
/// job that could not dispatch -- here, one whose config row is missing --
/// answered every worker with a 500 for as long as it led the candidate list.
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

/// A ban refuses the identity itself, on every worker endpoint.
///
/// The check rides in the same statement that resolves the identity, which is
/// what keeps it off a second round trip on every claim -- so it is worth a
/// test that it is still actually made, for both kinds of worker. Without it a
/// banned contributor keeps claiming and submitting and nothing says so.
#[tokio::test]
async fn a_banned_identity_is_refused_however_it_authenticates() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    db.games_job(1, 2).await;

    // An anonymous worker, banned by the UUID the server minted for it.
    let (_, anon) = first_claim(&app).await;
    sqlx::query("INSERT INTO worker_bans (anon_uuid) VALUES ($1::uuid)")
        .bind(&anon)
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, _) = send(
        &app,
        post_json("/api/worker/task", &[("x-worker-uuid", &anon)], claim_body("1.0.0", &[])),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "a banned anonymous worker cannot claim");
    let (status, _) = send(
        &app,
        post_json(
            "/api/worker/heartbeat",
            &[("x-worker-uuid", &anon)],
            json!({ "claim_token": Uuid::new_v4() }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "nor heartbeat for one it already held");

    // An authenticated worker, banned by user id.
    let user = db.user("bannedcontributor", false).await;
    let raw_key = "bt_".to_string() + &"a".repeat(64);
    sqlx::query("INSERT INTO api_keys (user_id, key_hash) VALUES ($1, $2)")
        .bind(user)
        .bind(birdtest::auth::api_key::hash_key(&raw_key))
        .execute(&db.pool)
        .await
        .unwrap();
    let bearer = format!("Bearer {raw_key}");

    // The key works before the ban...
    let (status, _) = send(
        &app,
        post_json("/api/worker/task", &[("authorization", &bearer)], claim_body("1.0.0", &[])),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "an unbanned key claims normally");

    sqlx::query("INSERT INTO worker_bans (user_id) VALUES ($1)")
        .bind(user)
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, _) = send(
        &app,
        post_json("/api/worker/task", &[("authorization", &bearer)], claim_body("1.0.0", &[])),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "and not after it");
}

/// Bug: a claim token worked for any registered identity, so a banned worker
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

/// Bug: public endpoints published anonymous workers' UUIDs, which are their
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

/// Bug: an opening-rack submission was never checked against the task it
/// answered.
///
/// Every other job type has that rule -- a batch reports exactly what was
/// dispatched -- and opening racks are where it bites hardest in both
/// directions. Answering with fewer racks still completed the task, leaving a
/// hole in the rack space nothing revisits, because the job's finish condition
/// only asks whether every task completed. Answering with racks from nowhere
/// stored them as analyses of this job and added them to `jobs.racks_analyzed`,
/// the progress counter the dashboard reads.
#[tokio::test]
async fn an_opening_rack_result_must_answer_the_racks_it_was_given() {
    let db = TestDb::new().await;
    let admin = db.user("admin", true).await;
    let player = db.static_player("solver", admin).await;
    let job = db.bare_job("opening_rack", 1, admin).await;
    sqlx::query(
        "INSERT INTO job_opening_rack_config
             (job_id, player_config_id, racks_per_batch, rack_size, total_racks)
         VALUES ($1, $2, 3, 7, 100)",
    )
    .bind(job)
    .bind(player)
    .execute(&db.pool)
    .await
    .unwrap();
    let app = birdtest::app(db.state().await);

    let (assignment, uuid) = first_claim(&app).await;
    let token = assignment["claim_token"].as_str().unwrap().to_string();
    let racks: Vec<String> = assignment["task_request"]["racks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap().to_string())
        .collect();
    assert_eq!(racks.len(), 3);

    let analysed = |racks: &[String]| {
        json!({ "racks": racks.iter().map(|rack| json!({
            "rack": rack,
            "num_moves": 1,
            "moves": [{ "move": "8G WUZ", "score": 30, "equity": 32.5 }],
        })).collect::<Vec<_>>() })
    };

    // Short of what was dispatched.
    let (status, body) = submit_as(&app, &uuid, &token, analysed(&racks[..1])).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["message"].as_str().unwrap().contains("dispatched"), "{body}");

    // The right number of racks, but not the ones asked for. The count check
    // alone would let this through, which is why the set is compared.
    let mut invented = racks.clone();
    invented[1] = "ZZZZZZZ".to_string();
    let (status, body) = submit_as(&app, &uuid, &token, analysed(&invented)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["message"].as_str().unwrap().contains("ZZZZZZZ"), "{body}");

    // The right number, all of them dispatched, but one twice -- so one is
    // missing. A 400 that says so, not the unique index's 409.
    let mut doubled = racks.clone();
    doubled[2] = racks[0].clone();
    let (status, body) = submit_as(&app, &uuid, &token, analysed(&doubled)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["message"].as_str().unwrap().contains("twice"), "{body}");

    // Nothing was stored or counted by any attempt, and the claim is still
    // open for the real answer.
    let job_row = birdtest::jobstats::load_job(&db.pool, job).await.unwrap();
    assert_eq!(job_row.racks_analyzed, 0);

    // Order is not part of the contract, only the set.
    let mut reordered = racks.clone();
    reordered.reverse();
    let (status, body) = submit_as(&app, &uuid, &token, analysed(&reordered)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let job_row = birdtest::jobstats::load_job(&db.pool, job).await.unwrap();
    assert_eq!(job_row.racks_analyzed, 3);
}

/// Bug: concurrent claims against one job collided on the seed cursor and the
/// losers were answered `204` while work existed.
///
/// The next seed is `MAX(seed)`, which a concurrent claim's uncommitted task is
/// invisible to, so overlapping claims all compute the same one. The
/// `(job_id, seed)` unique index catches that, but only by failing the loser,
/// and `scheduler::claim` gives up after three attempts -- so past three-way
/// contention a worker was told there was nothing to do. Claims for one job
/// already serialize on the `jobs` row (`claims_issued`), so taking the job's
/// dispatch lock before reading the cursor costs nothing that was not already
/// being paid and turns the lost race into a short wait.
#[tokio::test]
async fn concurrent_claims_tile_the_seed_space_instead_of_colliding() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 10).await;
    let app = birdtest::app(db.state().await);

    // Well past the three retries the old path allowed.
    const WORKERS: usize = 8;
    let claims = futures::future::join_all((0..WORKERS).map(|_| {
        let app = app.clone();
        async move { send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await }
    }))
    .await;

    let mut seeds = Vec::new();
    for (status, body) in &claims {
        assert_eq!(status, &StatusCode::OK, "a worker was told there was no work: {body}");
        seeds.push(body["task_request"]["seed"].as_str().unwrap().to_string());
    }
    seeds.sort();
    seeds.dedup();
    assert_eq!(seeds.len(), WORKERS, "every claim got its own slice of the seed space");

    let tasks: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tasks WHERE job_id = $1")
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(tasks, WORKERS as i64);
}

/// Positions and their ranked moves go out in multi-row statements rather than
/// one per position, so the thing worth pinning is that each record still ends
/// up with *its own* moves.
///
/// A batch is `racks_per_batch` positions -- 500 by default, up to 10,000 --
/// and the ids come back in insertion order, which is what lines them up. Get
/// the order wrong and every rack is stored with another rack's analysis:
/// well-formed, plausible, and silently false.
#[tokio::test]
async fn a_batched_opening_rack_submission_keeps_each_racks_own_moves() {
    let db = TestDb::new().await;
    let admin = db.user("admin", true).await;
    let player = db.static_player("solver", admin).await;
    let job = db.bare_job("opening_rack", 1, admin).await;
    sqlx::query(
        "INSERT INTO job_opening_rack_config
             (job_id, player_config_id, racks_per_batch, rack_size, total_racks)
         VALUES ($1, $2, 8, 7, 100)",
    )
    .bind(job)
    .bind(player)
    .execute(&db.pool)
    .await
    .unwrap();
    let app = birdtest::app(db.state().await);

    let (assignment, uuid) = first_claim(&app).await;
    let token = assignment["claim_token"].as_str().unwrap().to_string();
    let racks: Vec<String> = assignment["task_request"]["racks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap().to_string())
        .collect();
    assert_eq!(racks.len(), 8);

    // Each rack gets a move naming itself, and two ranked moves, so both the
    // record ordering and the per-record rank are checked.
    let result = json!({
        "racks": racks.iter().enumerate().map(|(i, rack)| json!({
            "rack": rack,
            "moves": [
                { "move": format!("best-{rack}"), "score": 30 + i as i32, "equity": 32.5 },
                { "move": format!("second-{rack}"), "score": 10, "equity": 12.5 },
            ],
        })).collect::<Vec<_>>()
    });
    let (status, body) = submit_as(&app, &uuid, &token, result).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let stored: Vec<(String, i16, String)> = sqlx::query_as(
        "SELECT r.rack, m.rank, m.move
         FROM position_analysis_records r
         JOIN position_analysis_moves m ON m.record_id = r.id
         JOIN tasks t ON t.id = r.task_id
         WHERE t.job_id = $1
         ORDER BY r.rack, m.rank",
    )
    .bind(job)
    .fetch_all(&db.pool)
    .await
    .unwrap();

    assert_eq!(stored.len(), 16, "two moves for each of eight racks");
    for (rack, rank, play) in &stored {
        let expected = if *rank == 1 { format!("best-{rack}") } else { format!("second-{rack}") };
        assert_eq!(play, &expected, "rack {rack} rank {rank} got another rack's move");
    }
}

/// Captured in-game positions are keyed on the task, not the claim, so a
/// redundant claim replaying the same deterministic games records nothing new.
///
/// Written as one multi-row insert with `ON CONFLICT DO NOTHING`, what comes
/// back is a *subset* of what went in, so the second claim's moves must be
/// matched to the rows that actually landed rather than zipped against the
/// whole batch. Zipped, the second claim would attach its moves to the wrong
/// records, or to none.
#[tokio::test]
async fn redundant_captured_positions_are_recorded_once() {
    let db = TestDb::new().await;
    let job = db.games_job(2, 2).await;
    sqlx::query("UPDATE job_game_config SET capture_positions = true WHERE job_id = $1")
        .bind(job)
        .execute(&db.pool)
        .await
        .unwrap();
    let app = birdtest::app(db.state().await);

    let positions = json!([
        { "game_index": 0, "turn_number": 0, "rack": "AEINRST", "position": "cgp-0",
          "num_moves": 40, "moves": [{ "move": "8D RETAINS", "score": 74, "equity": 81.2 }] },
        { "game_index": 1, "turn_number": 0, "rack": "AEINRSU", "position": "cgp-1",
          "num_moves": 30, "moves": [{ "move": "8D URINATES", "score": 70, "equity": 77.0 }] },
    ]);
    let mut result = games_result(2, 1);
    result["positions"] = positions;

    // Two independent workers on the same task, both replaying the same games.
    let (first, first_uuid) = first_claim(&app).await;
    let (second, second_uuid) = first_claim(&app).await;
    assert_eq!(first["task_request"]["seed"], second["task_request"]["seed"], "the same task");

    for (uuid, assignment) in [(&first_uuid, &first), (&second_uuid, &second)] {
        let token = assignment["claim_token"].as_str().unwrap();
        let (status, body) = submit_as(&app, uuid, token, result.clone()).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["accepted"], true, "{body}");
    }

    let (records, moves): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM position_analysis_records r WHERE r.job_id = $1),
                (SELECT COUNT(*) FROM position_analysis_moves m
                 JOIN position_analysis_records r ON r.id = m.record_id
                 WHERE r.job_id = $1)",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(records, 2, "redundancy must not multiply the corpus");
    assert_eq!(moves, 2, "and the second claim's moves must not be written twice either");

    // Both aggregates are still stored separately: redundancy still verifies
    // the result, it just does not duplicate the positions.
    let results: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM game_results r JOIN tasks t ON t.id = r.task_id WHERE t.job_id = $1",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(results, 2);
}

/// `expected_data` is the union over the job and its players, deduplicated, and
/// it is what the client verifies before it runs anything.
///
/// It is one query over a union of the per-type config tables rather than a
/// match on `job_type` followed by two more queries, so this pins the answer
/// rather than the shape of the code: two players on one lexicon contribute one
/// `kwg` entry, a static player contributes no `winpct` entry at all, and a
/// leave-generation job -- which has no player config row -- carries exactly
/// the three files its bot loads.
#[tokio::test]
async fn an_assignment_names_every_file_the_task_loads_and_no_others() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);

    // Both players are static and share one config row, which is legal and is
    // the degenerate case the dedup has to survive.
    let admin = db.user("admin", true).await;
    let player = db.static_player("shared", admin).await;
    let job = db.bare_job("games", 1, admin).await;
    sqlx::query(
        "INSERT INTO job_game_config
             (job_id, player1_config_id, player2_config_id, games_per_batch, min_games, max_games)
         VALUES ($1, $2, $2, 1, 1000000, 1000000)",
    )
    .bind(job)
    .bind(player)
    .execute(&db.pool)
    .await
    .unwrap();

    let (assignment, _) = first_claim(&app).await;
    let mut roles: Vec<String> = assignment["expected_data"]["files"]
        .as_array()
        .expect("expected_data carries files")
        .iter()
        .map(|f| f["role"].as_str().unwrap().to_string())
        .collect();
    roles.sort();
    assert_eq!(
        roles,
        vec!["klv", "kwg", "layout", "letterdist"],
        "one entry per distinct file, and no winpct for a static player"
    );
    assert_eq!(assignment["expected_data"]["algorithm"], "sha256");
    for file in assignment["expected_data"]["files"].as_array().unwrap() {
        assert!(file["sha256"].as_str().unwrap().len() == 64, "{file}");
        assert!(file["path"].as_str().unwrap().contains('/'), "{file}");
        assert_eq!(file["tarball_date"], "20251004");
    }

    // And it states every setting the task needs rather than leaving one to
    // the worker's build, which MAGPIE refuses: the job's run-wide settings,
    // and each player's.
    let request = &assignment["task_request"];
    assert_eq!(request["bingo_bonus"], json!(50), "{request}");
    assert_eq!(request["sim_cutoff"], json!(0.005), "{request}");
    for field in ["sort_strategy", "num_plies", "num_plays", "num_plies_recorded", "movegen_margin"] {
        assert!(!request["player1"][field].is_null(), "player1 {field}: {request}");
    }
}

/// Contribution counters are running totals now, not counts over `task_claims`,
/// and the contributor lists read them. A counter that does not move, or moves
/// twice, is a wrong leaderboard that nothing else contradicts.
#[tokio::test]
async fn contributions_are_counted_as_they_arrive() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 2).await;
    let app = birdtest::app(db.state().await);

    let counters = |uuid: String| {
        let pool = db.pool.clone();
        async move {
            sqlx::query_as::<_, (i64, Option<chrono::DateTime<chrono::Utc>>)>(
                "SELECT tasks_completed, last_completed_at FROM anonymous_workers WHERE uuid = $1::uuid",
            )
            .bind(uuid)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };

    let (assignment, uuid) = first_claim(&app).await;
    assert_eq!(counters(uuid.clone()).await, (0, None), "a claim is not a contribution");

    let token = assignment["claim_token"].as_str().unwrap();
    let (_, body) = submit_as(&app, &uuid, token, games_result(2, 1)).await;
    assert_eq!(body["accepted"], true);

    let (completed, last) = counters(uuid.clone()).await;
    assert_eq!(completed, 1);
    assert!(last.is_some(), "the timestamp is the last task finished, not the last request");

    // And a second task moves it again.
    let (status, assignment) = claim_as(&app, &uuid).await;
    assert_eq!(status, StatusCode::OK, "{assignment}");
    let token = assignment["claim_token"].as_str().unwrap();
    submit_as(&app, &uuid, token, games_result(2, 2)).await;
    assert_eq!(counters(uuid.clone()).await.0, 2);

    // The job's own task counters track creation and completion separately.
    let (total, completed): (i64, i64) =
        sqlx::query_as("SELECT tasks_total, tasks_completed FROM jobs WHERE id = $1")
            .bind(job)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!((total, completed), (2, 2));

    // And the leaderboard reads them rather than counting claims.
    let (status, body) = send(&app, get_request("/api/workers", &[])).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["total"], 1);
    assert_eq!(body["items"][0]["tasks_completed"], 2);
    assert!(body["items"][0]["last_seen_at"].is_string(), "{body}");
}

/// The results feed pages by cursor, because a job's corpus is millions of rows
/// and `OFFSET` produces every one of them before the page asked for.
///
/// What this pins is that the cursor actually walks the whole set exactly once:
/// a keyset that ties (`submitted_at` is transaction time, so a whole batch
/// shares it) would silently repeat or skip rows between pages.
#[tokio::test]
async fn the_results_feed_walks_every_row_exactly_once() {
    let db = TestDb::new().await;
    let admin = db.user("admin", true).await;
    let player = db.static_player("solver", admin).await;
    let job = db.bare_job("opening_rack", 1, admin).await;
    sqlx::query(
        "INSERT INTO job_opening_rack_config
             (job_id, player_config_id, racks_per_batch, rack_size, total_racks)
         VALUES ($1, $2, 6, 7, 100)",
    )
    .bind(job)
    .bind(player)
    .execute(&db.pool)
    .await
    .unwrap();
    let app = birdtest::app(db.state().await);

    // Two batches, so the feed spans more than one submission timestamp and
    // more than one page.
    let mut expected: Vec<String> = Vec::new();
    for _ in 0..2 {
        let (assignment, uuid) = first_claim(&app).await;
        let token = assignment["claim_token"].as_str().unwrap().to_string();
        let racks: Vec<String> = assignment["task_request"]["racks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r.as_str().unwrap().to_string())
            .collect();
        expected.extend(racks.iter().cloned());
        let result = json!({
            "racks": racks.iter().map(|rack| json!({
                "rack": rack,
                "moves": [{ "move": "8G WUZ", "score": 30, "equity": 32.5 }],
            })).collect::<Vec<_>>()
        });
        let (status, body) = submit_as(&app, &uuid, &token, result).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    let mut seen: Vec<String> = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..10 {
        let path = match &cursor {
            Some(c) => format!("/api/jobs/{job}/results?per_page=5&cursor={c}"),
            None => format!("/api/jobs/{job}/results?per_page=5"),
        };
        let (status, body) = send(&app, get_request(&path, &[])).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        for item in body["items"].as_array().unwrap() {
            seen.push(item["rack"].as_str().unwrap().to_string());
        }
        match body["next_cursor"].as_str() {
            Some(next) => cursor = Some(next.to_string()),
            None => break,
        }
    }

    assert_eq!(seen.len(), expected.len(), "the walk repeated or skipped rows: {seen:?}");
    let mut sorted_seen = seen.clone();
    sorted_seen.sort();
    let mut sorted_expected = expected.clone();
    sorted_expected.sort();
    assert_eq!(sorted_seen, sorted_expected);

    // An unparseable cursor starts from the beginning rather than erroring: it
    // is opaque, so a caller cannot be expected to repair one.
    let (status, body) =
        send(&app, get_request(&format!("/api/jobs/{job}/results?cursor=nonsense"), &[])).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["items"].as_array().unwrap().len(), expected.len());
}

/// A worker that ran a task and could not produce an accepted result hands the
/// slot straight back, rather than holding it for the heartbeat timeout.
#[tokio::test]
async fn a_failed_task_is_handed_straight_back() {
    let db = TestDb::new().await;
    db.games_job(1, 2).await;
    let app = birdtest::app(db.state().await);

    let (assignment, uuid) = first_claim(&app).await;
    let (status, body) = send(
        &app,
        post_json(
            "/api/worker/decline",
            &[("x-worker-uuid", uuid.as_str())],
            json!({ "claim_token": assignment["claim_token"], "reason": "task_failed" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    let (next, _) = first_claim(&app).await;
    assert_eq!(
        next["task_request"]["seed"], assignment["task_request"]["seed"],
        "the same task goes to the next worker at once"
    );
}

/// Waits until at least `count` sessions of this test's database are blocked on
/// a lock, so a test can release a blocker at the moment the interleaving it
/// needs has happened.
async fn wait_for_lock_waiters(db: &TestDb, count: i64) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pg_stat_activity
             WHERE datname = current_database() AND wait_event_type = 'Lock'",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        if waiting >= count {
            return;
        }
        assert!(std::time::Instant::now() < deadline, "expected {count} session(s) to block");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

/// Bug: a claim that selected a job while it was active went on to hand out a
/// task after the job had been completed (or deactivated) underneath it. The
/// claim waited on the job's row lock and then updated it regardless.
///
/// Deterministic: the completion is held uncommitted until the claim is blocked
/// on the job's row, which is the interleaving that issued the task.
#[tokio::test]
async fn a_claim_racing_a_jobs_completion_hands_nothing_out() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 2).await;
    let app = birdtest::app(db.state().await);

    let mut completer = db.pool.begin().await.unwrap();
    sqlx::query("UPDATE jobs SET status = 'completed' WHERE id = $1")
        .bind(job)
        .execute(&mut *completer)
        .await
        .unwrap();

    let release = async {
        wait_for_lock_waiters(&db, 1).await;
        completer.commit().await.unwrap();
    };
    let ((status, body), ()) = tokio::join!(
        send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))),
        release,
    );
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    let claims: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM task_claims")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let tasks: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tasks WHERE job_id = $1")
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!((claims, tasks), (0, 0), "nothing is issued against a completed job");
}

/// Bug: games and game-pairs jobs generated tasks past `max_games` /
/// `max_pairs`. Nothing those tasks played could change the verdict, and a
/// busy fleet generated one per worker until the debounced finish check ran.
#[tokio::test]
async fn sprt_jobs_hand_out_nothing_past_their_cap() {
    let db = TestDb::new().await;
    let games = db.games_job(1, 2).await;
    sqlx::query("UPDATE job_game_config SET max_games = 3 WHERE job_id = $1")
        .bind(games)
        .execute(&db.pool)
        .await
        .unwrap();
    let app = birdtest::app(db.state().await);

    // Seeds 1 and 3 cover four games, which reaches the cap of three.
    let (first, _) = first_claim(&app).await;
    let (second, _) = first_claim(&app).await;
    assert_eq!((first["task_request"]["seed"].as_str(), second["task_request"]["seed"].as_str()),
               (Some("1"), Some("3")));
    let (status, body) =
        send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "a third batch would start past the cap: {body}");

    // The same cap, counted in pairs.
    sqlx::query("UPDATE jobs SET status = 'completed' WHERE id = $1")
        .bind(games)
        .execute(&db.pool)
        .await
        .unwrap();
    let admin = db.user("pairs-admin", true).await;
    let p1 = db.static_player("pairs-p1", admin).await;
    let p2 = db.static_player("pairs-p2", admin).await;
    let pairs = db.bare_job("game_pairs", 1, admin).await;
    sqlx::query(
        "INSERT INTO job_game_pair_config
             (job_id, player1_config_id, player2_config_id, pairs_per_batch, min_pairs, max_pairs)
         VALUES ($1, $2, $3, 1, 1, 1)",
    )
    .bind(pairs)
    .bind(p1)
    .bind(p2)
    .execute(&db.pool)
    .await
    .unwrap();
    let (only, _) = first_claim(&app).await;
    assert_eq!(only["job_id"].as_str(), Some(pairs.to_string().as_str()));
    let (status, body) =
        send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
}

/// A pool's page shows the residuals its latest fit stored, not ones rebuilt
/// on each view. Rebuilding ran the pool's evidence matrix -- a grouped scan
/// over every paired result it counts -- on every public page view. Stored,
/// the residuals describe the evidence that fit used, even after more arrives.
#[tokio::test]
async fn a_pools_residuals_are_the_ones_its_latest_fit_stored() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let admin = db.user("pool-admin", true).await;
    // Named so the pool orders the anchor first: it is the residual's row, and
    // the job's player 1.
    let anchor = db.static_player("anchor", admin).await;
    let rival = db.static_player("rival", admin).await;
    let job = db.bare_job("game_pairs", 1, admin).await;
    sqlx::query(
        "INSERT INTO job_game_pair_config
             (job_id, player1_config_id, player2_config_id, pairs_per_batch, min_pairs, max_pairs)
         VALUES ($1, $2, $3, 1, 1000000, 1000000)",
    )
    .bind(job)
    .bind(anchor)
    .bind(rival)
    .execute(&db.pool)
    .await
    .unwrap();
    let pool: Uuid = sqlx::query_scalar(
        "INSERT INTO rating_pools (name, variant, letterdist_id, layout_id, anchor_player_config_id)
         SELECT 'pool', variant, letterdist_id, layout_id, $2 FROM jobs WHERE id = $1
         RETURNING id",
    )
    .bind(job)
    .bind(anchor)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO rating_pool_members (pool_id, player_config_id) VALUES ($1, $2), ($1, $3)",
    )
    .bind(pool)
    .bind(anchor)
    .bind(rival)
    .execute(&db.pool)
    .await
    .unwrap();

    // One pair, both games to player 1, or both to player 2.
    let pair = |player_one_won: bool| {
        let mut result = games_result(2, if player_one_won { 2 } else { 0 });
        result["pentanomial"] =
            if player_one_won { json!([0, 0, 0, 0, 1]) } else { json!([1, 0, 0, 0, 0]) };
        result
    };

    let (assignment, uuid) = first_claim(&app).await;
    let token = assignment["claim_token"].as_str().unwrap();
    let (_, body) = submit_as(&app, &uuid, token, pair(true)).await;
    assert_eq!(body["accepted"], true, "{body}");

    let run = birdtest::ratings::recompute(&db.pool, pool, birdtest::ratings::Trigger::Manual)
        .await
        .unwrap();
    let stored: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM rating_run_residuals WHERE run_id = $1")
            .bind(run)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(stored, 1, "one head-to-head, stored with the run");

    let pool_page = format!("/api/rating-pools/{pool}");
    let (status, detail) = send(&app, get_request(&pool_page, &[])).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    let residuals = detail["residuals"].as_array().unwrap();
    assert_eq!(residuals.len(), 1, "{detail}");
    assert_eq!(residuals[0]["row"], json!(anchor), "{detail}");
    assert_eq!(residuals[0]["col"], json!(rival), "{detail}");
    assert_eq!(residuals[0]["pairs"], json!(1.0), "{detail}");
    assert_eq!(residuals[0]["actual"], json!(1.0), "{detail}");

    // A second pair the other way, and no refit. The page still shows the
    // fit's evidence, where a rebuilt matrix would show two pairs, split.
    let (status, assignment) = claim_as(&app, &uuid).await;
    assert_eq!(status, StatusCode::OK, "{assignment}");
    let token = assignment["claim_token"].as_str().unwrap();
    let (_, body) = submit_as(&app, &uuid, token, pair(false)).await;
    assert_eq!(body["accepted"], true, "{body}");
    let (_, detail) = send(&app, get_request(&pool_page, &[])).await;
    assert_eq!(detail["residuals"][0]["pairs"], json!(1.0), "{detail}");
    assert_eq!(detail["residuals"][0]["actual"], json!(1.0), "{detail}");
}

// ---------------------------------------------------------------------------
// Derived files: wordmaps and rack info tables
// ---------------------------------------------------------------------------

/// An identity the claim endpoint recognises, without having to be handed a
/// task to be issued one. A worker is minted with its first assignment, and
/// several tests below need to claim against a job that has none to give.
async fn registered_worker(db: &TestDb) -> String {
    let uuid = Uuid::new_v4();
    sqlx::query("INSERT INTO anonymous_workers (uuid) VALUES ($1)")
        .bind(uuid)
        .execute(&db.pool)
        .await
        .unwrap();
    uuid.to_string()
}

/// A player config that asks for a wordmap and, optionally, a rack info table.
/// Written with plain SQL like every other builder here: what is under test is
/// dispatch, not the validator.
///
/// `kwg` and `klv` are passed in rather than created, because two players
/// sharing a lexicon must share the *row*: `input_data` rows are identified by
/// content, so two rows both named `NWL23` are two different lexicons that
/// happen to share a name, and would correctly need two different wordmaps.
async fn deriving_player(
    db: &TestDb,
    name: &str,
    kwg: Uuid,
    klv: Uuid,
    use_rit: bool,
) -> Uuid {
    let admin = db.user(&format!("admin{}", Uuid::new_v4().simple()), true).await;
    sqlx::query_scalar(
        "INSERT INTO player_configs
             (name, recorder_type, sort_strategy, kwg_id, klv_id, num_plies, num_plays,
              num_plies_recorded, num_plays_recorded, use_wordmap, use_rit,
              movegen_margin, created_by)
         VALUES ($1, 'best', 'equity', $2, $3, 0, 100, 2, 10, true, $4, 5, $5)
         RETURNING id",
    )
    .bind(name)
    .bind(kwg)
    .bind(klv)
    .bind(use_rit)
    .bind(admin)
    .fetch_one(&db.pool)
    .await
    .unwrap()
}

/// A `games` job between two given player configs.
async fn job_between(db: &TestDb, p1: Uuid, p2: Uuid) -> Uuid {
    let admin = db.user(&format!("admin{}", Uuid::new_v4().simple()), true).await;
    let job = db.bare_job("games", 1, admin).await;
    sqlx::query(
        "INSERT INTO job_game_config
             (job_id, player1_config_id, player2_config_id, games_per_batch, min_games, max_games)
         VALUES ($1, $2, $3, 10, 1000000, 1000000)",
    )
    .bind(job)
    .bind(p1)
    .bind(p2)
    .execute(&db.pool)
    .await
    .unwrap();
    job
}

/// The gate. A wordmap and a rack info table are built on the contributor's own
/// machine, and what makes that checkable is the hash the server publishes —
/// so a job whose hash does not exist yet must not be handed out. Dispatching
/// early would send a worker no `derived` entry, which it reads as a server
/// that checks nothing: the exact state this replaces, reached silently.
#[tokio::test]
async fn a_job_is_not_dispatched_until_its_derived_files_are_built() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let kwg = db.input_data("kwg", "NWL23").await;
    let klv = db.input_data("klv", "NWL23").await;
    let p1 = deriving_player(&db, "wmp-p1", kwg, klv, false).await;
    let p2 = deriving_player(&db, "wmp-p2", kwg, klv, false).await;
    let job = job_between(&db, p1, p2).await;
    let worker = registered_worker(&db).await;

    let (status, body) = claim_as(&app, &worker).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "nothing is built yet: {body}");

    // Two players on one lexicon need one wordmap, not two.
    assert_eq!(db.derived_ready(job).await, 1);

    let (status, body) = claim_as(&app, &worker).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let derived = body["expected_data"]["derived"].as_array().expect("derived");
    assert_eq!(derived.len(), 1, "{body}");
    assert_eq!(derived[0]["role"], "wmp");
    assert_eq!(derived[0]["name"], "NWL23");
    assert_eq!(derived[0]["builder"], "wmp-1");
    assert!(derived[0]["sha256"].is_string(), "{body}");
}

/// Once a job has been found dispatchable, its hashes are answered from memory
/// for the rest of the process: the query behind them ran for every candidate
/// job on every claim, and its answer for a dispatchable job cannot change
/// (see `derived::DerivedCache`). Pinned by removing the rows behind the
/// answer -- not a path anything real takes, but the one observation that
/// tells a remembered answer from a re-read one -- and by forgetting the job,
/// after which the gate is consulted again.
#[tokio::test]
async fn a_dispatchable_jobs_hashes_are_remembered_for_the_process() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let kwg = db.input_data("kwg", "NWL23").await;
    let klv = db.input_data("klv", "NWL23").await;
    let p1 = deriving_player(&db, "cache-p1", kwg, klv, false).await;
    let p2 = deriving_player(&db, "cache-p2", kwg, klv, false).await;
    let job = job_between(&db, p1, p2).await;
    let worker = registered_worker(&db).await;

    // Waiting is never remembered: the job is dispatched the moment it is built.
    let (status, _) = claim_as(&app, &worker).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(state.derived_ready.get(job).is_none(), "a waiting job is not remembered");
    db.derived_ready(job).await;
    let (status, first) = claim_as(&app, &worker).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let remembered = state.derived_ready.get(job).expect("a dispatchable job is remembered");
    assert_eq!(remembered.len(), 1);

    // Submit, so the worker can be handed the job's next task, then take the
    // rows away: the next claim can only carry the hash if it was remembered.
    let token = first["claim_token"].as_str().unwrap();
    let (status, body) = submit_as(&app, &worker, token, games_result(10, 5)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    sqlx::query("DELETE FROM derived_data").execute(&db.pool).await.unwrap();
    let (status, second) = claim_as(&app, &worker).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(
        second["expected_data"]["derived"], first["expected_data"]["derived"],
        "the remembered hashes are the ones the claim carries"
    );

    // Forgotten, the gate is consulted again -- and now finds nothing built.
    // A second worker asks, since the first has spent its request burst.
    let token = second["claim_token"].as_str().unwrap();
    submit_as(&app, &worker, token, games_result(10, 5)).await;
    state.derived_ready.forget(job);
    let (status, body) = claim_as(&app, &registered_worker(&db).await).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
}

/// A failed build blocks dispatch exactly as an unbuilt one does. "Give up and
/// send it anyway" is the wrong recovery — the worker would fall back to
/// whatever is on its disk, unchecked — and must not be reachable by accident.
#[tokio::test]
async fn a_failed_derived_build_keeps_a_job_undispatched() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let kwg = db.input_data("kwg", "NWL23").await;
    let klv = db.input_data("klv", "NWL23").await;
    let p1 = deriving_player(&db, "fail-p1", kwg, klv, false).await;
    let p2 = deriving_player(&db, "fail-p2", kwg, klv, false).await;
    let job = job_between(&db, p1, p2).await;
    db.derived_ready(job).await;
    sqlx::query(
        "UPDATE derived_data SET state = 'failed', sha256 = NULL, bytes = NULL,
                                 error = 'the lexicon has no stored bytes'",
    )
    .execute(&db.pool)
    .await
    .unwrap();

    let (status, body) = claim_as(&app, &registered_worker(&db).await).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
}

/// A rack info table belongs to a (lexicon, leaves) pair, not to a lexicon,
/// because it stores precomputed leave values that move generation uses in
/// place of the loaded KLV. MAGPIE's CLI finds one by lexicon name alone, which
/// is how a player pinning NWL23 words and CSW21 leaves — a pairing birdtest
/// accepts on purpose — would have ranked every full rack on NWL23's leaves.
/// The claim names the pair, so that is not expressible.
#[tokio::test]
async fn a_rack_info_table_is_pinned_under_its_pairs_name() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let kwg = db.input_data("kwg", "NWL23").await;
    let p1 = deriving_player(&db, "rit-p1", kwg, db.input_data("klv", "CSW21").await, true).await;
    let p2 = deriving_player(&db, "rit-p2", kwg, db.input_data("klv", "NWL23").await, false).await;
    let job = job_between(&db, p1, p2).await;

    // One wordmap for the shared lexicon, and one table for p1's pair. p1 asks
    // for the table; the wordmap it is built from is needed either way.
    assert_eq!(db.derived_ready(job).await, 2);

    let (status, body) = claim_as(&app, &registered_worker(&db).await).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let derived = body["expected_data"]["derived"].as_array().expect("derived");
    let rit = derived.iter().find(|d| d["role"] == "rit").expect("a rit entry");
    assert_eq!(rit["name"], "NWL23.CSW21", "{body}");
    assert_eq!(rit["builder"], "rit-1");

    // And the task request names the same file, so the worker loads what the
    // server pinned rather than inferring a name from the lexicon.
    let request = &body["task_request"];
    assert_eq!(request["player1"]["rit_name"], "NWL23.CSW21", "{body}");
    assert!(request["player2"]["rit_name"].is_null(), "{body}");
}

/// A worker that built the file and got different bytes hands the claim back
/// with both digests. Distinct from `missing_data`, which is a file the
/// contributor was supposed to download: nothing the contributor can do fixes
/// this one, and the two hashes are what makes a disagreement between the
/// fleet's builders visible instead of silently worked around.
#[tokio::test]
async fn a_derived_mismatch_releases_the_claim_and_records_both_hashes() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let kwg = db.input_data("kwg", "NWL23").await;
    let klv = db.input_data("klv", "NWL23").await;
    let p1 = deriving_player(&db, "mm-p1", kwg, klv, false).await;
    let p2 = deriving_player(&db, "mm-p2", kwg, klv, false).await;
    let job = job_between(&db, p1, p2).await;
    db.derived_ready(job).await;

    let uuid = registered_worker(&db).await;
    let (status, body) = claim_as(&app, &uuid).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let token = body["claim_token"].as_str().unwrap().to_string();

    let (status, decline) = send(
        &app,
        post_json(
            "/api/worker/decline",
            &[("x-worker-uuid", uuid.as_str())],
            json!({
                "claim_token": token,
                "reason": "derived_mismatch",
                "missing": [{
                    "role": "wmp", "name": "NWL23",
                    "expected": "a".repeat(64), "actual": "b".repeat(64),
                }],
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{decline}");

    let (role, expected, actual) = sqlx::query_as::<_, (String, String, Option<String>)>(
        "SELECT role, expected, actual FROM worker_data_gaps WHERE job_id = $1",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(role, "wmp");
    assert_eq!(expected, "a".repeat(64));
    assert_eq!(actual.as_deref(), Some("b".repeat(64).as_str()));

    // The claim is handed straight back rather than held for the heartbeat
    // timeout: the task is fine, this worker cannot run it.
    let open: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM task_claims WHERE state = 'claimed'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(open, 0);
}

/// Each player's derived files come from that player's own rows, so two players
/// on different lexicons need two wordmaps rather than sharing one.
///
/// Nothing is shared between players, so this ought to fall out of the query —
/// but "ought to" is what a test is for, and comparing bots on two lexicons is
/// a supported configuration that a wordmap keyed on the wrong player would
/// silently break.
#[tokio::test]
async fn players_on_different_lexicons_need_a_wordmap_each() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let nwl = db.input_data("kwg", "NWL23").await;
    let csw = db.input_data("kwg", "CSW21").await;
    let klv = db.input_data("klv", "NWL23").await;
    let p1 = deriving_player(&db, "two-lex-p1", nwl, klv, false).await;
    let p2 = deriving_player(&db, "two-lex-p2", csw, klv, false).await;
    let job = job_between(&db, p1, p2).await;

    assert_eq!(db.derived_ready(job).await, 2);

    let (status, body) = claim_as(&app, &registered_worker(&db).await).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let derived = body["expected_data"]["derived"].as_array().expect("derived");
    let mut names: Vec<&str> =
        derived.iter().map(|d| d["name"].as_str().unwrap()).collect();
    names.sort_unstable();
    assert_eq!(names, ["CSW21", "NWL23"], "{body}");
    // Two lexicons, two hashes: a single wordmap standing in for both is the
    // failure this rules out.
    let hashes: std::collections::HashSet<&str> =
        derived.iter().map(|d| d["sha256"].as_str().unwrap()).collect();
    assert_eq!(hashes.len(), 2, "{body}");
}

/// A job's immutable configuration -- its players, its letter distribution,
/// its `expected_data` -- is read once per process and kept
/// (`jobs::dispatch::JobTemplates`), so the claim transaction reads only what
/// changes from claim to claim. A purge deletes results and tasks and leaves
/// the configuration alone, so the template stands across one; deleting the
/// job is what forgets it.
#[tokio::test]
async fn a_jobs_template_is_read_once_survives_a_purge_and_goes_with_the_job() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 2).await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let admin = db.user("root", true).await;
    let headers = admin_headers(&state.cfg, admin);
    let borrowed: Vec<(&str, &str)> =
        headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();

    assert!(state.templates.get(job).is_none(), "nothing is read before the first claim");
    let (first, uuid) = first_claim(&app).await;
    let template = state.templates.get(job).expect("the first claim reads the template");
    assert_eq!(
        template.expected.len(),
        first["expected_data"]["files"].as_array().unwrap().len(),
        "the assignment's expected_data is the template's"
    );

    let (status, body) =
        send(&app, post_json(&format!("/api/admin/jobs/{job}/purge"), &borrowed, json!({}))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(state.templates.get(job).is_some(), "a purge changes no configuration");

    // The purged job starts its space over, and every claim after it carries
    // exactly what the first did.
    let (status, again) = claim_as(&app, &uuid).await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(again["task_request"]["seed"], "1");
    assert_eq!(again["expected_data"]["files"], first["expected_data"]["files"]);
    assert_eq!(again["task_request"]["player1"], first["task_request"]["player1"]);
    assert_eq!(again["task_request"]["player2"], first["task_request"]["player2"]);
    assert_eq!(again["task_request"]["bingo_bonus"], first["task_request"]["bingo_bonus"]);

    let delete = axum::http::Request::delete(format!("/api/admin/jobs/{job}"))
        .header("cookie", headers[0].1.as_str())
        .header("x-csrf-token", headers[1].1.as_str())
        .body(axum::body::Body::empty())
        .unwrap();
    let (status, body) = send(&app, delete).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert!(state.templates.get(job).is_none(), "a deleted job is forgotten");
}

/// There is no priority: an admin parks a job at 0%, which is offered to
/// nobody, exactly as an inactive one is. Every claim goes to the active job
/// above 0% that is furthest behind its share.
#[tokio::test]
async fn a_job_at_zero_allocation_is_offered_to_nobody() {
    let db = TestDb::new().await;
    let parked = db.games_job(1, 2).await;
    let running = db.games_job(1, 2).await;
    sqlx::query("UPDATE jobs SET allocation = 0 WHERE id = $1")
        .bind(parked)
        .execute(&db.pool)
        .await
        .unwrap();
    let app = birdtest::app(db.state().await);

    for _ in 0..3 {
        let (assignment, _) = first_claim(&app).await;
        assert_eq!(assignment["job_id"], running.to_string(), "only the job above 0% is offered");
    }

    // With the running job parked too, active jobs exist and nothing rules
    // them out, so the answer is "nothing right now" rather than a shutdown.
    sqlx::query("UPDATE jobs SET allocation = 0 WHERE id = $1")
        .bind(running)
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, body) =
        send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
}

/// Every task carries the seed its games are played from, and every assignment
/// states it: an opening-rack task's is the index of its first rack, so rack
/// `i` of the batch is analysed from `seed + i` on every worker alike.
#[tokio::test]
async fn every_assignment_states_the_seed_its_task_was_stored_with() {
    let db = TestDb::new().await;
    let admin = db.user("root", true).await;
    let player = db.static_player("analyser", admin).await;
    let job = db.bare_job("opening_rack", 1, admin).await;
    sqlx::query(
        "INSERT INTO job_opening_rack_config
             (job_id, player_config_id, racks_per_batch, rack_size, total_racks)
         VALUES ($1, $2, 3, 2, 1000)",
    )
    .bind(job)
    .bind(player)
    .execute(&db.pool)
    .await
    .unwrap();
    let app = birdtest::app(db.state().await);

    let (first, uuid) = first_claim(&app).await;
    assert_eq!(first["task_request"]["seed"], "0", "the first slice starts the space");
    assert_eq!(first["task_request"]["racks"].as_array().unwrap().len(), 3);
    let (status, second) = claim_as(&app, &uuid).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["task_request"]["seed"], "3", "the next slice's seed is its first rack's index");

    let stored: Vec<i64> = sqlx::query_scalar(
        "SELECT seed FROM tasks WHERE job_id = $1 ORDER BY created_at",
    )
    .bind(job)
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(stored, vec![0, 3], "the assignment's seed is the task's");
}

/// Bug: `shutdown_or_idle` counted every active job, parked ones included, so a
/// job at 0% -- which is offered to nobody, exactly as an inactive one is --
/// could still get a worker told to shut down: a parked job with a floor above
/// the worker's MAGPIE answered `magpie_too_old`, and a parked job in its
/// unsupported set answered `data_out_of_date`. The same job switched to
/// `inactive` answered `204`. A contributor who exits on that is not there when
/// the admin raises a job it could have run.
#[tokio::test]
async fn a_parked_job_shuts_nobody_down() {
    let db = TestDb::new().await;
    let too_new = db.games_job(1, 2).await;
    sqlx::query(
        "UPDATE jobs SET allocation = 0, min_magpie_major = 2, min_magpie_minor = 0,
                         min_magpie_patch = 0
         WHERE id = $1",
    )
    .bind(too_new)
    .execute(&db.pool)
    .await
    .unwrap();
    let unsupported = db.games_job(1, 2).await;
    sqlx::query("UPDATE jobs SET allocation = 0 WHERE id = $1")
        .bind(unsupported)
        .execute(&db.pool)
        .await
        .unwrap();
    let app = birdtest::app(db.state().await);

    // Both axes would rule this worker out, and neither job is on offer.
    let (status, body) =
        send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[unsupported]))).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    // Raised above 0%, the same jobs are what the worker is told about.
    sqlx::query("UPDATE jobs SET allocation = 50 WHERE id = ANY($1)")
        .bind(vec![too_new, unsupported])
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, body) =
        send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[unsupported]))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["shutdown"]["reason"], "both", "{body}");
}

/// Bug: the admin results stream and the export of an opening-rack job wrote
/// the `position_analysis_records` row alone -- the rack, how many moves were
/// ranked, when -- and none of the moves. The artifact PLAN.md names as the
/// path for analysing the corpus held no move, score or equity at all.
#[tokio::test]
async fn an_opening_rack_corpus_carries_each_racks_ranked_moves() {
    let db = TestDb::new().await;
    let admin = db.user("admin", true).await;
    let player = db.static_player("solver", admin).await;
    let job = db.bare_job("opening_rack", 1, admin).await;
    sqlx::query(
        "INSERT INTO job_opening_rack_config
             (job_id, player_config_id, racks_per_batch, rack_size, total_racks)
         VALUES ($1, $2, 3, 7, 100)",
    )
    .bind(job)
    .bind(player)
    .execute(&db.pool)
    .await
    .unwrap();
    let cfg = db.config();
    let app = birdtest::app(db.state().await);

    let (assignment, uuid) = first_claim(&app).await;
    let token = assignment["claim_token"].as_str().unwrap().to_string();
    let racks: Vec<String> = assignment["task_request"]["racks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap().to_string())
        .collect();
    let result = json!({
        "racks": racks.iter().map(|rack| json!({
            "rack": rack,
            "num_moves": 40,
            "moves": [
                { "move": format!("best-{rack}"), "score": 30, "equity": 32.5,
                  "win_percentage": 55.0, "blended_utility": 0.6,
                  "plies": [ { "ply": 0, "bingo_percentage": 1.5, "average_score": 24.0 },
                             { "ply": 1, "bingo_percentage": 2.5, "average_score": 31.0 } ] },
            ],
        })).collect::<Vec<_>>()
    });
    let (status, body) = submit_as(&app, &uuid, &token, result).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let path = format!("/api/admin/jobs/{job}/results/stream");
    let (status, body) = send(&app, get_request(&path, &admin_headers(&cfg, admin))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // Newline-delimited, so not one JSON document: the harness hands it back
    // as text.
    let text = body.as_str().expect("an NDJSON body").to_string();
    let lines: Vec<serde_json::Value> =
        text.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
    assert_eq!(lines.len(), 3, "one line per analysed rack: {text}");
    for line in &lines {
        let rack = line["rack"].as_str().unwrap();
        assert_eq!(line["num_moves"], 40);
        let moves = line["moves"].as_array().expect("a record carries its moves");
        assert_eq!(moves.len(), 1, "{line}");
        assert_eq!(moves[0]["rank"], 1);
        assert_eq!(moves[0]["move"], format!("best-{rack}"));
        assert_eq!(moves[0]["equity"], 32.5);
        assert_eq!(moves[0]["win_percentage"], 55.0);
        let plies = moves[0]["plies"].as_array().expect("a move carries its plies");
        assert_eq!(plies.len(), 2, "{line}");
        assert_eq!(plies[1]["ply"], 1);
        assert_eq!(plies[1]["average_score"], 31.0);
    }
}

/// Bug: `?worker=` was applied to every row of the job -- a username compare
/// or a SHA-256 of the claim's UUID per record, behind two joins -- so a page
/// for a name nobody has read the whole job to find nothing: seconds per
/// request at a million records, on a public route with no limit on it. The
/// name is resolved to an identity first now, a name that belongs to nobody is
/// an empty page without reading the job, and the filter is an equality on the
/// claim's identity column.
#[tokio::test]
async fn the_results_feed_filters_by_who_a_name_is() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let job = db.games_job(1, 2).await;

    // One result from an anonymous worker...
    let (claim, anon) = first_claim(&app).await;
    let token = claim["claim_token"].as_str().unwrap();
    let (_, body) = submit_as(&app, &anon, token, games_result(2, 1)).await;
    assert_eq!(body["accepted"], true, "{body}");
    let pseudonym = birdtest::auth::public_anon_id(anon.parse().unwrap());

    // ...and two from an account.
    let user = db.user("keyed", false).await;
    let raw_key = "bt_".to_string() + &"b".repeat(64);
    sqlx::query("INSERT INTO api_keys (user_id, key_hash) VALUES ($1, $2)")
        .bind(user)
        .bind(birdtest::auth::api_key::hash_key(&raw_key))
        .execute(&db.pool)
        .await
        .unwrap();
    let bearer = format!("Bearer {raw_key}");
    for _ in 0..2 {
        let (status, claim) = send(
            &app,
            post_json("/api/worker/task", &[("authorization", &bearer)], claim_body("1.0.0", &[])),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{claim}");
        let (status, body) = send(
            &app,
            post_json(
                "/api/worker/result",
                &[("authorization", &bearer)],
                json!({ "claim_token": claim["claim_token"], "result": games_result(2, 2) }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    let feed = |worker: &str| get_request(&format!("/api/jobs/{job}/results?worker={worker}"), &[]);

    let (status, body) = send(&app, feed("keyed")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 2, "{body}");
    assert!(items.iter().all(|item| item["username"] == "keyed"), "{body}");

    let (status, body) = send(&app, feed(&pseudonym)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{body}");
    assert_eq!(items[0]["anon_id"], pseudonym);

    // Nobody: a username that does not exist, and a well-formed pseudonym that
    // no contributor has.
    for nobody in ["no-such-contributor", "0123456789abcdef"] {
        let (status, body) = send(&app, feed(nobody)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["items"].as_array().unwrap().len(), 0, "{nobody}: {body}");
        assert!(body["next_cursor"].is_null(), "{body}");
    }

    // Unfiltered, all three.
    let (_, body) = send(&app, get_request(&format!("/api/jobs/{job}/results"), &[])).await;
    assert_eq!(body["items"].as_array().unwrap().len(), 3, "{body}");
}

/// The display pool is what keeps page views off the path workers wait on, and
/// its statement timeout is what bounds how long any one read holds one of its
/// connections. A cancelled read is load, not a fault: `503`, not `500`.
#[tokio::test]
async fn the_display_pool_bounds_its_reads() {
    let db = TestDb::new().await;
    let read_pool = birdtest::db::connect_read(&db.url).await.unwrap();

    let timeout: String =
        sqlx::query_scalar("SHOW statement_timeout").fetch_one(&read_pool).await.unwrap();
    assert_eq!(timeout, "15s");
    // The main pool is unbounded: a purge or a generation's seeding runs on it.
    let timeout: String =
        sqlx::query_scalar("SHOW statement_timeout").fetch_one(&db.pool).await.unwrap();
    assert_eq!(timeout, "0");

    // What a read past the bound turns into, without waiting fifteen seconds.
    let mut conn = read_pool.acquire().await.unwrap();
    sqlx::query("SET statement_timeout = '50ms'").execute(&mut *conn).await.unwrap();
    let cancelled = sqlx::query("SELECT pg_sleep(5)").execute(&mut *conn).await.unwrap_err();
    let err: birdtest::error::AppError = cancelled.into();
    assert_eq!(err.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(err.code, "unavailable");
    drop(conn);
    read_pool.close().await;
}

/// Gap: `capture_positions` exists to build a corpus, and a games job's export
/// and stream were its result rows only -- the positions had no way out of the
/// database. They are a second artifact of the export, and `?positions=true`
/// on the admin stream, in the shape an opening-rack line has.
#[tokio::test]
async fn a_games_jobs_captured_positions_can_be_streamed_out() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 2).await;
    sqlx::query("UPDATE job_game_config SET capture_positions = true WHERE job_id = $1")
        .bind(job)
        .execute(&db.pool)
        .await
        .unwrap();
    let cfg = db.config();
    let admin = db.user("root", true).await;
    let app = birdtest::app(db.state().await);

    let mut result = games_result(2, 1);
    result["positions"] = json!([
        { "game_index": 0, "turn_number": 0, "rack": "AEINRST", "position": "cgp-0",
          "num_moves": 40, "moves": [{ "move": "8D RETAINS", "score": 74, "equity": 81.2 }] },
        { "game_index": 1, "turn_number": 3, "rack": "AEINRSU", "position": "cgp-1",
          "previous_move": "8D DOG", "previous_move_score": 10,
          "num_moves": 30, "moves": [{ "move": "8D URINATES", "score": 70, "equity": 77.0 }] },
    ]);
    let (claim, uuid) = first_claim(&app).await;
    let (status, body) =
        submit_as(&app, &uuid, claim["claim_token"].as_str().unwrap(), result).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let headers = admin_headers(&cfg, admin);
    let ndjson = |body: serde_json::Value| -> Vec<serde_json::Value> {
        body.as_str()
            .map(|text| text.lines().map(|line| serde_json::from_str(line).unwrap()).collect())
            // A single line is one JSON document, which the harness parses.
            .unwrap_or_else(|| vec![body.clone()])
    };

    // The results stream is what it always was: the job's result rows.
    let path = format!("/api/admin/jobs/{job}/results/stream");
    let (status, body) = send(&app, get_request(&path, &headers)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let results = ndjson(body);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["games"], 2);
    assert!(results[0].get("moves").is_none(), "{results:?}");

    let (status, body) = send(&app, get_request(&format!("{path}?positions=true"), &headers)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let mut positions = ndjson(body);
    positions.sort_by_key(|p| p["game_index"].as_i64());
    assert_eq!(positions.len(), 2, "{positions:?}");
    assert_eq!(positions[1]["position"], "cgp-1");
    assert_eq!(positions[1]["turn_number"], 3);
    assert_eq!(positions[1]["previous_move"], "8D DOG");
    assert_eq!(positions[1]["moves"][0]["move"], "8D URINATES");

    // An opening-rack job's stream already is its positions.
    let player = db.static_player("solver", admin).await;
    let racks = db.bare_job("opening_rack", 1, admin).await;
    sqlx::query(
        "INSERT INTO job_opening_rack_config
             (job_id, player_config_id, racks_per_batch, rack_size, total_racks)
         VALUES ($1, $2, 3, 7, 100)",
    )
    .bind(racks)
    .bind(player)
    .execute(&db.pool)
    .await
    .unwrap();
    let (status, body) = send(
        &app,
        get_request(&format!("/api/admin/jobs/{racks}/results/stream?positions=true"), &headers),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

/// PLAN.md singles this error out: a claim with no body is what a MAGPIE older
/// than the contribute protocol sends, and the answer is what its contributor
/// reads, so it has to name the fix. It was axum's own plain-text `422` --
/// outside the API's error shape and its list of statuses, like every other
/// body that failed to parse.
#[tokio::test]
async fn a_claim_without_a_usable_body_is_told_what_to_send() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);

    for body in ["", "{}", "{\"unsupported_jobs\": []}", "not json"] {
        let request = axum::http::Request::post("/api/worker/task")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body))
            .unwrap();
        let (status, answer) = send(&app, request).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}: {answer}");
        assert_eq!(answer["code"], "bad_request", "{body:?}: {answer}");
        let message = answer["message"].as_str().expect("a JSON error body");
        assert!(message.contains("magpie_version"), "{message}");
        assert!(message.contains("update MAGPIE"), "{message}");
    }

    // The same shape from a cookie-backed route.
    let (status, answer) =
        send(&app, post_json("/api/auth/login", &[], json!({ "username": 5 }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
    assert_eq!(answer["code"], "bad_request", "{answer}");
}
