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
    submit_with_movegens(app, uuid, token, result, json!(1000)).await
}

/// A submission reporting `movegens` as given (a number, or anything else a
/// client might send); `Value::Null` leaves the field out.
async fn submit_with_movegens(
    app: &axum::Router,
    uuid: &str,
    token: &str,
    result: serde_json::Value,
    movegens: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let mut body = json!({ "claim_token": token, "result": result });
    if !movegens.is_null() {
        body["movegens"] = movegens;
    }
    send(app, post_json("/api/worker/result", &[("x-worker-uuid", uuid)], body)).await
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

    db.games_job(2).await;
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

/// Bug: the submit path read the claim outside its transaction and marked it
/// completed unconditionally. A timeout reclaiming it in between left the
/// claim both abandoned and completed and the task's live count decremented
/// twice. The sequential version of that race: a result for a reclaimed claim
/// is refused and leaves the counters alone, and a retried submission of an
/// accepted result is an `accepted: false`, not a 500.
#[tokio::test]
async fn submissions_for_reclaimed_or_already_accepted_claims_change_nothing() {
    let db = TestDb::new().await;
    let job = db.games_job(2).await;
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
    db.games_job(2).await;

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
    db.games_job(2).await;
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
    let job = db.games_job(2).await;
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

/// The opening-rack detail page reads a running count of analysed racks
/// rather than counting distinct racks over every stored analysis, which at a
/// million racks took seconds on every view and every live push.
#[tokio::test]
async fn analysed_racks_are_counted_as_they_arrive() {
    let db = TestDb::new().await;
    let admin = db.user("admin", true).await;
    let player = db.static_player("solver", admin).await;
    let job = db.bare_job("opening_rack", admin).await;
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

    // Two workers, a task each: a task has one slot.
    let (a, uuid_a) = first_claim(&app).await;
    let (b, uuid_b) = first_claim(&app).await;
    assert_ne!(a["task_request"]["seed"], b["task_request"]["seed"], "a task each");
    for (assignment, uuid) in [(&a, &uuid_a), (&b, &uuid_b)] {
        let racks: Vec<String> = assignment["task_request"]["racks"]
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
        let token = assignment["claim_token"].as_str().unwrap();
        let (status, body) = submit_as(&app, uuid, token, result).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    let job_row = birdtest::jobstats::load_job(&db.pool, job).await.unwrap();
    assert_eq!(job_row.racks_analyzed, 4);
    let stats = birdtest::jobstats::compute(&db.pool, &job_row).await.unwrap();
    let racks = stats.opening_racks.expect("an opening-rack job reports rack stats");
    assert_eq!(racks.racks_analyzed, 4, "four racks were analysed, two by each worker");

    // Each worker is credited with its own claim's movegens, which the claim
    // keeps for a purge to give back.
    for uuid in [&uuid_a, &uuid_b] {
        let credited: (i64, i64) = sqlx::query_as(
            "SELECT w.movegens, c.movegens FROM anonymous_workers w
             JOIN task_claims c ON c.claimed_by_anon_uuid = w.uuid
             WHERE w.uuid = $1::uuid",
        )
        .bind(uuid)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(credited, (1000, 1000), "{uuid}");
    }
}

/// I-SCHED-21: an opening-rack consensus job analyses its rack space once,
/// then reissues the racks that are not yet settled -- fewest analyses first,
/// none another task is analysing, none the claiming worker analysed when it
/// can be helped -- as lists, from seeds past the end of the space. A rack
/// settles once at least two analyses agree completely, or at its third
/// without a consensus, and the job completes once every rack is settled.
#[tokio::test]
async fn a_consensus_job_reissues_its_unsettled_racks_until_each_settles() {
    let db = TestDb::new().await;
    let admin = db.user("admin", true).await;
    let player = db.static_player("analyst", admin).await;
    let job = db.bare_job("opening_rack", admin).await;
    // Written directly: job creation refuses a consensus for a static player,
    // and nothing here depends on how the moves were ranked.
    sqlx::query(
        "INSERT INTO job_opening_rack_config
             (job_id, player_config_id, racks_per_batch, rack_size, total_racks,
              consensus_pct, min_results_per_rack, max_results_per_rack)
         VALUES ($1, $2, 2, 7, 4, 100, 2, 3)",
    )
    .bind(job)
    .bind(player)
    .execute(&db.pool)
    .await
    .unwrap();
    let app = birdtest::app(db.state().await);

    let racks_of = |assignment: &serde_json::Value| -> Vec<String> {
        assignment["task_request"]["racks"]
            .as_array()
            .expect("an opening-rack assignment carries racks")
            .iter()
            .map(|r| r.as_str().unwrap().to_string())
            .collect()
    };
    // Each rack's best move as the worker reports it.
    let submit = |assignment: serde_json::Value, uuid: String, best: Vec<(String, &'static str)>| {
        let app = app.clone();
        async move {
            let result = json!({ "racks": best.iter().map(|(rack, play)| json!({
                "rack": rack, "num_moves": 1,
                "moves": [{ "move": play, "score": 30, "equity": 32.5 }],
            })).collect::<Vec<_>>() });
            let token = assignment["claim_token"].as_str().unwrap().to_string();
            let (status, body) = submit_as(&app, &uuid, &token, result).await;
            assert_eq!((status, &body), (StatusCode::OK, &json!({ "accepted": true })));
        }
    };
    let counters = || async {
        sqlx::query_as::<_, (i64, i64, i64, String)>(
            "SELECT racks_analyzed, racks_settled, racks_without_consensus, status::text
             FROM jobs WHERE id = $1",
        )
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap()
    };

    // The first pass: the space, two racks a task, from seeds 0 and 2.
    let (a, uuid_a) = first_claim(&app).await;
    let (b, uuid_b) = first_claim(&app).await;
    assert_eq!((a["task_request"]["seed"].clone(), b["task_request"]["seed"].clone()), (json!("0"), json!("2")));
    let (first_a, first_b) = (racks_of(&a), racks_of(&b));
    let [r1, r2] = [first_a[0].clone(), first_a[1].clone()];
    let [r3, r4] = [first_b[0].clone(), first_b[1].clone()];
    submit(a, uuid_a.clone(), vec![(r1.clone(), "8G WUZ"), (r2.clone(), "8G WUZ")]).await;
    submit(b, uuid_b.clone(), vec![(r3.clone(), "8G WUZ"), (r4.clone(), "8G WUZ")]).await;
    assert_eq!(counters().await, (4, 0, 0, "active".into()), "analysed once, none settled");

    // Reissues: past the end of the space, each worker given the racks the
    // other analysed, and never one another task holds.
    let (status, again_a) = claim_as(&app, &uuid_a).await;
    assert_eq!(status, StatusCode::OK, "{again_a}");
    assert_eq!(again_a["task_request"]["seed"], json!("4"));
    let mut expected = vec![r3.clone(), r4.clone()];
    expected.sort();
    let mut got = racks_of(&again_a);
    got.sort();
    assert_eq!(got, expected, "A analyses what B did");
    let (status, again_b) = claim_as(&app, &uuid_b).await;
    assert_eq!(status, StatusCode::OK, "{again_b}");
    assert_eq!(again_b["task_request"]["seed"], json!("6"));
    let mut expected = vec![r1.clone(), r2.clone()];
    expected.sort();
    let mut got = racks_of(&again_b);
    got.sort();
    assert_eq!(got, expected, "B analyses what A did");
    let listed: Vec<Option<Vec<String>>> = sqlx::query_scalar(
        "SELECT r.racks FROM opening_rack_requests r JOIN tasks t ON t.id = r.task_id
         WHERE t.job_id = $1 ORDER BY t.seed",
    )
    .bind(job)
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(listed.iter().map(Option::is_some).collect::<Vec<_>>(), [false, false, true, true]);

    // r1, r2 and r3 agree twice and settle; r4 splits.
    submit(again_a, uuid_a.clone(), vec![(r3.clone(), "8G WUZ"), (r4.clone(), "8G ZOA")]).await;
    submit(again_b, uuid_b.clone(), vec![(r1.clone(), "8G WUZ"), (r2.clone(), "8G WUZ")]).await;
    assert_eq!(counters().await, (4, 3, 0, "active".into()));

    // Both workers have analysed r4, the last unsettled rack: it goes to A
    // anyway rather than to nobody.
    let (status, last) = claim_as(&app, &uuid_a).await;
    assert_eq!(status, StatusCode::OK, "{last}");
    assert_eq!(racks_of(&last), vec![r4.clone()]);
    // Nothing else is free meanwhile.
    let (status, _) = claim_as(&app, &uuid_b).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Its third analysis disagrees again: settled at its most, without a
    // consensus, and the job is done.
    submit(last, uuid_a.clone(), vec![(r4.clone(), "8G QI")]).await;
    assert_eq!(counters().await, (4, 4, 1, "completed".into()));
    let standing: (i32, String, i32, bool, bool) = sqlx::query_as(
        "SELECT results, top_move, top_count, settled, without_consensus
         FROM opening_rack_progress WHERE job_id = $1 AND rack = $2",
    )
    .bind(job)
    .bind(&r4)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(standing, (3, "8G QI".into(), 1, true, true));

    // The page's lookup numbers each analysis of a rack.
    let query = r4.replace('?', "%3F");
    let (_, page) = send(&app, get_request(&format!("/api/jobs/{job}/results?rack={query}"), &[])).await;
    let analyses: Vec<i64> = page["items"].as_array().unwrap().iter().map(|m| m["analysis"].as_i64().unwrap()).collect();
    assert_eq!(analyses, [1, 2, 3], "{page}");

    // Its lists are held, together, to what one analysis can record: at
    // 12,000 moves each, each analysis lists its best 32,767 / 3, from the
    // top. The lookup is public, and a hundred analyses of 32,767 moves
    // each came back whole.
    sqlx::query(
        "INSERT INTO position_analysis_moves (record_id, rank, move, score, equity)
         SELECT r.id, g, '8G M' || g, 0, 0
         FROM position_analysis_records r, generate_series(2, 12000) g
         WHERE r.job_id = $1 AND r.rack = $2 AND r.game_index IS NULL",
    )
    .bind(job)
    .bind(&r4)
    .execute(&db.pool)
    .await
    .unwrap();
    let (_, page) = send(&app, get_request(&format!("/api/jobs/{job}/results?rack={query}"), &[])).await;
    let items = page["items"].as_array().unwrap();
    for analysis in 1..=3 {
        let ranks: Vec<i64> = items
            .iter()
            .filter(|m| m["analysis"] == json!(analysis))
            .map(|m| m["rank"].as_i64().unwrap())
            .collect();
        assert_eq!(ranks, (1..=10_922).collect::<Vec<i64>>(), "analysis {analysis}");
    }
}

/// A PATCH of an opening-rack job's consensus settings, as an admin.
async fn patch_consensus(
    app: &axum::Router,
    headers: &[(String, String)],
    job: Uuid,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let mut builder = axum::http::Request::patch(format!("/api/admin/jobs/{job}/consensus"))
        .header("content-type", "application/json");
    for (name, value) in headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    send(app, builder.body(axum::body::Body::from(body.to_string())).unwrap()).await
}

/// An opening-rack job over four racks, two to a task, with these consensus
/// settings, analysed by a simulating player (job creation and the edit
/// refuse a consensus to a static one).
async fn consensus_job(db: &TestDb, admin: Uuid, pct: f64, min: i32, max: i32) -> Uuid {
    let player = db.sim_player(&format!("simmer{}", Uuid::new_v4().simple()), admin).await;
    let job = db.bare_job("opening_rack", admin).await;
    sqlx::query(
        "INSERT INTO job_opening_rack_config
             (job_id, player_config_id, racks_per_batch, rack_size, total_racks,
              consensus_pct, min_results_per_rack, max_results_per_rack)
         VALUES ($1, $2, 2, 7, 4, $3, $4, $5)",
    )
    .bind(job)
    .bind(player)
    .bind(pct)
    .bind(min)
    .bind(max)
    .execute(&db.pool)
    .await
    .unwrap();
    job
}

/// I-OR-EDIT-1: an opening-rack job's consensus settings change after
/// creation, and the job starts and stops to match. A job wanting one
/// analysis per rack keeps a progress row per rack all the same, so raising
/// its settings after it completed reopens it and reissues every rack from
/// those rows; lowering them while a reissue is in flight settles every rack,
/// and that reissue's submission completes the job without counting its racks
/// a second time. Raising what is already satisfied leaves it completed, and
/// still demotes its final export: the standings it carries changed.
#[tokio::test]
async fn an_opening_rack_jobs_consensus_can_change_and_the_job_follows() {
    let db = TestDb::new().await;
    let admin = db.user("admin", true).await;
    let job = consensus_job(&db, admin, 100.0, 1, 1).await;
    let state = db.state().await;
    let headers = admin_headers(&state.cfg, admin);
    let app = birdtest::app(state);

    let racks_of = |assignment: &serde_json::Value| -> Vec<String> {
        assignment["task_request"]["racks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r.as_str().unwrap().to_string())
            .collect()
    };
    let submit = |assignment: serde_json::Value, uuid: String, play: &'static str| {
        let app = app.clone();
        async move {
            let racks: Vec<String> = assignment["task_request"]["racks"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r.as_str().unwrap().to_string())
                .collect();
            let result = json!({ "racks": racks.iter().map(|rack| json!({
                "rack": rack, "num_moves": 1,
                "moves": [{ "move": play, "score": 30, "equity": 32.5 }],
            })).collect::<Vec<_>>() });
            let token = assignment["claim_token"].as_str().unwrap().to_string();
            let (status, body) = submit_as(&app, &uuid, &token, result).await;
            assert_eq!((status, &body), (StatusCode::OK, &json!({ "accepted": true })));
        }
    };
    let counters = || async {
        sqlx::query_as::<_, (i64, i64, i64, String)>(
            "SELECT racks_analyzed, racks_settled, racks_without_consensus, status::text
             FROM jobs WHERE id = $1",
        )
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap()
    };
    let pool = &db.pool;
    let audit = |after: i64| async move {
        sqlx::query_as::<_, (String, Option<String>, Option<String>, Option<String>)>(
            "SELECT action, old_status, new_status, reason FROM audit_log
             WHERE job_id = $1 AND id > $2 ORDER BY id",
        )
        .bind(job)
        .bind(after)
        .fetch_all(pool)
        .await
        .unwrap()
    };
    let last_audit = || async {
        sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(id) FROM audit_log")
            .fetch_one(&db.pool)
            .await
            .unwrap()
            .unwrap_or(0)
    };

    // One analysis per rack: the first pass settles every rack, and the job
    // completes -- with a progress row per rack all the same.
    let (a, uuid_a) = first_claim(&app).await;
    let (b, uuid_b) = first_claim(&app).await;
    submit(a, uuid_a.clone(), "8G WUZ").await;
    submit(b, uuid_b.clone(), "8G WUZ").await;
    assert_eq!(counters().await, (4, 4, 0, "completed".into()));
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM opening_rack_progress WHERE job_id = $1")
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(rows, 4, "a job wanting one analysis per rack keeps its rows");
    // Its final export, which reopening it makes a snapshot.
    sqlx::query("INSERT INTO job_exports (job_id, state, is_final) VALUES ($1, 'ready', TRUE)")
        .bind(job)
        .execute(&db.pool)
        .await
        .unwrap();

    // Two agreeing analyses a rack now, three at most: every rack is
    // unsettled, and the completed job is reopened -- inactive at the 0% a
    // completed job holds, until it is given an allocation.
    let before = last_audit().await;
    let (status, body) =
        patch_consensus(&app, &headers, job, json!({ "min_results_per_rack": 2, "max_results_per_rack": 3 })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["reopened"], json!(true));
    assert_eq!(body["unsettled_racks"], json!(4));
    assert_eq!((&body["job"]["status"], &body["job"]["allocation"]), (&json!("inactive"), &json!(0)));
    assert_eq!(
        (body["config"]["min_results_per_rack"].clone(), body["config"]["max_results_per_rack"].clone()),
        (json!(2), json!(3))
    );
    assert_eq!(counters().await, (4, 0, 0, "inactive".into()));
    // The job list counts racks settled, as the job's page does: every rack
    // is analysed, and none is done.
    let listed = || async {
        let (status, page) = send(&app, get_request("/api/jobs", &[])).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        let item = page["items"].as_array().unwrap().iter().find(|j| j["id"] == json!(job.to_string())).cloned();
        let item = item.expect("the job is listed");
        (item["units_completed"].clone(), item["max_units"].clone())
    };
    assert_eq!(listed().await, (json!(0), json!(4)));
    assert_eq!(
        audit(before).await,
        vec![
            ("job.consensus_changed".into(), None, None, Some("min 1 -> 2, max 1 -> 3; 4 racks unsettled".into())),
            ("job.deactivated".into(), Some("completed".into()), Some("inactive".into()), None),
        ]
    );
    let finals: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM job_exports WHERE job_id = $1 AND is_final")
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(finals, 0, "the old final export is a snapshot now");
    let refs: Vec<(&str, &str)> = headers.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let (status, body) = send(&app, allocate(job, 50, &refs)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(counters().await, (4, 0, 0, "active".into()));

    // The reissues start from the rows: the racks the other worker analysed.
    let (status, again_a) = claim_as(&app, &uuid_a).await;
    assert_eq!(status, StatusCode::OK, "{again_a}");
    assert_eq!(again_a["task_request"]["seed"], json!("4"));
    let (status, again_b) = claim_as(&app, &uuid_b).await;
    assert_eq!(status, StatusCode::OK, "{again_b}");
    assert_eq!(racks_of(&again_a).len() + racks_of(&again_b).len(), 4);
    submit(again_a, uuid_a.clone(), "8G WUZ").await;
    assert_eq!(counters().await, (4, 2, 0, "active".into()), "two racks agree twice");
    assert_eq!(listed().await, (json!(2), json!(4)));

    // One analysis is enough again while B's reissue is in flight: every
    // rack settles, and the job waits for that reissue rather than
    // completing under it.
    let (status, body) =
        patch_consensus(&app, &headers, job, json!({ "min_results_per_rack": 1, "max_results_per_rack": 1 })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!((body["unsettled_racks"].clone(), body["reopened"].clone()), (json!(0), json!(false)));
    assert_eq!(counters().await, (4, 4, 0, "active".into()));
    // Its racks were analysed before, so they are not counted again; their
    // two analyses now disagree, which at one analysis a rack at most
    // settles them without a consensus.
    submit(again_b, uuid_b.clone(), "8G ZOA").await;
    assert_eq!(counters().await, (4, 4, 2, "completed".into()));
    // Each line of the corpus carries its rack's standing: every rack has two
    // analyses, which the job's maximum of one no longer hides.
    let response = tower::ServiceExt::oneshot(
        app.clone(),
        get_request(&format!("/api/admin/jobs/{job}/results/stream"), &headers),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let corpus = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
    let standings: Vec<serde_json::Value> = String::from_utf8(corpus.to_vec())
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap()["consensus"].clone())
        .collect();
    assert_eq!(standings.len(), 8, "four racks analysed twice each");
    assert!(standings.iter().all(|c| c["results"] == json!(2)), "{standings:?}");
    assert_eq!(standings.iter().filter(|c| c["without_consensus"] == json!(true)).count(), 4);

    // Asking for what every rack already has leaves it completed -- but its
    // standings are restated under the new share, so its final export, and
    // one still building from before the edit, no longer stand for it.
    sqlx::query(
        "INSERT INTO job_exports (job_id, state, is_final) VALUES ($1, 'ready', TRUE), ($1, 'running', FALSE)",
    )
    .bind(job)
    .execute(&db.pool)
    .await
    .unwrap();
    let (status, body) = patch_consensus(&app, &headers, job, json!({ "consensus_pct": 60 })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!((body["reopened"].clone(), body["job"]["status"].clone()), (json!(false), json!("completed")));
    let exports: Vec<(String, bool)> = sqlx::query_as(
        "SELECT state, is_final FROM job_exports WHERE job_id = $1 ORDER BY state",
    )
    .bind(job)
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        exports,
        vec![("failed".into(), false), ("ready".into(), false), ("ready".into(), false)],
        "a completed job's edit demotes its final export and fails a running one"
    );

    // A purge's census counts the rack standings it destroys with the rest.
    let mut purge = axum::http::Request::post(format!("/api/admin/jobs/{job}/purge"));
    for (name, value) in &headers {
        purge = purge.header(name.as_str(), value.as_str());
    }
    let (status, body) = send(&app, purge.body(axum::body::Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let census: String = sqlx::query_scalar(
        "SELECT reason FROM audit_log WHERE action = 'job.purged.census' AND target_id = $1",
    )
    .bind(job.to_string())
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(census.contains(" rack_standings=4 "), "{census}");
}

/// I-OR-EDIT-2: the edit refuses what creation refuses, and only for an
/// opening-rack job; a change that changes nothing writes nothing; and a
/// completed job it reopens comes back inactive at 0%, for the admin to give
/// an allocation.
#[tokio::test]
async fn a_consensus_edit_is_checked_and_reopens_inactive() {
    let db = TestDb::new().await;
    let admin = db.user("admin", true).await;
    let job = consensus_job(&db, admin, 100.0, 1, 1).await;
    let state = db.state().await;
    let headers = admin_headers(&state.cfg, admin);
    let app = birdtest::app(state);
    let fields = |body: &serde_json::Value| -> Vec<String> {
        body["fields"].as_array().map_or(Vec::new(), |f| {
            f.iter().map(|e| e["field"].as_str().unwrap().to_string()).collect()
        })
    };

    for (body, field) in [
        (json!({ "consensus_pct": 50 }), "consensus_pct"),
        (json!({ "min_results_per_rack": 0 }), "min_results_per_rack"),
        (json!({ "min_results_per_rack": 3, "max_results_per_rack": 2 }), "max_results_per_rack"),
        (json!({ "max_results_per_rack": 101 }), "max_results_per_rack"),
    ] {
        let (status, response) = patch_consensus(&app, &headers, job, body.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {response}");
        assert_eq!(fields(&response), vec![field.to_string()], "{body}: {response}");
    }

    // A static player's analyses always agree.
    let static_job = db.bare_job("opening_rack", admin).await;
    let analyst = db.static_player("analyst", admin).await;
    // Ranking every play, as a static opening-rack player must.
    sqlx::query("UPDATE player_configs SET recorder_type = 'all' WHERE id = $1")
        .bind(analyst)
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO job_opening_rack_config
             (job_id, player_config_id, racks_per_batch, rack_size, total_racks)
         VALUES ($1, $2, 2, 7, 4)",
    )
    .bind(static_job)
    .bind(analyst)
    .execute(&db.pool)
    .await
    .unwrap();
    let (status, response) =
        patch_consensus(&app, &headers, static_job, json!({ "max_results_per_rack": 2 })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{response}");
    assert_eq!(fields(&response), vec!["max_results_per_rack".to_string()]);

    // Only an opening-rack job has the settings.
    let games = db.games_job(2).await;
    let (status, response) = patch_consensus(&app, &headers, games, json!({ "consensus_pct": 80 })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{response}");

    // Nothing that differs: no audit row.
    let audited = || async {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM audit_log WHERE job_id = $1")
            .bind(job)
            .fetch_one(&db.pool)
            .await
            .unwrap()
    };
    let rows = audited().await;
    let (status, response) = patch_consensus(&app, &headers, job, json!({ "consensus_pct": 100 })).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(audited().await, rows);

    // Completed with a rack analysed once. The others are inactive while it
    // is claimed from, so the claim is its.
    sqlx::query("UPDATE jobs SET status = 'inactive', allocation = 0 WHERE id <> $1")
        .bind(job)
        .execute(&db.pool)
        .await
        .unwrap();
    let (a, uuid) = first_claim(&app).await;
    let racks: Vec<String> =
        a["task_request"]["racks"].as_array().unwrap().iter().map(|r| r.as_str().unwrap().into()).collect();
    let result = json!({ "racks": racks.iter().map(|rack| json!({
        "rack": rack, "num_moves": 1, "moves": [{ "move": "8G WUZ", "score": 30, "equity": 32.5 }],
    })).collect::<Vec<_>>() });
    let (status, _) = submit_as(&app, &uuid, a["claim_token"].as_str().unwrap(), result).await;
    assert_eq!(status, StatusCode::OK);
    sqlx::query("UPDATE jobs SET status = 'completed', allocation = 0 WHERE id = $1")
        .bind(job)
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, response) =
        patch_consensus(&app, &headers, job, json!({ "min_results_per_rack": 2, "max_results_per_rack": 2 })).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["reopened"], json!(true));
    assert_eq!((&response["job"]["status"], &response["job"]["allocation"]), (&json!("inactive"), &json!(0)));
}

/// Every rack of an opening-rack assignment, as the worker would report them,
/// each with `play` as its best move.
fn rack_result(assignment: &serde_json::Value, play: &str) -> serde_json::Value {
    let racks = assignment["task_request"]["racks"].as_array().unwrap();
    json!({ "racks": racks.iter().map(|rack| json!({
        "rack": rack, "num_moves": 1,
        "moves": [{ "move": play, "score": 30, "equity": 32.5 }],
    })).collect::<Vec<_>>() })
}

fn assigned_racks(assignment: &serde_json::Value) -> Vec<String> {
    assignment["task_request"]["racks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap().to_string())
        .collect()
}

/// I-OR-EDIT-3: a finish check that read an opening-rack job settled does
/// not complete it once a consensus edit has unsettled its racks. The edit
/// leaves the job active and lowers `racks_settled`, which neither purge
/// witness sees: a check whose update waited on the edit's row lock completed
/// the job with the racks the edit had just unsettled, never to be reissued.
/// Driven as `I-STATS-9d` drives the purge: the completion is called with
/// what the check observed before the edit.
#[tokio::test]
async fn a_finish_check_overtaken_by_a_consensus_edit_does_not_complete_the_job() {
    let db = TestDb::new().await;
    let admin = db.user("admin", true).await;
    let job = consensus_job(&db, admin, 100.0, 1, 1).await;
    let state = db.state().await;
    let headers = admin_headers(&state.cfg, admin);
    let app = birdtest::app(state);

    let (a, uuid_a) = first_claim(&app).await;
    let (b, uuid_b) = first_claim(&app).await;
    for (assignment, uuid) in [(a, uuid_a), (b, uuid_b)] {
        let token = assignment["claim_token"].as_str().unwrap().to_string();
        let (status, body) = submit_as(&app, &uuid, &token, rack_result(&assignment, "8G WUZ")).await;
        assert_eq!((status, body), (StatusCode::OK, json!({ "accepted": true })));
    }
    // Every rack settled, and a check has read it so -- but not yet written
    // the completion the last submission's own check would have.
    sqlx::query("UPDATE jobs SET status = 'active', allocation = 50 WHERE id = $1")
        .bind(job)
        .execute(&db.pool)
        .await
        .unwrap();
    let observed: i64 = sqlx::query_scalar("SELECT claims_issued FROM jobs WHERE id = $1")
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap();

    // The edit commits first, unsettling every rack.
    let (status, body) =
        patch_consensus(&app, &headers, job, json!({ "min_results_per_rack": 2, "max_results_per_rack": 2 })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!((body["unsettled_racks"].clone(), body["job"]["status"].clone()), (json!(4), json!("active")));

    let finish = birdtest::jobs::Finish::RacksAnalysed;
    let completed =
        birdtest::jobs::complete_unless_purged(&db.pool, job, observed, finish, || false).await.unwrap();
    assert!(!completed, "the racks the edit unsettled are still to analyse");
    let status: String = sqlx::query_scalar("SELECT status::text FROM jobs WHERE id = $1")
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(status, "active");
    // And they are reissued.
    let (again, _) = first_claim(&app).await;
    assert_eq!(again["task_request"]["seed"], json!("4"));
    assert_eq!(assigned_racks(&again).len(), 2);
}

/// I-OR-EDIT-4: while a consensus edit of an active job holds its locks, a
/// claim skips the job without waiting, a submission for one of its claims
/// is answered `503` at once, and a second edit is a `409`. Without the hold
/// a purge takes, the claim waited out the dispatch lock's two seconds and
/// the submission its claim's five, each on a pool connection, for as long
/// as the edit restated the job's racks. Once the edit is done the
/// submission goes through.
#[tokio::test]
async fn a_consensus_edit_in_progress_costs_claims_and_submissions_no_wait() {
    let db = TestDb::new().await;
    let admin = db.user("admin", true).await;
    let job = consensus_job(&db, admin, 100.0, 2, 3).await;
    let state = db.state().await;
    let headers = admin_headers(&state.cfg, admin);
    let app = birdtest::app(state);

    let (open, uuid) = first_claim(&app).await;
    let token = open["claim_token"].as_str().unwrap().to_string();

    // The edit is held at the job's row, its hold and every lock before the
    // row taken.
    let mut row = db.pool.begin().await.unwrap();
    sqlx::query("SELECT 1 FROM jobs WHERE id = $1 FOR UPDATE")
        .bind(job)
        .execute(&mut *row)
        .await
        .unwrap();
    let edit = {
        let (app, headers) = (app.clone(), headers.clone());
        tokio::spawn(async move {
            patch_consensus(&app, &headers, job, json!({ "max_results_per_rack": 4 })).await
        })
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pg_stat_activity
             WHERE datname = current_database() AND wait_event_type = 'Lock'
               AND query LIKE 'SELECT * FROM jobs WHERE id = $1 FOR UPDATE%'",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        if waiting > 0 {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "the edit never reached the job's row");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let started = std::time::Instant::now();
    let (status, body) =
        send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "the job is skipped: {body}");
    assert!(started.elapsed() < std::time::Duration::from_millis(1500), "the claim waited {:?}", started.elapsed());

    let started = std::time::Instant::now();
    let (status, body) = submit_as(&app, &uuid, &token, rack_result(&open, "8G WUZ")).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert!(started.elapsed() < std::time::Duration::from_millis(1500), "the submission waited {:?}", started.elapsed());

    let (status, body) = patch_consensus(&app, &headers, job, json!({ "max_results_per_rack": 5 })).await;
    assert_eq!(status, StatusCode::CONFLICT, "a second edit meanwhile: {body}");

    row.rollback().await.unwrap();
    let (status, body) = edit.await.unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["config"]["max_results_per_rack"], json!(4));

    // The claim outlived the edit, and its result is taken now.
    let (status, body) = submit_as(&app, &uuid, &token, rack_result(&open, "8G WUZ")).await;
    assert_eq!((status, body), (StatusCode::OK, json!({ "accepted": true })));
}

/// I-OR-EDIT-5: a consensus edit that is refused, or that changes nothing,
/// is answered before it takes the hold or a lock, so it ends with no reclaim
/// grace: a lapsed claim of the job is reclaimed at once after it. The
/// request was checked only under the locks, so a games job's `400`, a typo
/// in the share or a double click's unchanged second edit each held the
/// job's claims off while it locked them, and then kept its lapsed claims
/// from being reclaimed for a heartbeat timeout.
#[tokio::test]
async fn a_refused_or_unchanged_consensus_edit_holds_nothing() {
    let db = TestDb::new().await;
    let admin = db.user("admin", true).await;
    let games = db.games_job(2).await;
    let state = db.state().await;
    let headers = admin_headers(&state.cfg, admin);
    let app = birdtest::app(state.clone());

    // A games job's claim, long lapsed.
    let (open, _) = first_claim(&app).await;
    assert_eq!(open["job_id"], json!(games.to_string()));
    sqlx::query(
        "UPDATE task_claims SET last_heartbeat_at = now() - interval '1 hour',
                                claimed_at = now() - interval '1 hour'
         WHERE claim_token = $1::uuid",
    )
    .bind(open["claim_token"].as_str().unwrap())
    .execute(&db.pool)
    .await
    .unwrap();
    let (status, body) = patch_consensus(&app, &headers, games, json!({ "consensus_pct": 80 })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(birdtest::scheduler::reclaim_lapsed(&state, &[games]).await.unwrap(), 1, "reclaimed at once");

    // An opening-rack job: a share out of range, and the settings it has.
    let job = consensus_job(&db, admin, 100.0, 2, 3).await;
    let (status, body) = patch_consensus(&app, &headers, job, json!({ "consensus_pct": 50 })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body) =
        patch_consensus(&app, &headers, job, json!({ "consensus_pct": 100, "max_results_per_rack": 3 })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["config"]["max_results_per_rack"], json!(3));
    assert_eq!(state.dispatch_holds.reclaimable(&[job]), vec![job], "no grace after either");
}

/// I-OR-REISSUE-1: a consensus job's reissue hands a single identity that
/// analysed every rack a full batch -- the fewest analyses first, none in
/// flight -- and still prefers, among the racks with the fewest analyses,
/// ones the claiming identity has not analysed. The preference used to walk
/// every unsettled rack until it found a batch the identity had not seen,
/// a probe of its analyses per rack, under the job's dispatch lock: for an
/// identity that had analysed the whole first pass, every rack of the job on
/// every reissue claim. It now looks at a few batches' worth -- and only
/// those: with every rack it has not seen pushed past the window, it is
/// handed racks it has seen, where the walk found the unseen ones beyond.
#[tokio::test]
async fn a_reissue_looks_at_a_window_of_racks_not_every_one() {
    let db = TestDb::new().await;
    let admin = db.user("admin", true).await;
    let pool = &db.pool;
    let mut state = db.state().await;
    // One identity makes forty requests for the first pass alone.
    state.limits.worker = std::sync::Arc::new(governor::RateLimiter::keyed(governor::Quota::per_second(
        std::num::NonZeroU32::new(10_000).unwrap(),
    )));
    let app = birdtest::app(state);
    // Forty racks, two to a task: a window of four batches is eight racks.
    let forty_racks = || async {
        let job = consensus_job(&db, admin, 100.0, 2, 3).await;
        sqlx::query("UPDATE job_opening_rack_config SET total_racks = 40 WHERE job_id = $1")
            .bind(job)
            .execute(pool)
            .await
            .unwrap();
        job
    };
    let analyse = |uuid: String| {
        let app = app.clone();
        async move {
            let (status, assignment) = claim_as(&app, &uuid).await;
            assert_eq!(status, StatusCode::OK, "{assignment}");
            let token = assignment["claim_token"].as_str().unwrap().to_string();
            let (status, body) = submit_as(&app, &uuid, &token, rack_result(&assignment, "8G WUZ")).await;
            assert_eq!((status, body), (StatusCode::OK, json!({ "accepted": true })));
            assigned_racks(&assignment)
        }
    };
    let sorted_racks = |job: Uuid| async move {
        sqlx::query_scalar::<_, String>("SELECT rack FROM opening_rack_progress WHERE job_id = $1 ORDER BY rack")
            .bind(job)
            .fetch_all(pool)
            .await
            .unwrap()
    };

    // One identity analyses the whole first pass.
    let job = forty_racks().await;
    let (first, a) = first_claim(&app).await;
    let token = first["claim_token"].as_str().unwrap().to_string();
    let (status, _) = submit_as(&app, &a, &token, rack_result(&first, "8G WUZ")).await;
    assert_eq!(status, StatusCode::OK);
    for _ in 1..20 {
        analyse(a.clone()).await;
    }
    let racks = sorted_racks(job).await;
    assert_eq!(racks.len(), 40);
    // It has seen every rack, and is handed a full batch all the same: the
    // fewest analyses first, then by rack.
    let (status, again) = claim_as(&app, &a).await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(again["task_request"]["seed"], json!("40"));
    assert_eq!(assigned_racks(&again), racks[0..2]);
    // Another identity has seen none of them: the next two, none in flight.
    let (other, _) = first_claim(&app).await;
    assert_eq!(assigned_racks(&other), racks[2..4]);
    sqlx::query("UPDATE jobs SET status = 'inactive', allocation = 0 WHERE id = $1").bind(job).execute(pool).await.unwrap();

    // Two identities split the first pass: within the window, each is handed
    // the other's racks first.
    let job = forty_racks().await;
    let (first, a) = first_claim(&app).await;
    let token = first["claim_token"].as_str().unwrap().to_string();
    let (status, _) = submit_as(&app, &a, &token, rack_result(&first, "8G WUZ")).await;
    assert_eq!(status, StatusCode::OK);
    let (first, b) = first_claim(&app).await;
    let token = first["claim_token"].as_str().unwrap().to_string();
    let (status, _) = submit_as(&app, &b, &token, rack_result(&first, "8G WUZ")).await;
    assert_eq!(status, StatusCode::OK);
    let mut by_b = assigned_racks(&first);
    for i in 2..20 {
        if i % 2 == 0 {
            analyse(a.clone()).await;
        } else {
            by_b.extend(analyse(b.clone()).await);
        }
    }
    let racks = sorted_racks(job).await;
    let window = &racks[0..8];
    let mut expected: Vec<String> = window.iter().filter(|r| by_b.contains(r)).cloned().collect();
    expected.extend(window.iter().filter(|r| !by_b.contains(r)).cloned());
    expected.truncate(2);
    assert!(window.iter().filter(|r| by_b.contains(r)).count() >= 1, "the window holds a rack of B's");
    let (status, again) = claim_as(&app, &a).await;
    assert_eq!(status, StatusCode::OK, "{again}");
    let mut got = assigned_racks(&again);
    got.sort();
    expected.sort();
    assert_eq!(got, expected, "A is handed B's racks from the window first");

    // Every rack B analysed is pushed past the window, as more analyses
    // would: the window is now racks A has seen, and A is handed the first
    // two of them. A walk past the window would have found B's.
    sqlx::query("UPDATE opening_rack_progress SET results = results + 1 WHERE job_id = $1 AND rack = ANY($2)")
        .bind(job)
        .bind(&by_b)
        .execute(pool)
        .await
        .unwrap();
    let own: Vec<String> = racks.iter().filter(|r| !by_b.contains(r)).cloned().collect();
    assert!(own.len() >= 8, "A's racks fill the window");
    let (status, again) = claim_as(&app, &a).await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(assigned_racks(&again), own[0..2], "A looks no further than the window");
}

/// I-OR-REISSUE-2: a reissue leaves out exactly the racks of the reissues
/// still open -- one claimed, and one given back and waiting to go out again
/// -- and not those of the reissues the job has completed, which it reads
/// past rather than through: the in-flight racks were read by visiting every
/// reissue the job had ever made, a cost that grew with its history, under
/// the dispatch lock (the timing, measured by hand: PLAN.md, "What these
/// reads cost").
#[tokio::test]
async fn a_reissue_leaves_out_the_open_reissues_and_only_those() {
    let db = TestDb::new().await;
    let admin = db.user("admin", true).await;
    let pool = &db.pool;
    let mut state = db.state().await;
    // One identity makes every request.
    state.limits.worker = std::sync::Arc::new(governor::RateLimiter::keyed(governor::Quota::per_second(
        std::num::NonZeroU32::new(10_000).unwrap(),
    )));
    let app = birdtest::app(state);
    // Eight racks, two to a task, three analyses each to settle.
    let job = consensus_job(&db, admin, 100.0, 3, 5).await;
    sqlx::query("UPDATE job_opening_rack_config SET total_racks = 8 WHERE job_id = $1")
        .bind(job)
        .execute(pool)
        .await
        .unwrap();
    let (first, a) = first_claim(&app).await;
    let mut open = first;
    let take = |assignment: serde_json::Value| {
        let app = app.clone();
        let a = a.clone();
        async move {
            let token = assignment["claim_token"].as_str().unwrap().to_string();
            let (status, body) = submit_as(&app, &a, &token, rack_result(&assignment, "8G WUZ")).await;
            assert_eq!((status, body), (StatusCode::OK, json!({ "accepted": true })));
        }
    };
    // The first pass, then two reissues completed.
    for _ in 0..6 {
        take(open).await;
        let (status, next) = claim_as(&app, &a).await;
        assert_eq!(status, StatusCode::OK, "{next}");
        open = next;
    }
    let by_results = || async {
        sqlx::query_scalar::<_, String>(
            "SELECT rack FROM opening_rack_progress WHERE job_id = $1 ORDER BY results, rack",
        )
        .bind(job)
        .fetch_all(pool)
        .await
        .unwrap()
    };
    // One reissue claimed (`open`), and one declined back to `available`.
    let (status, declined) = claim_as(&app, &a).await;
    assert_eq!(status, StatusCode::OK, "{declined}");
    assert_eq!(decline_as(&app, &a, &declined["claim_token"], "task_failed").await, StatusCode::NO_CONTENT);
    let reissues: Vec<(String, i64)> = sqlx::query_as(
        "SELECT state::text, COUNT(*) FROM tasks WHERE job_id = $1 AND seed >= 8
         GROUP BY state ORDER BY state",
    )
    .bind(job)
    .fetch_all(pool)
    .await
    .unwrap();
    assert_eq!(
        reissues,
        vec![("available".into(), 1), ("claimed".into(), 1), ("completed".into(), 2)]
    );
    let racks = by_results().await;
    let mut in_flight = assigned_racks(&open);
    in_flight.extend(assigned_racks(&declined));
    in_flight.sort();
    let mut fewest = racks[0..4].to_vec();
    fewest.sort();
    assert_eq!(in_flight, fewest, "the open reissues hold the racks with the fewest analyses");

    // A skips the task it declined, so its claim is a new reissue: of the
    // completed reissues' racks, not the open ones'.
    let (status, again) = claim_as(&app, &a).await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(again["task_request"]["seed"], json!("16"));
    assert_eq!(assigned_racks(&again), racks[4..6]);
}

/// Bug: any error claiming from one job failed the whole claim, so a single
/// job that could not dispatch -- here, one whose config row is missing --
/// answered every worker with a 500 for as long as it led the candidate list.
#[tokio::test]
async fn a_job_that_cannot_dispatch_does_not_block_the_others() {
    let db = TestDb::new().await;
    let admin = db.user("admin", true).await;
    // Created first, so it wins the deficit tie-break and is tried first.
    let broken = db.bare_job("games", admin).await;
    let healthy = db.games_job(2).await;
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
    let job = db.games_job(2).await;
    sqlx::query(
        "UPDATE jobs SET min_magpie_major = 2, min_magpie_minor = 0, min_magpie_patch = 0
         WHERE id = $1",
    )
    .bind(job)
    .execute(&db.pool)
    .await
    .unwrap();
    let admin = db.user("admin", true).await;
    let finished = db.bare_job("games", admin).await;
    sqlx::query("UPDATE jobs SET status = 'completed', allocation = 0 WHERE id = $1")
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
    let job = db.games_job(2).await;
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
    db.games_job(2).await;

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
    db.games_job(2).await;

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
    let job = db.games_job(2).await;

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
    let job = db.bare_job("opening_rack", admin).await;
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
/// and `scheduler::claim` gave up after three attempts (eight rounds now) -- so
/// past three-way contention a worker was told there was nothing to do. Claims for one job
/// already serialize on the `jobs` row (`claims_issued`), so taking the job's
/// dispatch lock before reading the cursor costs nothing that was not already
/// being paid and turns the lost race into a short wait.
#[tokio::test]
async fn concurrent_claims_tile_the_seed_space_instead_of_colliding() {
    let db = TestDb::new().await;
    let job = db.games_job(10).await;
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
    let job = db.bare_job("opening_rack", admin).await;
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
            "num_moves": 2,
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
    let job = db.bare_job("games", admin).await;
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
    for field in [
        "sort_strategy", "num_plies", "num_plays", "num_plies_recorded", "movegen_margin",
        "endgame_plies", "peg_max_bag",
    ] {
        assert!(!request["player1"][field].is_null(), "player1 {field}: {request}");
    }
    // A player that solves nothing states its PEG settings as null, which is
    // how MAGPIE tells "not used" from "left to this build".
    assert_eq!(request["player1"]["endgame_plies"], json!(0), "{request}");
    for field in ["peg_stage_top_k", "peg_scenario_stride", "peg_opp_model", "peg_nested"] {
        let value = request["player1"].get(field);
        assert!(value.is_some_and(|v| v.is_null()), "player1 {field} sent as null: {request}");
    }
}

/// Contribution counters are running totals now, not counts over `task_claims`,
/// and the contributor lists read them. A counter that does not move, or moves
/// twice, is a wrong leaderboard that nothing else contradicts.
#[tokio::test]
async fn contributions_are_counted_as_they_arrive() {
    let db = TestDb::new().await;
    let job = db.games_job(2).await;
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

/// A contributor is credited with what each of its claims did: the time the
/// claim was held, claim to submission, and the move generations it reported.
/// Two workers, a task each, are each credited their own; the job counts both
/// tasks' games.
#[tokio::test]
async fn each_accepted_claim_credits_its_time_and_movegens_to_its_contributor() {
    let db = TestDb::new().await;
    let job = db.games_job(2).await;
    let app = birdtest::app(db.state().await);

    let counters = |uuid: String| {
        let pool = db.pool.clone();
        async move {
            sqlx::query_as::<_, (i64, i64, i64)>(
                "SELECT tasks_completed, compute_ms, movegens
                 FROM anonymous_workers WHERE uuid = $1::uuid",
            )
            .bind(uuid)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };

    let (first, a) = first_claim(&app).await;
    let (second, b) = first_claim(&app).await;
    assert_ne!(first["task_request"]["seed"], second["task_request"]["seed"], "a task each");
    assert_eq!(counters(a.clone()).await, (0, 0, 0), "a claim is not a contribution");
    // The first claim was held a minute and a half; the second not at all.
    sqlx::query(
        "UPDATE task_claims SET claimed_at = now() - interval '90 seconds' WHERE claim_token = $1::uuid",
    )
    .bind(first["claim_token"].as_str().unwrap())
    .execute(&db.pool)
    .await
    .unwrap();

    for (assignment, uuid, movegens) in [(&first, &a, 70_000), (&second, &b, 300)] {
        let token = assignment["claim_token"].as_str().unwrap();
        let (_, body) =
            submit_with_movegens(&app, uuid, token, games_result(2, 1), json!(movegens)).await;
        assert_eq!(body["accepted"], true, "{body}");
    }

    let (tasks, compute_ms, movegens) = counters(a.clone()).await;
    assert_eq!((tasks, movegens), (1, 70_000));
    assert!((90_000..100_000).contains(&compute_ms), "held 90 s: {compute_ms} ms");
    let (tasks, compute_ms, movegens) = counters(b.clone()).await;
    assert_eq!((tasks, movegens), (1, 300), "the second worker is credited its own movegens");
    assert!(compute_ms < 10_000, "{compute_ms} ms");
    let games_completed: i64 = sqlx::query_scalar("SELECT games_completed FROM jobs WHERE id = $1")
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(games_completed, 4, "the job counts both tasks' games");

    // The claims keep what they did, and the list shows it, most work first.
    let per_claim: Vec<i64> =
        sqlx::query_scalar("SELECT movegens FROM task_claims WHERE job_id = $1 ORDER BY claimed_at")
            .bind(job)
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(per_claim, [70_000, 300]);
    let (_, body) = send(&app, get_request("/api/workers", &[])).await;
    assert_eq!(body["items"][0]["anon_id"], birdtest::auth::public_anon_id(a.parse().unwrap()));
    assert!(body["items"][0]["compute_seconds"].as_f64().unwrap() >= 90.0, "{body}");
    assert_eq!(body["items"][0]["movegens"], 70_000);
    assert_eq!(body["items"][1]["movegens"], 300);
    assert!(body["items"][0].get("games_played").is_none(), "{body}");
}

/// A submission must say how many move generations it did, as a whole number
/// no machine could exceed for the time the claim was held. Refused, it stores
/// nothing: the claim stays open, the task unfinished, the contributor
/// uncredited -- and a corrected submission is still accepted.
#[tokio::test]
async fn a_result_without_plausible_movegens_is_refused_and_stores_nothing() {
    let db = TestDb::new().await;
    let job = db.games_job(2).await;
    let app = birdtest::app(db.state().await);
    let (assignment, uuid) = first_claim(&app).await;
    let token = assignment["claim_token"].as_str().unwrap();

    // Left out, negative, fractional, and more per millisecond held than any
    // machine generates (the claim was held well under a second). Four, and
    // the accepted one: a worker's burst of work-in-hand requests is five.
    for movegens in [
        serde_json::Value::Null,
        json!(-1),
        json!(1.5),
        json!(10_000_000_000_000_i64),
    ] {
        let (status, body) =
            submit_with_movegens(&app, &uuid, token, games_result(2, 1), movegens.clone()).await;
        assert!(status.is_client_error(), "{movegens}: {status} {body}");
        assert!(body.to_string().contains("movegens"), "refused for its movegens: {movegens}: {body}");
        let (claim_state, task_state, completed): (String, String, i64) = sqlx::query_as(
            "SELECT c.state::text, t.state::text,
                    (SELECT tasks_completed FROM anonymous_workers WHERE uuid = $2::uuid)
             FROM task_claims c JOIN tasks t ON t.id = c.task_id WHERE c.claim_token = $1::uuid",
        )
        .bind(token)
        .bind(&uuid)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!((claim_state.as_str(), task_state.as_str(), completed), ("claimed", "claimed", 0), "{movegens}");
    }
    let games: i64 = sqlx::query_scalar("SELECT games_completed FROM jobs WHERE id = $1")
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(games, 0, "nothing refused reached the job");

    let (status, body) = submit_with_movegens(&app, &uuid, token, games_result(2, 1), json!(0)).await;
    assert_eq!((status, &body["accepted"]), (StatusCode::OK, &json!(true)), "{body}");
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
    let job = db.bare_job("opening_rack", admin).await;
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
                "num_moves": 1,
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
    db.games_job(2).await;
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
    let job = db.games_job(2).await;
    let app = birdtest::app(db.state().await);

    let mut completer = db.pool.begin().await.unwrap();
    sqlx::query("UPDATE jobs SET status = 'completed', allocation = 0 WHERE id = $1")
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
async fn games_jobs_hand_out_nothing_past_their_cap() {
    let db = TestDb::new().await;
    let games = db.games_job(2).await;
    // No test: its floor of a million would be past the cap.
    sqlx::query(
        "UPDATE job_game_config SET max_games = 3, test_enabled = FALSE, min_games = 0
         WHERE job_id = $1",
    )
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
    sqlx::query("UPDATE jobs SET status = 'completed', allocation = 0 WHERE id = $1")
        .bind(games)
        .execute(&db.pool)
        .await
        .unwrap();
    let admin = db.user("pairs-admin", true).await;
    let p1 = db.static_player("pairs-p1", admin).await;
    let p2 = db.static_player("pairs-p2", admin).await;
    let pairs = db.bare_job("game_pairs", admin).await;
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
    let job = db.bare_job("game_pairs", admin).await;
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
              use_wit, movegen_margin, created_by)
         VALUES ($1, 'best', 'equity', $2, $3, 0, 100, 2, 10, true, $4, false, 5, $5)
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
    let job = db.bare_job("games", admin).await;
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
/// early would carry no hash for a file its player asks for, which MAGPIE
/// refuses (`derived_mismatch`), setting the job aside for the run on every
/// worker that claims it.
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

/// I-DERIVED-10: a job whose files were built under another builder -- a
/// deployment whose MAGPIE bumped `wmp-N` -- has them queued under this
/// binary's builder by the next claim that considers it. Only creating or
/// activating a job queued anything, so after such a deployment every job
/// needing a wordmap or a table answered `204` for good, with nothing queued
/// to show why (thirty-first audit).
#[tokio::test]
async fn a_file_built_under_another_builder_is_queued_under_this_one_by_a_claim() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let kwg = db.input_data("kwg", "NWL23").await;
    let klv = db.input_data("klv", "NWL23").await;
    let p1 = deriving_player(&db, "bump-p1", kwg, klv, false).await;
    let p2 = deriving_player(&db, "bump-p2", kwg, klv, false).await;
    let job = job_between(&db, p1, p2).await;
    assert_eq!(db.derived_ready(job).await, 1);
    // What the previous deployment built: the same file under its builder.
    sqlx::query("UPDATE derived_data SET builder = 'wmp-0'").execute(&db.pool).await.unwrap();
    let worker = registered_worker(&db).await;

    let (status, body) = claim_as(&app, &worker).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "nothing is built under wmp-1: {body}");
    let queued: Vec<(String, String)> =
        sqlx::query_as("SELECT builder, state FROM derived_data ORDER BY builder")
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(
        queued,
        vec![("wmp-0".into(), "built".into()), ("wmp-1".into(), "pending".into())],
        "the claim queued the file under this binary's builder"
    );
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
    let job = db.games_job(2).await;
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
    let (status, body) = send(&app, allocate(job, 50, &borrowed)).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // The purged job, activated again, starts its space over, and every claim
    // after it carries exactly what the first did.
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

/// There is no priority: an admin parks a job at 0%, which makes it inactive
/// and offered to nobody. Every claim goes to the active job above 0% that is
/// furthest behind its share.
#[tokio::test]
async fn a_job_at_zero_allocation_is_offered_to_nobody() {
    let db = TestDb::new().await;
    let parked = db.games_job(2).await;
    let running = db.games_job(2).await;
    sqlx::query("UPDATE jobs SET status = 'inactive', allocation = 0 WHERE id = $1")
        .bind(parked)
        .execute(&db.pool)
        .await
        .unwrap();
    let app = birdtest::app(db.state().await);

    for _ in 0..3 {
        let (assignment, _) = first_claim(&app).await;
        assert_eq!(assignment["job_id"], running.to_string(), "only the job above 0% is offered");
    }

    // With the running job parked too, nothing is on offer, and nothing
    // rules the worker out either: the answer is "nothing right now" rather
    // than a shutdown.
    sqlx::query("UPDATE jobs SET status = 'inactive', allocation = 0 WHERE id = $1")
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
    let job = db.bare_job("opening_rack", admin).await;
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
/// the admin raises a job it could have run. A job at 0% is inactive now
/// (`jobs_allocation_is_status`), so the two cannot differ again; this holds
/// the line.
#[tokio::test]
async fn a_parked_job_shuts_nobody_down() {
    let db = TestDb::new().await;
    let too_new = db.games_job(2).await;
    sqlx::query(
        "UPDATE jobs SET status = 'inactive', allocation = 0, min_magpie_major = 2, min_magpie_minor = 0,
                         min_magpie_patch = 0
         WHERE id = $1",
    )
    .bind(too_new)
    .execute(&db.pool)
    .await
    .unwrap();
    let unsupported = db.games_job(2).await;
    sqlx::query("UPDATE jobs SET status = 'inactive', allocation = 0 WHERE id = $1")
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
    sqlx::query("UPDATE jobs SET status = 'active', allocation = 50 WHERE id = ANY($1)")
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
    // A simmer, whose analyses carry the win percentages and plies below.
    let player = db.sim_player("simmer", admin).await;
    let job = db.bare_job("opening_rack", admin).await;
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
    let job = db.games_job(2).await;

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
                json!({ "claim_token": claim["claim_token"], "movegens": 1000, "result": games_result(2, 2) }),
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

    // A contributor with claims in another job and none in this one: an empty
    // page, decided from their claims in this job before reading its records.
    // (Read the other way, the planner judged their share of this job from
    // their share of all claims, and walked all of it to return nothing.)
    let other = db.games_job(2).await;
    let (status, body) =
        send(&app, get_request(&format!("/api/jobs/{other}/results?worker=keyed"), &[])).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["items"].as_array().unwrap().len(), 0, "{body}");
    assert!(body["next_cursor"].is_null(), "{body}");
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
    let job = db.games_job(2).await;
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
        { "game_index": 0, "turn_number": 0, "played_move": "8D PLAYED", "played_move_score": 10, "analysis": "static", "rack": "AEINRST", "position": "cgp-0",
          "num_moves": 40, "moves": [{ "move": "8D RETAINS", "score": 74, "equity": 81.2 }] },
        { "game_index": 1, "turn_number": 3, "played_move": "8D PLAYED", "played_move_score": 10, "analysis": "static", "rack": "AEINRSU", "position": "cgp-1",
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
    let racks = db.bare_job("opening_rack", admin).await;
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

async fn claim_fresh(app: &axum::Router) -> (StatusCode, serde_json::Value) {
    send(app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await
}

async fn decline_as(app: &axum::Router, uuid: &str, token: &serde_json::Value, reason: &str) -> StatusCode {
    send(
        app,
        post_json(
            "/api/worker/decline",
            &[("x-worker-uuid", uuid)],
            json!({ "claim_token": token, "reason": reason }),
        ),
    )
    .await
    .0
}

/// A-WORKER-19: a task its worker declined is not handed back to that worker
/// for an hour. It went back to `available` and, the oldest, was every claim
/// of its job, ahead of any new work -- the worker that had just failed it
/// included -- and MAGPIE stops after five failures in a row, so one task
/// that fails everywhere stopped every contributor claiming from the job.
/// Another worker is still offered it.
#[tokio::test]
async fn a_declined_task_is_not_handed_back_to_the_worker_that_declined_it() {
    let db = TestDb::new().await;
    db.games_job(1).await;
    let app = birdtest::app(db.state().await);

    let (status, first) = claim_fresh(&app).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let a = first["worker_uuid"].as_str().unwrap().to_string();
    let failing = first["task_request"]["seed"].clone();
    let mut token = first["claim_token"].clone();
    let mut seeds = vec![failing.clone()];
    for _ in 0..3 {
        // The work-in-hand bucket refills one a second.
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        assert_eq!(decline_as(&app, &a, &token, "task_failed").await, StatusCode::NO_CONTENT);
        let (status, next) = claim_as(&app, &a).await;
        assert_eq!(status, StatusCode::OK, "{next}");
        seeds.push(next["task_request"]["seed"].clone());
        token = next["claim_token"].clone();
    }
    let distinct: std::collections::HashSet<String> = seeds.iter().map(|s| s.to_string()).collect();
    assert_eq!(distinct.len(), seeds.len(), "each claim a new task: {seeds:?}");

    let (_, other) = claim_fresh(&app).await;
    assert_eq!(other["task_request"]["seed"], failing, "another worker is offered it");
}

/// A-WORKER-20: a result carrying captured positions for a job that does not
/// capture them is refused. They were stored, and the job's export then held
/// a positions file nobody asked for.
#[tokio::test]
async fn positions_from_a_job_that_does_not_capture_them_are_refused() {
    let db = TestDb::new().await;
    let job = db.games_job(2).await;
    let app = birdtest::app(db.state().await);
    let (status, assignment) = claim_fresh(&app).await;
    assert_eq!(status, StatusCode::OK, "{assignment}");
    assert_eq!(assignment["task_request"]["capture_positions"], json!(false));
    let uuid = assignment["worker_uuid"].as_str().unwrap();
    let mut result = games_result(2, 1);
    result["positions"] = json!([{
        "game_index": 0, "turn_number": 0, "played_move": "8D PLAYED", "played_move_score": 10, "analysis": "static", "rack": "AEINRST",
        "position": "15/15/15/15/15/15/15/15/15/15/15/15/15/15/15 AEINRST/ 0/0 0",
        "num_moves": 1, "moves": [{ "move": "8D RETAINS", "score": 70, "equity": 70.0 }]
    }]);
    let (status, body) = send(
        &app,
        post_json(
            "/api/worker/result",
            &[("x-worker-uuid", uuid)],
            json!({ "claim_token": assignment["claim_token"], "movegens": 1000, "result": result }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM position_analysis_records WHERE job_id = $1")
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(stored, 0);
}


fn captured(game: i32, turn: i32) -> serde_json::Value {
    json!({ "game_index": game, "turn_number": turn, "played_move": "8D PLAYED", "played_move_score": 10, "analysis": "static", "rack": "AEINRST",
            "position": "15/15/15/15/15/15/15/15/15/15/15/15/15/15/15 AEINRST/ 0/0 0",
            "num_moves": 1, "moves": [{ "move": "8D RETAINS", "score": 70, "equity": 70.0 }] })
}

/// A-WORKER-21: what a result for a capturing job must carry, and what no
/// result may. A capturing job's result with no positions, or none from one of
/// its games, was accepted and its task completed -- a hole in the corpus for
/// good; a NUL in a string failed at the insert as a `500`, which MAGPIE
/// retries until it gives up; and a 100 KB previous play, a score of
/// `i32::MIN` and a 500 KB bracketed "tile" were stored (the audit's pass 21).
/// Each is now a `400`, and nothing is stored.
#[tokio::test]
async fn a_capturing_jobs_result_is_complete_and_no_result_holds_what_cannot_be_stored() {
    let db = TestDb::new().await;
    let job = db.games_job(2).await;
    sqlx::query("UPDATE job_game_config SET capture_positions = true WHERE job_id = $1")
        .bind(job)
        .execute(&db.pool)
        .await
        .unwrap();
    let app = birdtest::app(db.state().await);

    let with = |positions: Vec<serde_json::Value>| {
        let mut result = games_result(2, 1);
        result["positions"] = json!(positions);
        result
    };
    let mut nul = captured(1, 0);
    nul["moves"][0]["move"] = json!("8D RET\u{0000}AINS");
    let mut long_previous = captured(1, 1);
    long_previous["previous_move"] = json!("x".repeat(100_000));
    let mut low_score = captured(1, 1);
    low_score["previous_move"] = json!("8D DOG");
    low_score["previous_move_score"] = json!(i32::MIN);
    let mut long_tile = captured(1, 0);
    long_tile["rack"] = json!(format!("[{}]", "Q".repeat(500_000)));
    let mut long_position = captured(1, 0);
    long_position["position"] = json!("1".repeat(5_000));
    let mut long_play = captured(1, 0);
    long_play["moves"][0]["move"] = json!("8D ".to_string() + &"Q".repeat(300));
    let cases = [
        ("no positions", games_result(2, 1), "has none from game 0"),
        ("none from game 1", with(vec![captured(0, 0)]), "has none from game 1"),
        ("a NUL in a move", with(vec![captured(0, 0), nul]), "NUL"),
        ("a 100 KB previous play", with(vec![captured(0, 0), captured(1, 0), long_previous]), "is not a play"),
        ("a previous play scoring i32::MIN", with(vec![captured(0, 0), captured(1, 0), low_score]), "which no play can score"),
        ("a 500 KB tile", with(vec![captured(0, 0), long_tile]), "bracketed tile"),
        ("a 5,000-character position", with(vec![captured(0, 0), long_position]), "is not a position"),
        ("a 300-character play", with(vec![captured(0, 0), long_play]), "is not a play"),
    ];
    // A claim each: a worker's requests are rate limited (A-WORKER-14).
    for (what, result, says) in cases {
        let (assignment, uuid) = first_claim(&app).await;
        let token = assignment["claim_token"].as_str().unwrap();
        let (status, body) = submit_as(&app, &uuid, token, result).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{what}: {body}");
        assert!(body["message"].as_str().is_some_and(|m| m.contains(says)), "{what}: expected {says:?} in {body}");
    }
    let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM position_analysis_records WHERE job_id = $1")
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(stored, 0);

    // The complete result is accepted.
    let (assignment, uuid) = first_claim(&app).await;
    let token = assignment["claim_token"].as_str().unwrap();
    let (status, body) = submit_as(&app, &uuid, token, with(vec![captured(0, 0), captured(1, 0)])).await;
    assert_eq!((status, &body), (StatusCode::OK, &json!({ "accepted": true })), "{body}");
}

/// A-WORKER-21 (declines): a decline naming a missing file with a NUL in it
/// failed at the insert as a `500` and left the claim open (the audit's pass
/// 21). It is a `400`, and the claim can still be declined properly.
#[tokio::test]
async fn a_decline_holding_a_nul_is_refused_and_the_claim_stays_declinable() {
    let db = TestDb::new().await;
    db.games_job(2).await;
    let app = birdtest::app(db.state().await);
    let (assignment, uuid) = first_claim(&app).await;
    let decline = |name: &str| {
        json!({ "claim_token": assignment["claim_token"], "reason": "missing_data",
                "missing": [{ "role": "kwg", "name": name, "expected": "abc" }] })
    };
    let (status, body) = send(&app, post_json("/api/worker/decline", &[("x-worker-uuid", &uuid)], decline("NWL\u{0000}23"))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body) = send(&app, post_json("/api/worker/decline", &[("x-worker-uuid", &uuid)], decline("NWL23"))).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
}
