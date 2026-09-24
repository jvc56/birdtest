//! Tier 3: the worker protocol's routes, through the real router, against a
//! real database. Each test names the TESTING.md `A-WORKER` guarantee it
//! proves; the races and the scheduler's decisions live in `worker_api.rs`.

mod common;

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use common::*;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

// --- helpers ----------------------------------------------------------------

/// An identity the server issued, written directly: several tests need many
/// workers, and minting each through a claim spends the per-IP bucket every
/// identity-less claim shares.
async fn registered_worker(db: &TestDb) -> String {
    let uuid = Uuid::new_v4();
    sqlx::query("INSERT INTO anonymous_workers (uuid) VALUES ($1)")
        .bind(uuid)
        .execute(&db.pool)
        .await
        .unwrap();
    uuid.to_string()
}

async fn claim(app: &axum::Router, headers: &[(&str, &str)], body: Value) -> (StatusCode, Value) {
    send(app, post_json("/api/worker/task", headers, body)).await
}

async fn claim_as(app: &axum::Router, uuid: &str) -> (StatusCode, Value) {
    claim(app, &[("x-worker-uuid", uuid)], claim_body("1.0.0", &[])).await
}

/// Claims with no identity, returning the assignment and the minted UUID.
async fn first_claim(app: &axum::Router) -> (Value, String) {
    let (status, body) = claim(app, &[], claim_body("1.0.0", &[])).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let uuid = body["worker_uuid"].as_str().expect("a minted worker_uuid").to_string();
    (body, uuid)
}

async fn decline_as(
    app: &axum::Router,
    uuid: &str,
    token: &Value,
    reason: &str,
    missing: Value,
) -> (StatusCode, Value) {
    send(
        app,
        post_json(
            "/api/worker/decline",
            &[("x-worker-uuid", uuid)],
            json!({ "claim_token": token, "reason": reason, "missing": missing }),
        ),
    )
    .await
}

async fn heartbeat_as(app: &axum::Router, uuid: &str, token: &Value) -> StatusCode {
    let (status, body) = send(
        app,
        post_json("/api/worker/heartbeat", &[("x-worker-uuid", uuid)], json!({ "claim_token": token })),
    )
    .await;
    assert_eq!(body, Value::Null, "a heartbeat answers with no body");
    status
}

async fn submit_as(app: &axum::Router, uuid: &str, token: &Value, result: Value) -> (StatusCode, Value) {
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

/// The status, headers and raw bytes: for the responses `send` would mangle
/// (binary bodies) or strip (a `Retry-After`).
async fn raw(app: &axum::Router, request: Request<Body>) -> (StatusCode, HeaderMap, Vec<u8>) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024 * 1024).await.unwrap();
    (status, headers, bytes.to_vec())
}

async fn count(db: &TestDb, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(&db.pool).await.unwrap()
}

fn message(body: &Value) -> &str {
    body["message"].as_str().unwrap_or_else(|| panic!("no message in {body}"))
}

fn edit(mut value: Value, change: impl FnOnce(&mut Value)) -> Value {
    change(&mut value);
    value
}

/// A leave-generation job over the test distribution, with its generation-1
/// universe seeded and a generation-0 artifact row, as `leave_gen.rs` builds
/// it: 100 games a task, so a task's rack occurrences are bounded by 100,000.
async fn leave_job(db: &TestDb, racks_per_task: i32) -> Uuid {
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
        "INSERT INTO leave_generation_artifacts
             (job_id, generation, artifact_key, sha256, builder)
         VALUES ($1, 0, 'leaves/test/generation-0.klv2', repeat('0', 64), 'klv-1')",
    )
    .bind(job)
    .execute(&db.pool)
    .await
    .unwrap();
    assert_eq!(db.derived_ready(job).await, 1, "a leave job needs one wordmap");
    let mut conn = db.pool.acquire().await.unwrap();
    let job_data = birdtest::jobs::load_job_data(&mut conn, job).await.unwrap();
    birdtest::jobs::leave_gen::seed_generation(&mut conn, job, 1, &job_data.letterdist)
        .await
        .unwrap();
    job
}

// --- A-WORKER-2, 3, 5, 6: what a claim is answered with ----------------------

/// A-WORKER-2: a claim whose `magpie_version` cannot be read is not assumed to
/// be current. Unparseable text is 0.0.0 (PLAN.md, "MAGPIE version
/// negotiation"), below the server's floor, so the worker is handed nothing and
/// told to update -- no claim, no task, no identity minted. A version that is
/// not a string at all is a malformed body and a `400` naming the field.
#[tokio::test]
async fn a_malformed_magpie_version_is_refused_rather_than_assumed() {
    let db = TestDb::new().await;
    db.games_job(1, 2).await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());

    for version in ["", "nonsense", "1", "v1.4.0", "1.x.0"] {
        let (status, body) = claim(&app, &[], claim_body(version, &[])).await;
        assert_eq!(status, StatusCode::OK, "{version:?}: {body}");
        let shutdown = &body["shutdown"];
        assert_eq!(shutdown["reason"], "magpie_too_old", "{version:?}: {body}");
        assert_eq!(shutdown["required_magpie_version"], "0.1.1", "{version:?}: {body}");
        assert_eq!(shutdown["download_url"], json!(state.cfg.magpie_download_url), "{body}");
        assert!(message(shutdown).contains("you are running 0.0.0"), "{version:?}: {body}");
        assert!(body.get("claim_token").is_none(), "{version:?} was handed a task: {body}");
    }

    let (status, body) = claim(&app, &[], json!({ "magpie_version": 1.4, "unsupported_jobs": [] })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(message(&body).contains("magpie_version"), "{body}");

    assert_eq!(count(&db, "SELECT COUNT(*) FROM task_claims").await, 0);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM tasks").await, 0);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM anonymous_workers").await, 0);

    // The same worker stating a version it really runs is served.
    let (status, body) = claim(&app, &[], claim_body("1.0.0", &[])).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["claim_token"].is_string(), "{body}");
}

/// A-WORKER-3: an `unsupported_jobs` list past 200 entries is truncated, not
/// rejected. The only active job listed at position 221 is dropped with the
/// overflow, so the worker is still handed it; listed at position 1 of the same
/// oversized list it is honoured.
#[tokio::test]
async fn an_oversized_unsupported_list_is_truncated_not_rejected() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 2).await;
    let app = birdtest::app(db.state().await);

    let mut listed: Vec<Uuid> = (0..250).map(|_| Uuid::new_v4()).collect();
    listed[220] = job;
    let (status, body) = claim(&app, &[], claim_body("1.0.0", &listed)).await;
    assert_eq!(status, StatusCode::OK, "an oversized list must not be refused: {body}");
    assert_eq!(body["job_id"], json!(job.to_string()), "the entry past the cap was not dropped: {body}");

    listed.swap(0, 220);
    let (status, body) = claim(&app, &[], claim_body("1.0.0", &listed)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["shutdown"]["reason"], "data_out_of_date", "entries within the cap still count: {body}");
}

/// A-WORKER-5: an `X-Worker-UUID` the server never issued is refused with a
/// `401` that says what to do instead, on the claim and on every other worker
/// endpoint, and nothing is written for it.
#[tokio::test]
async fn an_invented_worker_uuid_is_refused_with_the_fix() {
    let db = TestDb::new().await;
    db.games_job(1, 2).await;
    let app = birdtest::app(db.state().await);
    let invented = Uuid::new_v4().to_string();

    let (status, body) = claim_as(&app, &invented).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert!(message(&body).contains("unrecognized worker UUID"), "{body}");
    assert!(message(&body).contains("Omit the X-Worker-UUID header to be issued one"), "{body}");

    let (status, body) = send(
        &app,
        post_json("/api/worker/heartbeat", &[("x-worker-uuid", &invented)], json!({ "claim_token": Uuid::new_v4() })),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert!(message(&body).contains("unrecognized worker UUID"), "{body}");

    // Not a UUID at all is a different mistake, and says so.
    let (status, body) = claim_as(&app, "not-a-uuid").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(message(&body).contains("not a valid UUID"), "{body}");

    assert_eq!(count(&db, "SELECT COUNT(*) FROM anonymous_workers").await, 0);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM task_claims").await, 0);
}

/// A-WORKER-6: "nothing right now" is a bodiless `204` -- whether no job is on
/// offer at all or the jobs on offer have nothing to hand out -- and "you will
/// never be useful until something changes" is a `shutdown` body naming its
/// reason and remedy: `magpie_too_old` (below the server's floor, or below
/// every job's), `data_out_of_date` (every job on offer is one the worker
/// cannot run) and `both`. None of these hands anything out.
#[tokio::test]
async fn idle_and_each_shutdown_reason_are_distinct_answers() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let download = json!(state.cfg.magpie_download_url);

    // No job on offer.
    let (status, body) = claim(&app, &[], claim_body("1.0.0", &[])).await;
    assert_eq!((status, &body), (StatusCode::NO_CONTENT, &Value::Null));

    // A job on offer with nothing left to hand out: its one batch is out.
    let capped = db.games_job(1, 2).await;
    sqlx::query("UPDATE job_game_config SET max_games = 2 WHERE job_id = $1")
        .bind(capped)
        .execute(&db.pool)
        .await
        .unwrap();
    first_claim(&app).await;
    let (status, body) = claim(&app, &[], claim_body("1.0.0", &[])).await;
    assert_eq!((status, &body), (StatusCode::NO_CONTENT, &Value::Null), "idle is not a shutdown");
    sqlx::query("UPDATE jobs SET status = 'inactive' WHERE id = $1")
        .bind(capped)
        .execute(&db.pool)
        .await
        .unwrap();
    let claims_before = count(&db, "SELECT COUNT(*) FROM task_claims").await;

    let job = db.games_job(1, 2).await;

    // Below the server-wide floor: no job need be consulted.
    let (status, body) = claim(&app, &[], claim_body("0.0.9", &[])).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let shutdown = &body["shutdown"];
    assert_eq!(shutdown["reason"], "magpie_too_old", "{body}");
    assert_eq!(shutdown["required_magpie_version"], "0.1.1", "{body}");
    assert_eq!(shutdown["download_url"], download, "{body}");
    assert_eq!(shutdown["required_tarball_dates"], json!([]), "{body}");

    // Every job on offer is one this worker has found it cannot run.
    let (status, body) = claim(&app, &[], claim_body("1.0.0", &[job])).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let shutdown = &body["shutdown"];
    assert_eq!(shutdown["reason"], "data_out_of_date", "{body}");
    assert_eq!(shutdown["required_tarball_dates"], json!(["20251004"]), "{body}");
    assert_eq!(shutdown["required_magpie_version"], Value::Null, "{body}");
    assert_eq!(shutdown["download_url"], Value::Null, "updating MAGPIE fixes nothing here: {body}");
    assert!(message(shutdown).contains("input data you do not have"), "{body}");

    // And a second job this worker is too old for: both, led by the version.
    let newer = db.games_job(1, 2).await;
    sqlx::query(
        "UPDATE jobs SET min_magpie_major = 2, min_magpie_minor = 0, min_magpie_patch = 0
         WHERE id = $1",
    )
        .bind(newer)
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, body) = claim(&app, &[], claim_body("1.0.0", &[job])).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let shutdown = &body["shutdown"];
    assert_eq!(shutdown["reason"], "both", "{body}");
    assert_eq!(shutdown["required_magpie_version"], "2.0.0", "{body}");
    assert_eq!(shutdown["required_tarball_dates"], json!(["20251004"]), "{body}");
    assert_eq!(shutdown["download_url"], download, "{body}");

    // Below every job's own floor, with no data problem.
    sqlx::query("UPDATE jobs SET status = 'inactive' WHERE id = $1")
        .bind(job)
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, body) = claim(&app, &[], claim_body("1.0.0", &[])).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["shutdown"]["reason"], "magpie_too_old", "{body}");
    assert_eq!(body["shutdown"]["required_magpie_version"], "2.0.0", "{body}");

    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM task_claims").await,
        claims_before,
        "a shutdown hands nothing out"
    );
}

// --- A-WORKER-7, 8, 9: decline and heartbeat ---------------------------------

/// A-WORKER-7: an assignment names every file the task loads with the digest of
/// the exact `input_data` row the job pins, and a worker that cannot reproduce
/// one declines with `missing_data`. The decline is recorded file by file in
/// `worker_data_gaps` and releases the claim at once: the task goes straight to
/// the next worker instead of waiting out the heartbeat timeout.
#[tokio::test]
async fn a_claim_states_its_digests_and_a_missing_data_decline_releases_it_at_once() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 2).await;
    let app = birdtest::app(db.state().await);

    let (assignment, uuid) = first_claim(&app).await;
    let expected = &assignment["expected_data"];
    assert_eq!(expected["algorithm"], "sha256", "{assignment}");
    assert_eq!(expected["derived"], json!([]), "static players need no derived file: {assignment}");
    let files = expected["files"].as_array().expect("expected_data.files");
    let mut roles: Vec<&str> = files.iter().map(|f| f["role"].as_str().unwrap()).collect();
    roles.sort_unstable();
    // Two static players on two lexicons of their own, one bag, one board.
    assert_eq!(roles, ["klv", "klv", "kwg", "kwg", "layout", "letterdist"], "{assignment}");
    for file in files {
        let (sha256, bytes, date): (String, i64, String) = sqlx::query_as(
            "SELECT sha256, bytes, tarball_date FROM input_data WHERE path = $1 AND role = $2",
        )
        .bind(file["path"].as_str().unwrap())
        .bind(file["role"].as_str().unwrap())
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(file["sha256"], json!(sha256), "{file}");
        assert_eq!(file["bytes"], json!(bytes), "{file}");
        assert_eq!(file["tarball_date"], json!(date), "{file}");
    }

    let kwg = files.iter().find(|f| f["role"] == "kwg").unwrap();
    let klv = files.iter().find(|f| f["role"] == "klv").unwrap();
    let (status, body) = decline_as(
        &app,
        &uuid,
        &assignment["claim_token"],
        "missing_data",
        json!([
            { "role": "kwg", "name": kwg["name"], "expected": kwg["sha256"], "actual": null },
            { "role": "klv", "name": klv["name"], "expected": klv["sha256"], "actual": "f".repeat(64) },
        ]),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    let gaps: Vec<(Uuid, String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT g.job_id, g.role, g.name, g.expected, g.actual
         FROM worker_data_gaps g JOIN task_claims c ON c.id = g.claim_id
         WHERE c.claim_token = $1::uuid ORDER BY g.role DESC",
    )
    .bind(assignment["claim_token"].as_str().unwrap())
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        gaps,
        vec![
            (job, "kwg".into(), kwg["name"].as_str().unwrap().into(), kwg["sha256"].as_str().unwrap().into(), None),
            (
                job,
                "klv".into(),
                klv["name"].as_str().unwrap().into(),
                klv["sha256"].as_str().unwrap().into(),
                Some("f".repeat(64))
            ),
        ]
    );

    let (claim_state, active, task_state): (String, i32, String) = sqlx::query_as(
        "SELECT c.state::text, t.active_claim_count, t.state::text
         FROM task_claims c JOIN tasks t ON t.id = c.task_id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!((claim_state.as_str(), active, task_state.as_str()), ("declined", 0, "available"));

    let (next, _) = first_claim(&app).await;
    assert_eq!(
        next["task_request"]["seed"], assignment["task_request"]["seed"],
        "the declined task goes straight to the next worker"
    );
}

/// A-WORKER-8: a decline must give one of the five reasons the protocol knows.
/// Anything else is a `400` listing them, and leaves the claim held; each of
/// `missing_data`, `magpie_version`, `unknown_job_type`, `derived_mismatch` and
/// `task_failed` is accepted, releases the claim, and is recorded on the audit
/// row so a decline can be told from a vanished worker afterwards.
#[tokio::test]
async fn only_the_five_known_decline_reasons_are_accepted() {
    let db = TestDb::new().await;
    db.games_job(1, 2).await;
    let app = birdtest::app(db.state().await);

    let (assignment, uuid) = first_claim(&app).await;
    let (status, body) = decline_as(&app, &uuid, &assignment["claim_token"], "bored", json!([])).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    for reason in ["missing_data", "magpie_version", "unknown_job_type", "derived_mismatch", "task_failed"] {
        assert!(message(&body).contains(reason), "the refusal should list {reason}: {body}");
    }
    let held: String = sqlx::query_scalar("SELECT state::text FROM task_claims WHERE claim_token = $1::uuid")
        .bind(assignment["claim_token"].as_str().unwrap())
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(held, "claimed", "a refused decline released nothing");

    for reason in ["missing_data", "magpie_version", "unknown_job_type", "derived_mismatch", "task_failed"] {
        let (assignment, uuid) = first_claim(&app).await;
        let token = &assignment["claim_token"];
        let (status, body) = decline_as(&app, &uuid, token, reason, json!([])).await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{reason}: {body}");
        let (state, audited): (String, Option<String>) = sqlx::query_as(
            "SELECT c.state::text,
                    (SELECT a.reason FROM audit_log a
                     WHERE a.action = 'task.declined' AND a.target_id = c.id::text)
             FROM task_claims c WHERE c.claim_token = $1::uuid",
        )
        .bind(token.as_str().unwrap())
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!((state.as_str(), audited.as_deref()), ("declined", Some(reason)));
    }
}

/// A-WORKER-9: a heartbeat keeps its own claim alive -- `last_heartbeat_at`
/// moves, and a claim silent since long before the timeout survives the
/// reclamation that takes a claim nobody heard from.
///
/// A heartbeat for a token that is no longer live -- reclaimed, declined, or
/// never issued -- revives nothing. It is answered `204` all the same, which is
/// the design rather than a gap: PLAN.md's worker contract gives the heartbeat
/// exactly one answer, the client sends each one once and ignores failures, and
/// "it will find out when it submits" (`routes::worker::heartbeat`). So
/// "rejected" in TESTING.md means "has no effect", which is what is asserted.
#[tokio::test]
async fn a_heartbeat_extends_only_a_live_claim_of_the_caller() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 2).await;
    let app = birdtest::app(db.state().await);

    let (alive, alive_uuid) = first_claim(&app).await;
    let (silent, silent_uuid) = first_claim(&app).await;
    let (declined, declined_uuid) = first_claim(&app).await;
    let (status, _) = decline_as(&app, &declined_uuid, &declined["claim_token"], "task_failed", json!([])).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    sqlx::query(
        "UPDATE task_claims SET claimed_at = now() - interval '1 hour',
                                last_heartbeat_at = now() - interval '1 hour'",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let heard = |token: Value| {
        let pool = db.pool.clone();
        async move {
            sqlx::query_as::<_, (String, bool)>(
                "SELECT state::text, last_heartbeat_at > now() - interval '1 minute'
                 FROM task_claims WHERE claim_token = $1::uuid",
            )
            .bind(token.as_str().unwrap().to_string())
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };

    assert_eq!(heartbeat_as(&app, &alive_uuid, &alive["claim_token"]).await, StatusCode::NO_CONTENT);
    assert_eq!(heard(alive["claim_token"].clone()).await, ("claimed".into(), true));

    // Only the claim nobody heard from lapses.
    let reclaimed = birdtest::scheduler::reclaim_expired(&db.pool, job, 300.0).await.unwrap();
    assert_eq!(reclaimed, 1);
    assert_eq!(heard(alive["claim_token"].clone()).await, ("claimed".into(), true));

    // Late heartbeats for the lapsed and the declined claim bring neither back.
    for (uuid, token, state) in [
        (&silent_uuid, &silent["claim_token"], "abandoned"),
        (&declined_uuid, &declined["claim_token"], "declined"),
    ] {
        assert_eq!(heartbeat_as(&app, uuid, token).await, StatusCode::NO_CONTENT);
        assert_eq!(heard(token.clone()).await, (state.into(), false), "{state} claim was revived");
    }
    // Nor does one for a token that was never issued.
    assert_eq!(heartbeat_as(&app, &alive_uuid, &json!(Uuid::new_v4())).await, StatusCode::NO_CONTENT);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM task_claims WHERE state = 'claimed'").await, 1);
}

// --- A-WORKER-10, 12: results ------------------------------------------------

/// A-WORKER-10: an accepted result is published to the job's live stats
/// stream, and the payload already counts it.
#[tokio::test]
async fn an_accepted_result_is_published_to_the_jobs_live_stream() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 2).await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let mut watching = state.sse.subscribe(job);

    let (assignment, uuid) = first_claim(&app).await;
    let (status, body) = submit_as(&app, &uuid, &assignment["claim_token"], games_result(2, 1)).await;
    assert_eq!((status, body), (StatusCode::OK, json!({ "accepted": true })));

    let payload = tokio::time::timeout(std::time::Duration::from_secs(10), watching.recv())
        .await
        .expect("no stats event was published for an accepted result")
        .unwrap();
    let stats: Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(stats["job"]["id"], json!(job.to_string()), "{stats}");
    assert_eq!(stats["results_accepted"], json!(1), "{stats}");
    assert_eq!(stats["games"]["units_completed"], json!(2), "{stats}");
    assert_eq!(stats["games"]["wins"], json!(1), "{stats}");
}

/// A builder of one submission from the assignment it answers.
type Build = fn(&Value) -> Value;

/// Claims once per case as a fresh identity (so no case spends another's rate
/// limit), submits the case's result and requires a `400` whose message says
/// what was wrong. Nothing is stored by any of them; the valid result the
/// cases were derived from is then accepted, so each refusal was its mutation's.
async fn assert_each_refused(db: &TestDb, app: &axum::Router, valid: Build, cases: &[(&str, Build, &str)]) {
    for (name, build, says) in cases {
        let uuid = registered_worker(db).await;
        let (status, assignment) = claim_as(app, &uuid).await;
        assert_eq!(status, StatusCode::OK, "{name}: {assignment}");
        let (status, body) = submit_as(app, &uuid, &assignment["claim_token"], build(&assignment)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{name}: {body}");
        assert!(message(&body).contains(says), "{name}: expected {says:?} in {body}");
    }
    assert_eq!(
        count(db, "SELECT COUNT(*) FROM task_claims WHERE state = 'completed'").await,
        0,
        "a refused result was stored"
    );
    let uuid = registered_worker(db).await;
    let (_, assignment) = claim_as(app, &uuid).await;
    let (status, body) = submit_as(app, &uuid, &assignment["claim_token"], valid(&assignment)).await;
    assert_eq!((status, &body), (StatusCode::OK, &json!({ "accepted": true })), "the valid base: {body}");
}

fn games_with_position(_: &Value) -> Value {
    let mut result = games_result(2, 1);
    result["positions"] = json!([{
        "game_index": 1, "turn_number": 3, "rack": "AEINRST", "position": "cgp",
        "num_moves": 5,
        "moves": [{ "move": "8D RETAINS", "score": 74, "equity": 81.2,
                    "win_percentage": 61.5, "blended_utility": 0.6 }],
    }]);
    result
}

/// A-WORKER-12 (games): every plausibility rule a games submission can break
/// (`jobs::plausibility`, `game::validate_*`, the batch-size check) surfaces
/// over HTTP as a `400` naming the problem, never a `500`. The finiteness rules
/// are the one exception, unreachable here: JSON has no NaN or infinity, so a
/// body carrying one does not parse.
#[tokio::test]
async fn every_implausible_games_result_is_a_400_that_says_why() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 2).await;
    sqlx::query("UPDATE job_game_config SET capture_positions = true WHERE job_id = $1")
        .bind(job)
        .execute(&db.pool)
        .await
        .unwrap();
    let app = birdtest::app(db.state().await);

    let cases: &[(&str, Build, &str)] = &[
        ("counts that do not sum", |a| edit(games_with_position(a), |v| v["all_games"]["wins"] = json!(2)),
            "wins + losses + ties must equal games"),
        ("a negative count", |a| edit(games_with_position(a), |v| {
            v["all_games"]["wins"] = json!(3);
            v["all_games"]["losses"] = json!(-1);
        }), "all must be non-negative"),
        ("a negative standard deviation", |a| edit(games_with_position(a), |v| v["all_games"]["p1_score_sd"] = json!(-0.5)),
            "a standard deviation cannot be negative"),
        ("an impossible mean", |a| edit(games_with_position(a), |v| v["all_games"]["p2_score_mean"] = json!(5000.0)),
            "outside anything a word game can produce"),
        ("no games", |a| edit(games_with_position(a), |v| {
            v["all_games"]["games"] = json!(0);
            v["all_games"]["wins"] = json!(0);
            v["all_games"]["losses"] = json!(0);
            v["positions"] = json!([]);
        }), "must contain at least one game"),
        ("the wrong batch size", |a| edit(games_with_position(a), |v| {
            v["all_games"]["games"] = json!(4);
            v["all_games"]["wins"] = json!(2);
            v["all_games"]["losses"] = json!(2);
        }), "result reports 4 games but this task dispatched 2"),
        ("a position in no game", |a| edit(games_with_position(a), |v| v["positions"][0]["game_index"] = json!(2)),
            "names game 2 but the batch has 2"),
        ("an impossible turn", |a| edit(games_with_position(a), |v| v["positions"][0]["turn_number"] = json!(401)),
            "implausible turn number 401"),
        ("a position with no moves", |a| edit(games_with_position(a), |v| v["positions"][0]["moves"] = json!([])),
            "at least one ranked move"),
        ("an empty rack", |a| edit(games_with_position(a), |v| v["positions"][0]["rack"] = json!("")),
            "empty rack"),
        ("an overfull rack", |a| edit(games_with_position(a), |v| v["positions"][0]["rack"] = json!("AEINRSTU")),
            "more than a rack holds"),
        ("more moves than generated", |a| edit(games_with_position(a), |v| v["positions"][0]["num_moves"] = json!(0)),
            "claims only 0 were generated"),
        ("a negative score", |a| edit(games_with_position(a), |v| v["positions"][0]["moves"][0]["score"] = json!(-1)),
            "which no play can score"),
        ("a score no play reaches", |a| edit(games_with_position(a), |v| v["positions"][0]["moves"][0]["score"] = json!(2001)),
            "which no play can score"),
        ("an implausible equity", |a| edit(games_with_position(a), |v| v["positions"][0]["moves"][0]["equity"] = json!(-6000.0)),
            "implausible equity"),
        ("a win percentage over 100", |a| edit(games_with_position(a), |v| v["positions"][0]["moves"][0]["win_percentage"] = json!(100.5)),
            "is not a percentage"),
        ("a utility outside [0, 1]", |a| edit(games_with_position(a), |v| v["positions"][0]["moves"][0]["blended_utility"] = json!(1.5)),
            "outside [0, 1]"),
        ("not a games result at all", |a| edit(games_with_position(a), |v| {
            v.as_object_mut().unwrap().remove("all_games");
        }), "malformed task response"),
    ];
    assert_each_refused(&db, &app, games_with_position, cases).await;
}

fn pairs_result(_: &Value) -> Value {
    let mut result = games_result(2, 1);
    result["pentanomial"] = json!([0, 0, 1, 0, 0]);
    result["divergent_games"] = result["all_games"].clone();
    result
}

/// A-WORKER-12 (game pairs): the pentanomial and the game counts describe the
/// same games, and every way they can disagree is a `400` that says which.
#[tokio::test]
async fn every_implausible_game_pairs_result_is_a_400_that_says_why() {
    let db = TestDb::new().await;
    let admin = db.user("admin", true).await;
    let p1 = db.static_player("pairs-p1", admin).await;
    let p2 = db.static_player("pairs-p2", admin).await;
    let job = db.bare_job("game_pairs", 1, admin).await;
    sqlx::query(
        "INSERT INTO job_game_pair_config
             (job_id, player1_config_id, player2_config_id, pairs_per_batch, min_pairs, max_pairs)
         VALUES ($1, $2, $3, 1, 1000000, 1000000)",
    )
    .bind(job)
    .bind(p1)
    .bind(p2)
    .execute(&db.pool)
    .await
    .unwrap();
    let app = birdtest::app(db.state().await);

    let cases: &[(&str, Build, &str)] = &[
        ("an odd number of games", |a| edit(pairs_result(a), |v| {
            v["all_games"]["games"] = json!(3);
            v["all_games"]["wins"] = json!(2);
        }), "an even, non-zero number of games"),
        ("no pentanomial", |a| edit(pairs_result(a), |v| {
            v.as_object_mut().unwrap().remove("pentanomial");
        }), "must report a pentanomial"),
        ("a negative pentanomial count", |a| edit(pairs_result(a), |v| v["pentanomial"] = json!([-1, 0, 2, 0, 0])),
            "pentanomial counts must be non-negative"),
        ("pairs that are not half the games", |a| edit(pairs_result(a), |v| v["pentanomial"] = json!([0, 0, 2, 0, 0])),
            "exactly half the games played"),
        ("a pentanomial that scores differently", |a| edit(pairs_result(a), |v| v["pentanomial"] = json!([0, 0, 0, 1, 0])),
            "disagrees with the game counts"),
        ("an odd divergent subset", |a| edit(pairs_result(a), |v| {
            v["divergent_games"]["games"] = json!(1);
            v["divergent_games"]["losses"] = json!(0);
        }), "divergent_games must be even"),
        ("a divergent subset larger than the whole", |a| edit(pairs_result(a), |v| {
            v["divergent_games"]["games"] = json!(4);
            v["divergent_games"]["wins"] = json!(2);
            v["divergent_games"]["losses"] = json!(2);
        }), "no larger than the total games played"),
        ("an inconsistent divergent subset", |a| edit(pairs_result(a), |v| v["divergent_games"]["wins"] = json!(2)),
            "divergent_games: wins + losses + ties must equal games"),
        ("the wrong batch size", |a| edit(pairs_result(a), |v| {
            v["all_games"]["games"] = json!(4);
            v["all_games"]["wins"] = json!(2);
            v["all_games"]["losses"] = json!(2);
            v["pentanomial"] = json!([0, 0, 2, 0, 0]);
        }), "result reports 4 games but this task dispatched 2"),
    ];
    assert_each_refused(&db, &app, pairs_result, cases).await;
}

fn racks_analysed(assignment: &Value) -> Value {
    let racks = assignment["task_request"]["racks"].as_array().expect("an opening-rack assignment");
    json!({ "racks": racks.iter().map(|rack| json!({
        "rack": rack, "num_moves": 3,
        "moves": [{ "move": "8G WUZ", "score": 30, "equity": 32.5 }],
    })).collect::<Vec<_>>() })
}

/// A-WORKER-12 (opening racks): the analysis rules, as `400`s over HTTP. The
/// batch-against-task rules are pinned in `worker_api.rs`.
#[tokio::test]
async fn every_implausible_opening_rack_result_is_a_400_that_says_why() {
    let db = TestDb::new().await;
    let admin = db.user("admin", true).await;
    let player = db.static_player("solver", admin).await;
    let job = db.bare_job("opening_rack", 1, admin).await;
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

    let cases: &[(&str, Build, &str)] = &[
        ("no racks", |a| edit(racks_analysed(a), |v| v["racks"] = json!([])), "returned no racks"),
        ("a rack with no moves", |a| edit(racks_analysed(a), |v| v["racks"][0]["moves"] = json!([])),
            "was analyzed with no moves"),
        ("an overfull rack", |a| edit(racks_analysed(a), |v| v["racks"][1]["rack"] = json!("AEINRSTU")),
            "more than a rack holds"),
        ("more moves than ranked", |a| edit(racks_analysed(a), |v| v["racks"][0]["num_moves"] = json!(0)),
            "claims only 0 were generated"),
        ("a negative score", |a| edit(racks_analysed(a), |v| v["racks"][0]["moves"][0]["score"] = json!(-3)),
            "which no play can score"),
    ];
    assert_each_refused(&db, &app, racks_analysed, cases).await;
}

fn rack_occurrences(assignment: &Value) -> Value {
    let racks = assignment["task_request"]["forced_racks"].as_array().expect("a leave assignment");
    json!({ "racks": [
        { "rack": racks[0], "count": 40, "mean": 1.0 },
        { "rack": racks[1], "count": 35, "mean": -2.5 },
    ]})
}

/// A-WORKER-12 (leave generation): occurrences are summed on receipt, so every
/// rule that keeps a broken client from inflating or wedging a generation is a
/// `400` that says what it saw.
#[tokio::test]
async fn every_implausible_leave_result_is_a_400_that_says_why() {
    let db = TestDb::new().await;
    leave_job(&db, 2).await;
    let app = birdtest::app(db.state().await);

    let cases: &[(&str, Build, &str)] = &[
        ("no racks", |a| edit(rack_occurrences(a), |v| v["racks"] = json!([])), "carried no rack occurrences"),
        ("a partial rack", |a| edit(rack_occurrences(a), |v| v["racks"][0]["rack"] = json!("AB")),
            "leave generation reports full racks of 7"),
        ("an overfull rack", |a| edit(rack_occurrences(a), |v| v["racks"][0]["rack"] = json!("AABCDEFG")),
            "more than a rack holds"),
        ("a rack that never occurred", |a| edit(rack_occurrences(a), |v| v["racks"][1]["count"] = json!(0)),
            "a rack that did not occur should not be reported"),
        ("an implausible mean", |a| edit(rack_occurrences(a), |v| v["racks"][0]["mean"] = json!(6000.0)),
            "implausible mean equity"),
        ("a rack listed twice", |a| edit(rack_occurrences(a), |v| v["racks"][1]["rack"] = v["racks"][0]["rack"].clone()),
            "appears more than once"),
        ("more occurrences than the games drew", |a| edit(rack_occurrences(a), |v| v["racks"][0]["count"] = json!(100_000)),
            "a game cannot draw that many racks"),
    ];
    assert_each_refused(&db, &app, rack_occurrences, cases).await;
}

// --- A-WORKER-13, 14, 15 -----------------------------------------------------

/// A-WORKER-13: the artifact route serves a key only if the server minted it --
/// recorded in `leave_generation_artifacts` -- and answers `404` for any other,
/// including one naming an object that really is in the bucket. Otherwise it
/// is a read primitive for the whole bucket.
#[tokio::test]
async fn an_artifact_is_served_only_under_a_key_the_server_minted() {
    let db = TestDb::new().await;
    let (state, _bucket) = db.state_with_object_store().await;
    let app = birdtest::app(state.clone());
    let admin = db.user("admin", true).await;
    let job = db.bare_job("leave_generation", 1, admin).await;

    let minted = birdtest::jobs::leave_gen::artifact_key(job, 0);
    let leaves: Vec<u8> = (0..70_000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 11) as u8).collect();
    state.artifacts.put(&minted, leaves.clone()).await.unwrap();
    sqlx::query(
        "INSERT INTO leave_generation_artifacts (job_id, generation, artifact_key, sha256, builder)
         VALUES ($1, 0, $2, repeat('0', 64), 'klv-1')",
    )
    .bind(job)
    .bind(&minted)
    .execute(&db.pool)
    .await
    .unwrap();
    // Present in the bucket, and minted by nobody.
    state.artifacts.put("backups/secret.dump", b"not for workers".to_vec()).await.unwrap();

    let worker = registered_worker(&db).await;
    let fetch = |key: &str| {
        Request::get(format!("/api/worker/artifact?key={key}"))
            .header("x-worker-uuid", worker.as_str())
            .body(Body::empty())
            .unwrap()
    };

    let (status, headers, body) = raw(&app, fetch(&minted)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "application/octet-stream");
    assert_eq!(body, leaves, "the artifact came back altered");

    for key in [
        "backups/secret.dump".to_string(),
        birdtest::jobs::leave_gen::artifact_key(job, 1),
        format!("leaves/{job}/../../backups/secret.dump"),
    ] {
        let (status, body) = send(&app, fetch(&key)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{key}: {body}");
        assert_eq!(message(&body), "no such artifact", "{key}: {body}");
    }

    // And only for a worker the server knows.
    let (status, _) = send(&app, get_request(&format!("/api/worker/artifact?key={minted}"), &[])).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

/// A-WORKER-14: every worker endpoint shares one bucket per identity -- one
/// request a second with a burst of five (`ratelimit::RateLimiters`) -- so the
/// sixth request in a second is a `429` with a `Retry-After`, and another
/// identity is unaffected.
///
/// The six requests are heartbeats, the cheapest call there is, sent back to
/// back: the window they must fit is the one second before the bucket earns
/// its next token, which they take a few milliseconds of.
#[tokio::test]
async fn worker_requests_are_limited_per_identity() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let limited = registered_worker(&db).await;
    let other = registered_worker(&db).await;
    let token = json!(Uuid::new_v4());

    for i in 1..=5 {
        assert_eq!(heartbeat_as(&app, &limited, &token).await, StatusCode::NO_CONTENT, "request {i}");
    }
    let (status, headers, body) = raw(
        &app,
        post_json("/api/worker/heartbeat", &[("x-worker-uuid", &limited)], json!({ "claim_token": token })),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    let retry_after: u64 = headers["retry-after"].to_str().unwrap().parse().unwrap();
    assert!(retry_after >= 1, "{retry_after}");
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["code"], "rate_limited", "{body}");

    // The same bucket covers the claim, not just the heartbeat.
    let (status, _) = claim_as(&app, &limited).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);

    assert_eq!(heartbeat_as(&app, &other, &token).await, StatusCode::NO_CONTENT, "another identity is unaffected");
}

/// A-WORKER-14b: an account's worker is limited per API key, so two machines
/// on one account each have their own bucket -- keyed on the account, six idle
/// machines used it all -- and a key's sixth request in a burst is still 429.
#[tokio::test]
async fn an_accounts_workers_are_limited_per_key() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let user = db.user("contributor", false).await;
    let mut headers = Vec::new();
    for machine in ["machine-one", "machine-two"] {
        sqlx::query("INSERT INTO api_keys (user_id, key_hash) VALUES ($1, $2)")
            .bind(user)
            .bind(birdtest::auth::api_key::hash_key(machine))
            .execute(&db.pool)
            .await
            .unwrap();
        headers.push(format!("Bearer {machine}"));
    }
    let token = json!(Uuid::new_v4());
    let heartbeat = |auth: &str| {
        post_json("/api/worker/heartbeat", &[("authorization", auth)], json!({ "claim_token": token }))
    };

    for i in 1..=5 {
        let (status, _, _) = raw(&app, heartbeat(&headers[0])).await;
        assert_eq!(status, StatusCode::NO_CONTENT, "request {i}");
    }
    let (status, _, _) = raw(&app, heartbeat(&headers[0])).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "the key's own burst is spent");
    let (status, _, _) = raw(&app, heartbeat(&headers[1])).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "the account's other key is not");
}

/// A-WORKER-15: `client-version` reports the configured floor and where to get
/// MAGPIE, to anyone: a contributor asks before it has an identity.
#[tokio::test]
async fn client_version_reports_the_configured_floor_and_download_url() {
    let db = TestDb::new().await;
    let mut cfg = db.config();
    cfg.min_magpie_version = "1.10.2".into();
    cfg.magpie_download_url = "https://example.invalid/magpie/releases".into();
    let app = birdtest::app(db.state_with(cfg).await);

    let (status, body) = send(&app, get_request("/api/worker/client-version", &[])).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({
            "min_magpie_version": "1.10.2",
            "download_url": "https://example.invalid/magpie/releases",
        })
    );
}
