//! A completed job's export, end to end against a real object store
//! (`TEST_S3_ENDPOINT`): PLAN.md, "Exports". TESTING.md names no id for these;
//! each test cites the design sentence it proves.
//!
//! Every other test of the export either stops at the refusal or points the
//! state at a closed object store, so the success path -- the artifacts'
//! contents, their digests, the redirect, the purge -- ran nowhere.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::*;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::time::{Duration, Instant};
use tower::ServiceExt;
use uuid::Uuid;

async fn first_claim(app: &axum::Router) -> (serde_json::Value, String) {
    let (status, body) =
        send(app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let uuid = body["worker_uuid"].as_str().expect("a minted worker_uuid").to_string();
    (body, uuid)
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

fn borrowed(headers: &[(String, String)]) -> Vec<(&str, &str)> {
    headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect()
}

/// The status and headers of a response, and its body as text -- for the
/// NDJSON stream and the redirect, which `send`'s JSON parsing does not fit.
async fn send_raw(
    app: &axum::Router,
    request: Request<Body>,
) -> (StatusCode, axum::http::HeaderMap, String) {
    let response = app.clone().oneshot(request).await.unwrap();
    let (parts, body) = response.into_parts();
    let bytes = axum::body::to_bytes(body, 64 * 1024 * 1024).await.unwrap();
    (parts.status, parts.headers, String::from_utf8(bytes.to_vec()).unwrap())
}

/// Lines of NDJSON, sorted: neither the stream nor the export orders its rows,
/// so the same corpus may come out in two orders.
fn sorted_lines(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    lines.sort();
    lines
}

fn gunzip(bytes: &[u8]) -> String {
    let mut text = String::new();
    flate2::read::GzDecoder::new(bytes).read_to_string(&mut text).unwrap();
    text
}

async fn download(url: &str) -> Vec<u8> {
    let response = reqwest::get(url).await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK, "{url}");
    response.bytes().await.unwrap().to_vec()
}

/// A games job capturing positions, with `results` accepted results of two
/// captured positions each, completed by SQL -- every claim has landed, so the
/// export is not refused as unsettled.
async fn completed_capture_job(db: &TestDb, app: &axum::Router, results: usize) -> Uuid {
    let job = db.games_job(1, 2).await;
    sqlx::query("UPDATE job_game_config SET capture_positions = true WHERE job_id = $1")
        .bind(job)
        .execute(&db.pool)
        .await
        .unwrap();
    for i in 0..results {
        let mut result = games_result(2, 1);
        result["positions"] = json!([
            { "game_index": 0, "turn_number": 0, "rack": "AEINRST", "position": format!("cgp-{i}-0"),
              "num_moves": 40, "moves": [{ "move": "8D RETAINS", "score": 74, "equity": 81.2 }] },
            { "game_index": 1, "turn_number": 3, "rack": "AEINRSU", "position": format!("cgp-{i}-1"),
              "previous_move": "8D DOG", "previous_move_score": 10,
              "num_moves": 30, "moves": [{ "move": "8D URINATES", "score": 70, "equity": 77.0 }] },
        ]);
        let (claim, uuid) = first_claim(app).await;
        let (status, body) =
            submit_as(app, &uuid, claim["claim_token"].as_str().unwrap(), result).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body, json!({ "accepted": true }));
    }
    job
}

async fn complete(db: &TestDb, job: Uuid) {
    sqlx::query("UPDATE jobs SET status = 'completed' WHERE id = $1")
        .bind(job)
        .execute(&db.pool)
        .await
        .unwrap();
}

/// Starts an export over HTTP and polls `GET` until it leaves `running`.
async fn export_and_wait(
    app: &axum::Router,
    job: Uuid,
    headers: &[(String, String)],
) -> serde_json::Value {
    let path = format!("/api/admin/jobs/{job}/export");
    let (status, body) = send(app, post_json(&path, &borrowed(headers), json!({}))).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["state"], "running", "{body}");
    let id = body["id"].as_str().unwrap().to_string();

    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let (status, body) = send(app, get_request(&path, headers)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["id"], id.as_str(), "GET answers with the newest export");
        if body["state"] != "running" {
            return body;
        }
        assert!(Instant::now() < deadline, "the export never finished: {body}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// PLAN.md, "Exports": the export is the job's corpus read once -- the admin
/// stream "runs the same queries, so the two are the same corpus" -- as
/// gzipped NDJSON behind a presigned URL, with `row_count` and a digest
/// recorded. A games job that captured positions gets a second object,
/// `…/<export>.positions.ndjson.gz`, with `positions_row_count`,
/// `positions_bytes` and a `positions_download_url`; both live under
/// `exports/`.
#[tokio::test]
async fn a_completed_jobs_export_is_its_stream_and_its_positions_behind_presigned_urls() {
    let db = TestDb::new().await;
    let (state, bucket) = db.state_with_object_store().await;
    let app = birdtest::app(state.clone());
    let admin = db.user("root", true).await;
    let headers = admin_headers(&state.cfg, admin);
    let job = completed_capture_job(&db, &app, 3).await;

    // What the live stream says, read while the job is active and so before
    // any export exists to redirect it.
    let stream = format!("/api/admin/jobs/{job}/results/stream");
    let (status, _, results_text) = send_raw(&app, get_request(&stream, &headers)).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, positions_text) =
        send_raw(&app, get_request(&format!("{stream}?positions=true"), &headers)).await;
    assert_eq!(status, StatusCode::OK);
    let (streamed_results, streamed_positions) =
        (sorted_lines(&results_text), sorted_lines(&positions_text));
    assert_eq!((streamed_results.len(), streamed_positions.len()), (3, 6));

    complete(&db, job).await;
    let export = export_and_wait(&app, job, &headers).await;
    assert_eq!(export["state"], "ready", "{export}");
    assert!(export["error"].is_null(), "{export}");
    assert!(export["completed_at"].is_string(), "{export}");
    assert_eq!(export["row_count"], 3, "{export}");
    assert_eq!(export["positions_row_count"], 6, "{export}");
    let id = export["id"].as_str().unwrap();

    // Both objects, and nothing else, under the job's exports/ prefix.
    let results_key = format!("exports/{job}/{id}.ndjson.gz");
    let positions_key = format!("exports/{job}/{id}.positions.ndjson.gz");
    let mut keys = bucket.keys().await;
    keys.sort();
    let mut expected = vec![results_key.clone(), positions_key.clone()];
    expected.sort();
    assert_eq!(keys, expected);

    // Fetched the way an admin fetches them: through the presigned URLs, so
    // the bytes never pass through the backend.
    for (url_field, prefix, key, streamed) in [
        ("download_url", "", &results_key, &streamed_results),
        ("positions_download_url", "positions_", &positions_key, &streamed_positions),
    ] {
        let url = export[url_field].as_str().unwrap_or_else(|| panic!("{url_field}: {export}"));
        assert!(url.contains(key.as_str()), "{url} is not {key}");
        let bytes = download(url).await;
        assert_eq!(bytes, state.artifacts.get(key).await.unwrap(), "{key}");
        assert_eq!(export[format!("{prefix}bytes")], bytes.len() as i64, "{key}");
        assert_eq!(
            export[format!("{prefix}sha256")],
            hex::encode(Sha256::digest(&bytes)),
            "{key}: the recorded digest is of the object as stored"
        );
        let lines = sorted_lines(&gunzip(&bytes));
        assert_eq!(&lines, streamed, "{key} holds exactly what the stream streamed");
    }

    // The positions are the captured ones, one shape per file.
    let positions: Vec<serde_json::Value> =
        streamed_positions.iter().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert!(positions.iter().all(|p| p["moves"].is_array() && p["position"].is_string()));
    let results: Vec<serde_json::Value> =
        streamed_results.iter().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert!(results.iter().all(|r| r["games"] == 2 && r.get("moves").is_none()), "{results:?}");
}

/// PLAN.md, "Exports": a completed job's stream "redirects to an export
/// rather than re-scanning" -- `303` to the newest ready export's object, and
/// with `?positions=true` to its positions object.
#[tokio::test]
async fn a_completed_jobs_stream_redirects_to_its_ready_export() {
    let db = TestDb::new().await;
    let (state, _bucket) = db.state_with_object_store().await;
    let app = birdtest::app(state.clone());
    let admin = db.user("root", true).await;
    let headers = admin_headers(&state.cfg, admin);
    let job = completed_capture_job(&db, &app, 1).await;
    complete(&db, job).await;

    // Completed but never exported: streamed from the database, as before.
    let stream = format!("/api/admin/jobs/{job}/results/stream");
    let (status, _, body) = send_raw(&app, get_request(&stream, &headers)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.lines().count(), 1);

    let export = export_and_wait(&app, job, &headers).await;
    assert_eq!(export["state"], "ready", "{export}");
    let id = export["id"].as_str().unwrap();

    for (query, key) in [
        ("", format!("exports/{job}/{id}.ndjson.gz")),
        ("?positions=true", format!("exports/{job}/{id}.positions.ndjson.gz")),
    ] {
        let (status, response_headers, _) =
            send_raw(&app, get_request(&format!("{stream}{query}"), &headers)).await;
        assert_eq!(status, StatusCode::SEE_OTHER, "{query}");
        let location = response_headers["location"].to_str().unwrap();
        assert!(location.contains(&key), "{query}: {location} is not {key}");
        assert_eq!(download(location).await, state.artifacts.get(&key).await.unwrap());
    }

    // Past the lifetime the bucket keeps an export for, the stream goes back to
    // the database rather than redirect to an object that is gone, and the
    // admin page says so rather than offer a dead download.
    sqlx::query("UPDATE job_exports SET completed_at = completed_at - interval '30 days' WHERE job_id = $1")
        .bind(job)
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, _, body) = send_raw(&app, get_request(&stream, &headers)).await;
    assert_eq!(status, StatusCode::OK, "an expired export is not redirected to");
    assert_eq!(body.lines().count(), 1);
    let (status, detail) =
        send(&app, get_request(&format!("/api/admin/jobs/{job}/export"), &headers)).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["state"], "expired", "{detail}");
    assert!(detail["download_url"].is_null(), "{detail}");
}

/// PLAN.md, "Exports": only a games job that captured something grows a
/// second object -- and an empty corpus is still a valid export, one gzip
/// stream of nothing with `row_count` 0 and no positions fields at all.
#[tokio::test]
async fn an_export_of_a_job_with_nothing_captured_is_one_object() {
    let db = TestDb::new().await;
    let (state, bucket) = db.state_with_object_store().await;
    let app = birdtest::app(state.clone());
    let admin = db.user("root", true).await;
    let headers = admin_headers(&state.cfg, admin);
    // Capturing, but nobody contributed: asked of the rows, not the setting.
    let job = completed_capture_job(&db, &app, 0).await;
    complete(&db, job).await;

    let export = export_and_wait(&app, job, &headers).await;
    assert_eq!(export["state"], "ready", "{export}");
    assert_eq!(export["row_count"], 0, "{export}");
    for field in ["positions_row_count", "positions_bytes", "positions_sha256"] {
        assert!(export[field].is_null(), "{field}: {export}");
    }
    assert!(export.get("positions_download_url").is_none(), "{export}");

    let id = export["id"].as_str().unwrap();
    assert_eq!(bucket.keys().await, vec![format!("exports/{job}/{id}.ndjson.gz")]);
    let bytes = download(export["download_url"].as_str().unwrap()).await;
    assert_eq!(export["sha256"], hex::encode(Sha256::digest(&bytes)));
    assert_eq!(gunzip(&bytes), "");
}

/// PLAN.md, "Exports": "a purge deletes a job's exports with the results they
/// describe" -- both objects and the row, so no `ready` export of the purged
/// job survives to be downloaded or redirected to.
#[tokio::test]
async fn a_purge_deletes_both_export_objects_and_the_row() {
    let db = TestDb::new().await;
    let (state, bucket) = db.state_with_object_store().await;
    let app = birdtest::app(state.clone());
    let admin = db.user("root", true).await;
    let headers = admin_headers(&state.cfg, admin);
    let job = completed_capture_job(&db, &app, 2).await;
    complete(&db, job).await;
    let export = export_and_wait(&app, job, &headers).await;
    assert_eq!(export["state"], "ready", "{export}");
    assert_eq!(bucket.keys().await.len(), 2);

    let purge = Request::post(format!("/api/admin/jobs/{job}/purge"));
    let purge = headers
        .iter()
        .fold(purge, |b, (k, v)| b.header(k.as_str(), v.as_str()))
        .body(Body::empty())
        .unwrap();
    let (status, body) = send(&app, purge).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM job_exports WHERE job_id = $1")
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(rows, 0, "the export row goes with the results it described");
    let (status, _) =
        send(&app, get_request(&format!("/api/admin/jobs/{job}/export"), &headers)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // The objects are deleted off the request (best effort, on a task of its
    // own), so the bucket is watched until they are gone.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let keys = bucket.keys().await;
        if keys.is_empty() {
            break;
        }
        assert!(Instant::now() < deadline, "the purge left {keys:?} behind");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn export_row(db: &TestDb, job: Uuid, state: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO job_exports (job_id, state, requested_at) VALUES ($1, $2, now()) RETURNING id",
    )
    .bind(job)
    .bind(state)
    .fetch_one(&db.pool)
    .await
    .unwrap()
}

async fn export_state(db: &TestDb, id: Uuid) -> (String, Option<String>, bool) {
    sqlx::query_as(
        "SELECT state, error, completed_at IS NOT NULL FROM job_exports WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&db.pool)
    .await
    .unwrap()
}

/// I-EXPORT-8: one export of a job runs at a time. Only the page's disabled
/// button stopped a second, and each holds a pool connection for its whole
/// corpus read.
#[tokio::test]
async fn a_job_has_one_export_running_at_a_time() {
    let db = TestDb::new().await;
    let (state, _bucket) = db.state_with_object_store().await;
    let app = birdtest::app(state.clone());
    let admin = db.user("root", true).await;
    let headers = admin_headers(&state.cfg, admin);
    let job = completed_capture_job(&db, &app, 1).await;
    complete(&db, job).await;
    export_row(&db, job, "running").await;

    let borrowed: Vec<(&str, &str)> =
        headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let (status, body) =
        send(&app, post_json(&format!("/api/admin/jobs/{job}/export"), &borrowed, json!({}))).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["message"].as_str().unwrap().contains("already running"), "{body}");
}

/// PLAN.md, "Exports" and the schema's comment on `job_exports`: birdtest is a
/// single instance, so at startup an export still `running` belongs to a
/// process that is gone. `fail_orphaned` marks it failed with a reason and
/// leaves ready and failed exports alone; `GET` then reports the failure and
/// offers no download.
#[tokio::test]
async fn startup_fails_exports_left_running_and_leaves_the_rest_alone() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let admin = db.user("root", true).await;
    let headers = admin_headers(&state.cfg, admin);
    let (done, orphan) = (db.games_job(1, 2).await, db.games_job(1, 2).await);
    let ready = export_row(&db, done, "ready").await;
    let failed = export_row(&db, done, "failed").await;
    let running = export_row(&db, orphan, "running").await;

    assert_eq!(birdtest::exports::fail_orphaned(&db.pool).await.unwrap(), 1);

    let (state_, error, completed) = export_state(&db, running).await;
    assert_eq!(state_, "failed");
    assert!(error.as_deref().unwrap_or("").contains("restarted"), "{error:?}");
    assert!(completed, "a reaped export records when it ended");
    assert_eq!(export_state(&db, ready).await, ("ready".into(), None, false));
    assert_eq!(export_state(&db, failed).await, ("failed".into(), None, false));

    let (status, body) =
        send(&app, get_request(&format!("/api/admin/jobs/{orphan}/export"), &headers)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "failed", "{body}");
    assert!(body["error"].as_str().unwrap().contains("restarted"), "{body}");
    assert!(body.get("download_url").is_none(), "{body}");

    // Idempotent: nothing is left running.
    assert_eq!(birdtest::exports::fail_orphaned(&db.pool).await.unwrap(), 0);
}

/// Waits until a session running a statement that starts with `statement` is
/// blocked on a lock.
async fn wait_for_lock_waiter(db: &TestDb, statement: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pg_stat_activity
             WHERE datname = current_database() AND wait_event_type = 'Lock'
               AND state = 'active' AND ltrim(query) LIKE $1 || '%'",
        )
        .bind(statement)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        if waiting > 0 {
            return;
        }
        assert!(Instant::now() < deadline, "expected {statement:?} to block");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// `exports::build`'s guard: a process starting while an export runs --
/// a rolling deployment overlaps the two -- reaps its row, and the export's
/// own task finishing afterwards must not bring it back `ready`, or an admin
/// is handed an export nobody was sure had finished; and it removes the
/// objects it uploaded, which nothing will name.
///
/// Deterministic: the export is held at its first read of the results until
/// the row has been reaped, and the app runs on a pool of its own, named, so
/// the test can see when the export's final update has run -- its session
/// goes idle with that statement as its last.
#[tokio::test]
async fn an_export_reaped_while_it_ran_is_not_brought_back_ready() {
    let db = TestDb::new().await;
    let (mut state, bucket) = db.state_with_object_store().await;
    let options: sqlx::postgres::PgConnectOptions = db.url.parse().unwrap();
    let app_pool = sqlx::PgPool::connect_with(options.application_name("export-under-test"))
        .await
        .unwrap();
    state.pool = app_pool.clone();
    state.read_pool = app_pool.clone();
    let app = birdtest::app(state.clone());
    let admin = db.user("root", true).await;
    let headers = admin_headers(&state.cfg, admin);
    let job = completed_capture_job(&db, &app, 1).await;
    complete(&db, job).await;

    let mut reads = db.pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE game_results IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *reads)
        .await
        .unwrap();
    let (status, body) = send(
        &app,
        post_json(&format!("/api/admin/jobs/{job}/export"), &borrowed(&headers), json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let id: Uuid = body["id"].as_str().unwrap().parse().unwrap();
    wait_for_lock_waiter(&db, "SELECT to_jsonb(r)::text AS row FROM game_results").await;

    // The new process's startup, while the export is mid-read.
    assert_eq!(birdtest::exports::fail_orphaned(&db.pool).await.unwrap(), 1);
    reads.commit().await.unwrap();

    // The export goes on to upload both objects and then runs its final
    // update; wait for that statement to have finished.
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let finished: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_stat_activity
                            WHERE application_name = 'export-under-test' AND state = 'idle'
                              AND ltrim(query) LIKE 'UPDATE job_exports%')",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        if finished {
            break;
        }
        assert!(Instant::now() < deadline, "the export never reached its final update");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // It ran to the end -- and, finding its row gone from `running`, removed
    // the two objects it had uploaded, which nothing would ever name.
    let deadline = Instant::now() + Duration::from_secs(30);
    while !bucket.keys().await.is_empty() {
        assert!(Instant::now() < deadline, "the unclaimed objects were left behind");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let (state_, error, _) = export_state(&db, id).await;
    assert_eq!(state_, "failed");
    assert!(error.unwrap_or_default().contains("restarted"));

    // So nothing redirects to it, and GET offers no download.
    let (status, body) =
        send(&app, get_request(&format!("/api/admin/jobs/{job}/export"), &headers)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "failed", "{body}");
    assert!(body.get("download_url").is_none(), "{body}");
    let (status, _, _) = send_raw(
        &app,
        get_request(&format!("/api/admin/jobs/{job}/results/stream"), &headers),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "streamed from the database, not redirected");
}
