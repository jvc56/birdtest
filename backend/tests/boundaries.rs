//! Boundaries a coverage review found untested: the login limiter's two
//! halves, the role each input-data reference on a player config must have,
//! the result route's own body limit on the real router, the decline list's
//! bounds, the 100% allocation cap under concurrent activations, self-deletion,
//! a redundancy-2 task reopened by one lapsed slot beside one accepted result,
//! and the restart grace as the production state is built.

mod common;

use axum::body::Body;
use axum::http::header::{CONTENT_TYPE, RETRY_AFTER, SET_COOKIE};
use axum::http::{HeaderMap, Request, StatusCode};
use axum::Router;
use birdtest::routes::worker::MAX_RESULT_BYTES;
use birdtest::state::AppState;
use common::*;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tower::ServiceExt;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct Response {
    status: StatusCode,
    headers: HeaderMap,
    body: Value,
}

impl std::fmt::Debug for Response {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.status, self.body)
    }
}

/// Like `common::send`, keeping the headers.
async fn send_raw(app: &Router, request: Request<Body>) -> Response {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Response { status, headers, body }
}

fn borrow(headers: &[(String, String)]) -> Vec<(&str, &str)> {
    headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect()
}

fn request(method: &str, path: &str, headers: &[(String, String)]) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(path);
    for (name, value) in headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    builder.body(Body::empty()).unwrap()
}

/// Claims once with no identity, returning the assignment and the minted UUID.
async fn first_claim(app: &Router) -> (Value, String) {
    let (status, body) =
        send(app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let uuid = body["worker_uuid"].as_str().expect("a minted worker_uuid").to_string();
    (body, uuid)
}

async fn claim_as(app: &Router, uuid: &str) -> (StatusCode, Value) {
    send(app, post_json("/api/worker/task", &[("x-worker-uuid", uuid)], claim_body("1.0.0", &[])))
        .await
}

async fn submit_as(app: &Router, uuid: &str, token: &Value, result: Value) -> (StatusCode, Value) {
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

fn token(assignment: &Value) -> Uuid {
    assignment["claim_token"].as_str().unwrap().parse().unwrap()
}

async fn claim_state(db: &TestDb, token: Uuid) -> String {
    sqlx::query_scalar("SELECT state::text FROM task_claims WHERE claim_token = $1")
        .bind(token)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

/// Waits until at least `count` sessions of this test's database are blocked
/// on a lock (copied from `worker_api.rs`).
async fn wait_for_lock_waiters(db: &TestDb, count: i64) {
    let deadline = Instant::now() + Duration::from_secs(10);
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
        assert!(Instant::now() < deadline, "expected {count} session(s) to block");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

// ---------------------------------------------------------------------------
// Login rate limits
// ---------------------------------------------------------------------------

/// `RateLimiters::login`: `Quota::per_minute(10)`, whose burst is the whole
/// minute's ten. `ratelimit.rs` builds its quotas inline rather than from named
/// constants, so this mirrors that literal.
const LOGINS_PER_MINUTE: usize = 10;

const PASSWORD: &str = "vivid-otter-launches-quartz-72";
const WRONG: &str = "not-the-password-at-all-91";

/// A confirmed account with a real password hash.
async fn confirmed_user(db: &TestDb, username: &str, password: &str) -> Uuid {
    let hash = birdtest::auth::api_key::hash_password(password).unwrap();
    sqlx::query_scalar(
        "INSERT INTO users (username, email, password_hash, email_confirmed_at)
         VALUES ($1, $1 || '@example.invalid', $2, now()) RETURNING id",
    )
    .bind(username)
    .bind(hash)
    .fetch_one(&db.pool)
    .await
    .unwrap()
}

/// The router behind one trusted proxy, so `X-Forwarded-For` names the client.
async fn proxied_app(db: &TestDb) -> Router {
    let mut cfg = db.config();
    cfg.trusted_proxy_hops = 1;
    birdtest::app(db.state_with(cfg).await)
}

async fn login_from(app: &Router, ip: &str, username: &str, password: &str) -> Response {
    send_raw(
        app,
        post_json(
            "/api/auth/login",
            &[("x-forwarded-for", ip)],
            json!({ "username": username, "password": password }),
        ),
    )
    .await
}

fn assert_rate_limited(response: &Response, what: &str) {
    assert_eq!(response.status, StatusCode::TOO_MANY_REQUESTS, "{what}: {response:?}");
    assert_eq!(response.body["code"], "rate_limited", "{what}: {response:?}");
    let retry_after: u64 = response
        .headers
        .get(RETRY_AFTER)
        .unwrap_or_else(|| panic!("{what}: no Retry-After: {response:?}"))
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!(retry_after >= 1, "{what}: Retry-After {retry_after}");
    assert!(response.headers.get(SET_COOKIE).is_none(), "{what}: a limited attempt signs nobody in");
}

/// A-AUTH-11 (login, per address): the eleventh sign-in attempt from one
/// address in a minute is 429 with a `Retry-After` even though each tried a
/// different username -- and so is one with a real account's right password.
/// Another address is unaffected.
#[tokio::test]
async fn the_eleventh_login_from_one_address_is_rate_limited_even_with_the_right_password() {
    let db = TestDb::new().await;
    let app = proxied_app(&db).await;
    confirmed_user(&db, "realuser", PASSWORD).await;

    for i in 0..LOGINS_PER_MINUTE {
        let response = login_from(&app, "203.0.113.5", &format!("guess{i}"), WRONG).await;
        assert_eq!(response.status, StatusCode::UNAUTHORIZED, "#{i}: {response:?}");
    }
    let limited = login_from(&app, "203.0.113.5", "guess-last", WRONG).await;
    assert_rate_limited(&limited, "a new username from a spent address");
    let right = login_from(&app, "203.0.113.5", "realuser", PASSWORD).await;
    assert_rate_limited(&right, "the right password from a spent address");

    let elsewhere = login_from(&app, "198.51.100.9", "realuser", PASSWORD).await;
    assert_eq!(elsewhere.status, StatusCode::OK, "another address is unaffected: {elsewhere:?}");
}

/// `RateLimiters::login_account`: `Quota::per_minute(100)`, the cap on one
/// username from every address together.
const LOGINS_PER_ACCOUNT_PER_MINUTE: usize = 100;

/// A-AUTH-11 (login, per username), the half that matters since addresses are
/// cheap: the hundred-and-first attempt on one username in a minute is 429
/// although every attempt came from a different address -- with the right
/// password too, and whatever case or padding the name is typed with. Another
/// username is unaffected. (The attempts name an account that does not exist,
/// which the limiter cannot tell from one that does, and which costs no
/// Argon2 verify.)
#[tokio::test]
async fn a_username_tried_from_everywhere_is_rate_limited() {
    let db = TestDb::new().await;
    let app = proxied_app(&db).await;
    confirmed_user(&db, "bystander", PASSWORD).await;

    for i in 0..LOGINS_PER_ACCOUNT_PER_MINUTE {
        let ip = format!("10.{}.{}.1", i / 200, i % 200);
        let response = login_from(&app, &ip, "target", WRONG).await;
        assert_eq!(response.status, StatusCode::UNAUTHORIZED, "#{i}: {response:?}");
    }
    let limited = login_from(&app, "203.0.113.200", "target", PASSWORD).await;
    assert_rate_limited(&limited, "a spent username");
    let padded = login_from(&app, "203.0.113.201", " TARGET ", PASSWORD).await;
    assert_rate_limited(&padded, "case and padding do not make it another username");

    let other = login_from(&app, "203.0.113.202", "bystander", PASSWORD).await;
    assert_eq!(other.status, StatusCode::OK, "another username is unaffected: {other:?}");
}

/// A-BOUND-2: one address trying wrong passwords for an account cannot lock
/// its owner out. Keyed on the username alone at the per-address rate, one
/// wrong guess every six seconds from anywhere held any account -- an admin's,
/// whose name is public -- out of signing in, right password or not.
#[tokio::test]
async fn a_stranger_cannot_lock_an_account_out_of_signing_in() {
    let db = TestDb::new().await;
    let app = proxied_app(&db).await;
    confirmed_user(&db, "target", PASSWORD).await;

    for i in 0..LOGINS_PER_MINUTE {
        let response = login_from(&app, "203.0.113.66", "target", WRONG).await;
        assert_eq!(response.status, StatusCode::UNAUTHORIZED, "#{i}: {response:?}");
    }
    let limited = login_from(&app, "203.0.113.66", "target", WRONG).await;
    assert_rate_limited(&limited, "the guessing address");

    let owner = login_from(&app, "198.51.100.7", "target", PASSWORD).await;
    assert_eq!(owner.status, StatusCode::OK, "the owner signs in from their own address: {owner:?}");
}

// ---------------------------------------------------------------------------
// Player-config role checks
// ---------------------------------------------------------------------------

/// A-ADMIN-3 (player configs): `kwg_id`, `klv_id` and `winpct_id` must each
/// name an `input_data` row of that role. The foreign keys only say the row
/// exists, so each swap is refused with a 400 naming the role expected and the
/// role given, and creates nothing.
#[tokio::test]
async fn a_player_config_refuses_input_data_of_the_wrong_role() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let admin = db.user("root", true).await;
    let headers = admin_headers(&state.cfg, admin);
    let headers = borrow(&headers);
    let kwg = db.input_data("kwg", "NWL23").await;
    let klv = db.input_data("klv", "CSW24").await;
    let winpct = db.input_data("winpct", "winpct").await;
    let letterdist = db.input_data("letterdist", "english").await;
    let nothing = Uuid::new_v4();
    let unknown = format!("no input data row {nothing}");

    // A simmer, so all three references are in play; the last case shows the
    // body is otherwise acceptable.
    let body = |kwg_id: Uuid, klv_id: Uuid, winpct_id: Uuid| {
        json!({
            "name": format!("cfg-{}", Uuid::new_v4().simple()), "recorder_type": "best",
            "kwg_id": kwg_id, "klv_id": klv_id, "winpct_id": winpct_id,
            "num_plies": 2, "num_plays": 10, "num_plays_recorded": 1,
            "max_iterations": 100, "time_limit_secs": 0,
        })
    };
    let cases = [
        ("a klv as the kwg", body(klv, klv, winpct), "expected a kwg row, but CSW24 is a klv row"),
        (
            "a letter distribution as the kwg",
            body(letterdist, klv, winpct),
            "expected a kwg row, but english is a letterdist row",
        ),
        ("a kwg as the klv", body(kwg, kwg, winpct), "expected a klv row, but NWL23 is a kwg row"),
        (
            "a win% model as the klv",
            body(kwg, winpct, winpct),
            "expected a klv row, but winpct is a winpct row",
        ),
        (
            "a klv as the win% model",
            body(kwg, klv, klv),
            "expected a winpct row, but CSW24 is a klv row",
        ),
        (
            "a kwg as the win% model",
            body(kwg, klv, kwg),
            "expected a winpct row, but NWL23 is a kwg row",
        ),
        ("an unknown row", body(kwg, nothing, winpct), &unknown),
    ];
    for (what, body, message) in cases {
        let (status, response) =
            send(&app, post_json("/api/admin/player-configs", &headers, body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{what}: {response}");
        assert_eq!(response["code"], "bad_request", "{what}: {response}");
        assert_eq!(response["message"], message, "{what}: {response}");
    }
    let created: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM player_configs")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(created, 0, "a refused config is not stored");

    let (status, response) =
        send(&app, post_json("/api/admin/player-configs", &headers, body(kwg, klv, winpct))).await;
    assert_eq!(status, StatusCode::CREATED, "every role right: {response}");
}

// ---------------------------------------------------------------------------
// The result route's body limit, on the real router
// ---------------------------------------------------------------------------

/// A games job capturing positions, whose player 1 keeps `keep` moves.
async fn capturing_job(db: &TestDb, keep: i32) -> Uuid {
    let job = db.games_job(1, 2).await;
    sqlx::query("UPDATE job_game_config SET capture_positions = true WHERE job_id = $1")
        .bind(job)
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE player_configs SET num_plays_recorded = $2
         WHERE id = (SELECT player1_config_id FROM job_game_config WHERE job_id = $1)",
    )
    .bind(job)
    .bind(keep)
    .execute(&db.pool)
    .await
    .unwrap();
    db.derived_ready(job).await;
    job
}

/// Axum's own default body limit, which `/result` must be exempt from.
const AXUM_DEFAULT_BODY_LIMIT: usize = 2 * 1024 * 1024;

/// A-WORKER-10, A-WORKER-12 (body size): through the real router, a valid
/// result bigger than axum's 2 MB default -- a capture batch whose positions
/// each ranked thirty thousand moves -- is accepted and stored, which only
/// the route's own `DefaultBodyLimit::max(MAX_RESULT_BYTES)` allows.
#[tokio::test]
async fn a_capture_result_of_several_megabytes_is_accepted() {
    let db = TestDb::new().await;
    let job = capturing_job(&db, 3).await;
    let app = birdtest::app(db.state().await);
    let (assignment, uuid) = first_claim(&app).await;

    const MOVES: usize = 30_000;
    let position = |game: i32| {
        let moves: Vec<Value> = (0..MOVES)
            .map(|i| json!({ "move": format!("8D PADDING-{i:06}"), "score": 50, "equity": 60.0 }))
            .collect();
        json!({
            "game_index": game, "turn_number": 0, "rack": "AEINRST", "position": "cgp",
            "num_moves": MOVES, "moves": moves,
        })
    };
    let mut result = games_result(2, 1);
    result["positions"] = json!([position(0), position(1)]);
    let body = json!({ "claim_token": assignment["claim_token"], "result": result }).to_string();
    assert!(
        body.len() > AXUM_DEFAULT_BODY_LIMIT && body.len() < MAX_RESULT_BYTES,
        "the body is {} bytes: between axum's default and this route's limit",
        body.len()
    );

    let request = Request::post("/api/worker/result")
        .header(CONTENT_TYPE, "application/json")
        .header("x-worker-uuid", &uuid)
        .body(Body::from(body))
        .unwrap();
    let (status, answer) = send(&app, request).await;
    assert_eq!((status, &answer), (StatusCode::OK, &json!({ "accepted": true })));

    let kept: Vec<(i16, i32, i64)> = sqlx::query_as(
        "SELECT r.game_index, r.num_moves, COUNT(m.id)
         FROM position_analysis_records r
         LEFT JOIN position_analysis_moves m ON m.record_id = r.id
         WHERE r.job_id = $1 GROUP BY r.id ORDER BY r.game_index",
    )
    .bind(job)
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(kept, vec![(0, MOVES as i32, 3), (1, MOVES as i32, 3)]);
}

/// A-WORKER-12 (body size): through the real router, a result one byte over
/// `MAX_RESULT_BYTES` is a 413 in the API's error shape, and it touches
/// nothing: the claim stays open and its real result is then accepted.
#[tokio::test]
async fn a_result_over_the_limit_is_a_413_in_the_api_shape_and_changes_nothing() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 2).await;
    db.derived_ready(job).await;
    let app = birdtest::app(db.state().await);
    let (assignment, uuid) = first_claim(&app).await;

    // One buffer, moved into the request: valid JSON up to its padding, which
    // runs one byte past the limit.
    let head = format!("{{\"claim_token\":\"{}\",\"result\":\"", token(&assignment));
    let mut oversized = Vec::with_capacity(MAX_RESULT_BYTES + 1);
    oversized.extend_from_slice(head.as_bytes());
    oversized.resize(MAX_RESULT_BYTES - 1, b'x');
    oversized.extend_from_slice(b"\"}");
    assert_eq!(oversized.len(), MAX_RESULT_BYTES + 1);
    let request = Request::post("/api/worker/result")
        .header(CONTENT_TYPE, "application/json")
        .header("x-worker-uuid", &uuid)
        .body(Body::from(oversized))
        .unwrap();
    let (status, body) = send(&app, request).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
    assert_eq!(body["code"], "payload_too_large", "{body}");
    assert_eq!(body["message"], "the request body is larger than this endpoint accepts", "{body}");

    assert_eq!(claim_state(&db, token(&assignment)).await, "claimed", "the claim is untouched");
    let (status, body) = submit_as(&app, &uuid, &assignment["claim_token"], games_result(2, 1)).await;
    assert_eq!((status, &body), (StatusCode::OK, &json!({ "accepted": true })));
}

// ---------------------------------------------------------------------------
// Decline list bounds
// ---------------------------------------------------------------------------

/// A-WORKER-7 (bounds): a decline naming a hundred files, every field a
/// thousand characters, records exactly the first `MAX_MISSING_FILES` (32)
/// and each field cut to its first 128 characters -- characters, not bytes --
/// and still releases the claim.
#[tokio::test]
async fn a_decline_list_is_cut_to_its_cap_and_each_field_to_its_bound() {
    const SENT: usize = 100;
    const MAX_MISSING_FILES: usize = 32;
    const MAX_GAP_FIELD_CHARS: usize = 128;

    let db = TestDb::new().await;
    let job = db.games_job(1, 2).await;
    db.derived_ready(job).await;
    let app = birdtest::app(db.state().await);
    let (assignment, uuid) = first_claim(&app).await;

    // Two-byte characters after a distinguishing prefix, so a cut by bytes
    // would keep half as many.
    let field = |kind: &str, i: usize| format!("{kind}{i:03}-{}", "é".repeat(1000 - 7));
    let missing: Vec<Value> = (0..SENT)
        .map(|i| {
            json!({
                "role": field("rol", i), "name": field("nam", i),
                "expected": field("exp", i), "actual": field("act", i),
            })
        })
        .collect();
    assert_eq!(field("rol", 0).chars().count(), 1000);
    let (status, body) = send(
        &app,
        post_json(
            "/api/worker/decline",
            &[("x-worker-uuid", &uuid)],
            json!({ "claim_token": assignment["claim_token"], "reason": "missing_data", "missing": missing }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    let rows: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT role, name, expected, actual FROM worker_data_gaps WHERE job_id = $1 ORDER BY name",
    )
    .bind(job)
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), MAX_MISSING_FILES, "exactly the cap is recorded");
    let cut = |text: String| text.chars().take(MAX_GAP_FIELD_CHARS).collect::<String>();
    for (i, (role, name, expected, actual)) in rows.into_iter().enumerate() {
        assert_eq!(role, cut(field("rol", i)), "entry {i}: the first entries, in order");
        assert_eq!(name, cut(field("nam", i)));
        assert_eq!(expected, cut(field("exp", i)));
        assert_eq!(actual, Some(cut(field("act", i))));
        assert_eq!(role.chars().count(), MAX_GAP_FIELD_CHARS);
    }
    assert_eq!(claim_state(&db, token(&assignment)).await, "declined");
}

// ---------------------------------------------------------------------------
// The allocation cap
// ---------------------------------------------------------------------------

/// Two jobs, neither active.
async fn two_inactive_jobs(db: &TestDb) -> (Uuid, Uuid) {
    let first = db.games_job(1, 2).await;
    let second = db.games_job(1, 2).await;
    sqlx::query("UPDATE jobs SET status = 'inactive', allocation = NULL")
        .execute(&db.pool)
        .await
        .unwrap();
    (first, second)
}

async fn active_allocations(db: &TestDb) -> Vec<(Uuid, i32)> {
    sqlx::query_as("SELECT id, allocation FROM jobs WHERE status = 'active' ORDER BY id")
        .fetch_all(&db.pool)
        .await
        .unwrap()
}

/// A-ADMIN-4 (allocation cap): with 60% already active, activating another job
/// at 50% is a 409 naming the 40% of headroom, and changes nothing; 40%
/// itself -- exactly 100% in total -- is accepted.
#[tokio::test]
async fn an_activation_past_100_percent_is_refused_naming_the_headroom() {
    let db = TestDb::new().await;
    let (running, newcomer) = two_inactive_jobs(&db).await;
    sqlx::query("UPDATE jobs SET status = 'active', allocation = 60 WHERE id = $1")
        .bind(running)
        .execute(&db.pool)
        .await
        .unwrap();
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let headers = admin_headers(&state.cfg, db.user("root", true).await);
    let headers = borrow(&headers);
    let activate = |allocation: i32| {
        post_json(
            &format!("/api/admin/jobs/{newcomer}/activate"),
            &headers,
            json!({ "allocation": allocation }),
        )
    };

    let (status, body) = send(&app, activate(50)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "conflict", "{body}");
    assert_eq!(
        body["message"],
        "the other active jobs already allocate 60% — 40% is the most this job can take"
    );
    assert_eq!(active_allocations(&db).await, vec![(running, 60)], "nothing changed");

    let (status, body) = send(&app, activate(40)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let mut expected = vec![(running, 60), (newcomer, 40)];
    expected.sort();
    assert_eq!(active_allocations(&db).await, expected);
}

/// A-ADMIN-4 (allocation cap, concurrently): two jobs activated at 60% at the
/// same moment leave exactly one active. Each checks the others' sum under
/// the activation lock; without it each would read the other as absent and
/// both would go live at 120%.
///
/// Deterministic: the lock is held here until both activations are queued on
/// it, which is the interleaving that would let both through.
#[tokio::test]
async fn two_concurrent_activations_cannot_exceed_100_percent() {
    let db = TestDb::new().await;
    let (first, second) = two_inactive_jobs(&db).await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let headers = admin_headers(&state.cfg, db.user("root", true).await);
    let headers = borrow(&headers);
    let activate = |job: Uuid| {
        send(
            &app,
            post_json(&format!("/api/admin/jobs/{job}/activate"), &headers, json!({ "allocation": 60 })),
        )
    };

    let mut blocker = db.pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('birdtest.activate'))")
        .execute(&mut *blocker)
        .await
        .unwrap();
    let release = async {
        wait_for_lock_waiters(&db, 2).await;
        blocker.commit().await.unwrap();
    };
    let (a, b, ()) = tokio::join!(activate(first), activate(second), release);

    let mut statuses = [a.0, b.0];
    statuses.sort();
    assert_eq!(statuses, [StatusCode::OK, StatusCode::CONFLICT], "{} / {}", a.1, b.1);
    let refused = if a.0 == StatusCode::CONFLICT { &a.1 } else { &b.1 };
    assert_eq!(
        refused["message"],
        "the other active jobs already allocate 60% — 40% is the most this job can take"
    );
    let active = active_allocations(&db).await;
    assert_eq!(active.len(), 1, "exactly one went live: {active:?}");
    assert_eq!(active[0].1, 60);
}

// ---------------------------------------------------------------------------
// Self-deletion
// ---------------------------------------------------------------------------

/// A-ADMIN-14: an admin cannot delete their own account. The refusal is a 400
/// saying so, the account is untouched and its session still works; the same
/// admin deleting someone else succeeds.
#[tokio::test]
async fn an_admin_cannot_delete_their_own_account() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let admin = db.user("root", true).await;
    let other = db.user("someone", false).await;
    let headers = admin_headers(&state.cfg, admin);

    let (status, body) = send(&app, request("DELETE", &format!("/api/admin/users/{admin}"), &headers)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "bad_request", "{body}");
    assert_eq!(body["message"], "you cannot delete your own account");

    let account: (String, bool, bool) = sqlx::query_as(
        "SELECT username, is_admin, deleted_at IS NULL FROM users WHERE id = $1",
    )
    .bind(admin)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(account, ("root".into(), true, true), "the account still exists, unchanged");
    let (status, me) = send(&app, get_request("/api/me", &headers)).await;
    assert_eq!(status, StatusCode::OK, "the session still works: {me}");
    assert_eq!(me["username"], "root", "{me}");

    let (status, body) = send(&app, request("DELETE", &format!("/api/admin/users/{other}"), &headers)).await;
    assert!(status.is_success(), "deleting another account is allowed: {status} {body}");
    let deleted: bool = sqlx::query_scalar("SELECT deleted_at IS NOT NULL FROM users WHERE id = $1")
        .bind(other)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert!(deleted);
}

// ---------------------------------------------------------------------------
// Redundancy 2: one slot accepted, one lapsed
// ---------------------------------------------------------------------------

/// The state and counters of the task `claim` is a slot on.
async fn task_row(db: &TestDb, claim: Uuid) -> (String, i32, i32) {
    sqlx::query_as(
        "SELECT t.state::text, t.accepted_count, t.active_claim_count
         FROM tasks t JOIN task_claims c ON c.task_id = t.id WHERE c.claim_token = $1",
    )
    .bind(claim)
    .fetch_one(&db.pool)
    .await
    .unwrap()
}

/// I-SCHED-11, I-SCHED-14 (mixed reclaim): on a redundancy-2 task with one
/// slot accepted and the other lapsed, reclamation reopens the task --
/// `available`, one accepted, none live -- rather than leaving it `claimed` or
/// marking it `completed`. The worker whose result was accepted is never
/// offered the reopened slot (its next claim is a different task, not a 204),
/// and a third worker gets the same seed.
#[tokio::test]
async fn a_task_with_one_accepted_and_one_lapsed_slot_reopens_for_someone_else() {
    let db = TestDb::new().await;
    let job = db.games_job(2, 2).await;
    db.derived_ready(job).await;
    let app = birdtest::app(db.state().await);

    let (a, a_uuid) = first_claim(&app).await;
    let (b, _) = first_claim(&app).await;
    let seed = a["task_request"]["seed"].clone();
    assert_eq!(b["task_request"]["seed"], seed, "B fills the task's other slot");
    let (status, body) = submit_as(&app, &a_uuid, &a["claim_token"], games_result(2, 1)).await;
    assert_eq!((status, &body), (StatusCode::OK, &json!({ "accepted": true })));
    assert_eq!(task_row(&db, token(&a)).await, ("claimed".into(), 1, 1));

    // B's claim lapses: its last sign of life is an hour old.
    sqlx::query(
        "UPDATE task_claims SET claimed_at = now() - interval '1 hour', last_heartbeat_at = NULL
         WHERE claim_token = $1",
    )
    .bind(token(&b))
    .execute(&db.pool)
    .await
    .unwrap();

    let (status, next) = claim_as(&app, &a_uuid).await;
    assert_eq!(status, StatusCode::OK, "A is given other work, not told to wait: {next}");
    assert_ne!(next["task_request"]["seed"], seed, "A is never offered the task it already did");
    assert_eq!(claim_state(&db, token(&b)).await, "abandoned");
    assert_eq!(
        task_row(&db, token(&a)).await,
        ("available".into(), 1, 0),
        "one accepted and one lapsed reopens the task"
    );

    let (c, _) = first_claim(&app).await;
    assert_eq!(c["task_request"]["seed"], seed, "C gets the reopened slot");
    assert_eq!(task_row(&db, token(&a)).await, ("claimed".into(), 1, 1));
}

// ---------------------------------------------------------------------------
// The restart grace, as production builds the state
// ---------------------------------------------------------------------------

/// The AWS settings `common::state_with` sets before building an artifact
/// store, so the SDK does not go looking for instance metadata.
fn aws_env() {
    for (key, value) in [
        ("AWS_ACCESS_KEY_ID", "birdtest"),
        ("AWS_SECRET_ACCESS_KEY", "birdtestbirdtest"),
        ("AWS_REGION", "us-east-1"),
        ("AWS_EC2_METADATA_DISABLED", "true"),
    ] {
        if std::env::var_os(key).is_none() {
            std::env::set_var(key, value);
        }
    }
}

/// I-SCHED-20, as the binary wires it: `AppState::new` -- what `main` builds
/// the server's state with -- sets the grace to start plus the heartbeat
/// timeout, so a claim an hour past its timeout is not reclaimed by the
/// freshly built process: another worker is handed a different seed and the
/// claim stays open. Once the grace has passed (moved here, not waited for),
/// the same claim is reclaimed and its task handed out again.
#[tokio::test]
async fn a_freshly_built_production_state_grants_the_restart_grace() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 2).await;
    db.derived_ready(job).await;
    aws_env();
    let cfg = Arc::new(db.config());
    let before = Instant::now();
    let state = AppState::new(
        cfg.clone(),
        db.pool.clone(),
        db.pool.clone(),
        birdtest::magpie::Magpie::new(&cfg.magpie_bin, 1),
        test_builders(),
    )
    .await;
    let after = Instant::now();
    assert!(
        state.reclaim_from >= before + cfg.heartbeat_timeout
            && state.reclaim_from <= after + cfg.heartbeat_timeout,
        "the grace is the heartbeat timeout from when the state was built"
    );

    let app = birdtest::app(state.clone());
    let (silent, _) = first_claim(&app).await;
    sqlx::query(
        "UPDATE task_claims SET claimed_at = now() - interval '1 hour', last_heartbeat_at = NULL",
    )
    .execute(&db.pool)
    .await
    .unwrap();

    let (other, _) = first_claim(&app).await;
    assert_ne!(
        other["task_request"]["seed"], silent["task_request"]["seed"],
        "a young process does not hand out a task whose worker it could not have heard from"
    );
    assert_eq!(claim_state(&db, token(&silent)).await, "claimed");

    let mut graced = state;
    graced.reclaim_from = Instant::now();
    let app = birdtest::app(graced);
    let (third, _) = first_claim(&app).await;
    assert_eq!(claim_state(&db, token(&silent)).await, "abandoned", "reclaimed once the grace is over");
    assert_eq!(third["task_request"]["seed"], silent["task_request"]["seed"], "and its task handed out again");
}

/// A-BOUND-11: the API's answers carry `X-Content-Type-Options: nosniff`. The
/// pages get it from Nginx; the API is served by the load balancer straight
/// from the backend, so it has to set its own -- the Nginx comment said it did,
/// and nothing did.
#[tokio::test]
async fn api_responses_are_not_sniffed() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    for path in ["/health", "/api/jobs", "/api/jobs/00000000-0000-0000-0000-000000000000"] {
        let response = send_raw(&app, Request::get(path).body(Body::empty()).unwrap()).await;
        assert_eq!(
            response.headers.get("x-content-type-options").map(|v| v.to_str().unwrap()),
            Some("nosniff"),
            "{path}"
        );
    }
}

/// A-BOUND-12: a malformed id in a path is a JSON `404`, and a malformed query
/// string a JSON `400` -- not axum's plain-text rejections, which broke the
/// API's promise that every failure carries a `code` and a `message`.
#[tokio::test]
async fn malformed_paths_and_queries_answer_json() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let response =
        send_raw(&app, Request::get("/api/jobs/not-a-uuid").body(Body::empty()).unwrap()).await;
    assert_eq!(response.status, StatusCode::NOT_FOUND, "{response:?}");
    assert_eq!(response.body["code"], "not_found", "{response:?}");
    let response =
        send_raw(&app, Request::get("/api/jobs?page=many").body(Body::empty()).unwrap()).await;
    assert_eq!(response.status, StatusCode::BAD_REQUEST, "{response:?}");
    assert_eq!(response.body["code"], "bad_request", "{response:?}");
    // And an endpoint that does not exist, or a method one does not take.
    let response =
        send_raw(&app, Request::get("/api/no-such-thing").body(Body::empty()).unwrap()).await;
    assert_eq!(response.status, StatusCode::NOT_FOUND, "{response:?}");
    assert_eq!(response.body["code"], "not_found", "{response:?}");
    let response = send_raw(&app, Request::delete("/api/jobs").body(Body::empty()).unwrap()).await;
    assert_eq!(response.status, StatusCode::METHOD_NOT_ALLOWED, "{response:?}");
    assert_eq!(response.body["code"], "method_not_allowed", "{response:?}");
}
