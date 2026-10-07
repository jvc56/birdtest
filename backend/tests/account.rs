//! The Account API through the real router (TESTING.md, `A-ACCOUNT-*`): the
//! caller's own account and API keys.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use common::*;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use uuid::Uuid;

/// A session for `user` with a matching CSRF pair.
fn signed_in(db: &TestDb, user: Uuid) -> Vec<(String, String)> {
    admin_headers(&db.config(), user)
}

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

async fn create_key(app: &Router, headers: &[(String, String)], label: &str) -> Value {
    let (status, body) =
        send(app, request("POST", "/api/me/api-keys", headers, Some(json!({ "label": label })))).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body
}

async fn list_keys(app: &Router, headers: &[(String, String)]) -> Value {
    let (status, body) = send(app, get_request("/api/me/api-keys", headers)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

async fn set_active(app: &Router, headers: &[(String, String)], key: &Value, active: bool) -> (StatusCode, Value) {
    send(
        app,
        request(
            "PATCH",
            &format!("/api/me/api-keys/{}", key["id"].as_str().unwrap()),
            headers,
            Some(json!({ "is_active": active })),
        ),
    )
    .await
}

async fn revoke(app: &Router, headers: &[(String, String)], key: &Value) -> (StatusCode, Value) {
    send(
        app,
        request("DELETE", &format!("/api/me/api-keys/{}", key["id"].as_str().unwrap()), headers, None),
    )
    .await
}

/// A claim authenticated by `raw_key`. There is no job, so a key that is
/// accepted gets 204 (nothing to do) and one that is not gets 401.
async fn claim_with(app: &Router, raw_key: &str) -> (StatusCode, Value) {
    let bearer = format!("Bearer {raw_key}");
    send(app, post_json("/api/worker/task", &[("authorization", &bearer)], claim_body("1.0.0", &[]))).await
}

fn object_keys(value: &Value) -> BTreeSet<&str> {
    value.as_object().unwrap().keys().map(String::as_str).collect()
}

/// A-ACCOUNT-1: `GET /api/me` answers with the caller's own account and
/// nothing of its password: not the hash, not any field about it.
#[tokio::test]
async fn me_returns_the_caller_and_never_a_password_hash() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    db.user("someoneelse", false).await;
    let hash = birdtest::auth::api_key::hash_password("vivid-otter-launches-quartz-72").unwrap();
    let user: Uuid = sqlx::query_scalar(
        "INSERT INTO users (username, email, password_hash, email_confirmed_at)
         VALUES ('me', 'me@example.invalid', $1, now()) RETURNING id",
    )
    .bind(&hash)
    .fetch_one(&db.pool)
    .await
    .unwrap();

    let (status, body) = send(&app, get_request("/api/me", &signed_in(&db, user))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({
            "id": user, "username": "me", "email": "me@example.invalid",
            "is_admin": false, "tasks_completed": 0
        })
    );
    let text = body.to_string();
    for secret in [hash.as_str(), "$argon2", "password"] {
        assert!(!text.contains(secret), "{secret:?} in {text}");
    }
}

/// A-ACCOUNT-6: key creation is limited per account where it matters. Each
/// key is a worker rate-limit bucket of its own, so revoking a key and making
/// another was unmetered new capacity; the hundred-key cap alone did not
/// bound it. A first hundred keys at once -- a machine each -- are not held
/// back; making more by revoking is, at ten an hour.
#[tokio::test]
async fn key_churn_is_rate_limited_per_account() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let user = db.user("churner", false).await;
    let headers = signed_in(&db, user);
    let mut last = Value::Null;
    for n in 0..100 {
        last = create_key(&app, &headers, &format!("machine {n}")).await;
    }
    let (status, body) = send(
        &app,
        request("DELETE", &format!("/api/me/api-keys/{}", last["id"].as_str().unwrap()), &headers, None),
    )
    .await;
    assert!(status.is_success(), "{body}");
    let (status, body) =
        send(&app, request("POST", "/api/me/api-keys", &headers, Some(json!({ "label": "churned" })))).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");

    // Another account is not held back by this one.
    let other = db.user("bystander", false).await;
    create_key(&app, &signed_in(&db, other), "first").await;
}

/// A-ACCOUNT-7: a key's label is at most a hundred characters. Unbounded, a
/// burst of keys with 2 MB labels was 200 MB an account, listed on every
/// view of the account page and kept in every dump.
#[tokio::test]
async fn a_key_label_is_bounded() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let headers = signed_in(&db, db.user("labeller", false).await);
    let (status, body) = send(
        &app,
        request("POST", "/api/me/api-keys", &headers, Some(json!({ "label": "x".repeat(101) }))),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["fields"][0]["field"], "label", "{body}");
    create_key(&app, &headers, &"x".repeat(100)).await;
}

/// A-ACCOUNT-2: creating a key returns it in full once -- the key that
/// actually authenticates -- and the list never returns it, or its hash,
/// again.
#[tokio::test]
async fn a_new_api_key_is_shown_once_and_never_listed() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let user = db.user("keymaker", false).await;
    let headers = signed_in(&db, user);

    let created = create_key(&app, &headers, "laptop").await;
    assert_eq!(object_keys(&created), BTreeSet::from(["id", "label", "key"]));
    assert_eq!(created["label"], "laptop");
    let raw = created["key"].as_str().unwrap().to_string();
    assert!(raw.starts_with("bt_") && raw.len() == 3 + 64, "{raw}");
    let stored: String = sqlx::query_scalar("SELECT key_hash FROM api_keys WHERE id = $1::uuid")
        .bind(created["id"].as_str().unwrap())
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(stored, birdtest::auth::api_key::hash_key(&raw), "only the hash is stored");
    assert_eq!(claim_with(&app, &raw).await.0, StatusCode::NO_CONTENT, "the key shown is the real one");

    let second = create_key(&app, &headers, "desktop").await;
    assert_ne!(second["key"], created["key"]);

    let listed = list_keys(&app, &headers).await;
    let items = listed.as_array().unwrap();
    assert_eq!(items.len(), 2);
    for item in items {
        assert_eq!(
            object_keys(item),
            BTreeSet::from(["id", "label", "is_active", "created_at", "last_used_at"]),
            "{item}"
        );
    }
    let ids: BTreeSet<&str> = items.iter().map(|i| i["id"].as_str().unwrap()).collect();
    assert_eq!(ids, BTreeSet::from([created["id"].as_str().unwrap(), second["id"].as_str().unwrap()]));
    let text = listed.to_string();
    for secret in [raw.as_str(), second["key"].as_str().unwrap(), stored.as_str()] {
        assert!(!text.contains(secret), "the list shows a key or its hash: {text}");
    }
}

/// A-ACCOUNT-4: a deactivated key stops authenticating, reactivating it
/// restores it, and a revoked key is gone for good -- it cannot be
/// reactivated.
#[tokio::test]
async fn deactivation_suspends_a_key_reactivation_restores_it_and_revocation_is_final() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let user = db.user("rotator", false).await;
    let headers = signed_in(&db, user);
    let key = create_key(&app, &headers, "worker").await;
    let raw = key["key"].as_str().unwrap();
    assert_eq!(claim_with(&app, raw).await.0, StatusCode::NO_CONTENT);

    let (status, body) = set_active(&app, &headers, &key, false).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (status, body) = claim_with(&app, raw).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(body["message"], "unknown or inactive API key");
    assert_eq!(list_keys(&app, &headers).await[0]["is_active"], false);

    let (status, body) = set_active(&app, &headers, &key, true).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert_eq!(claim_with(&app, raw).await.0, StatusCode::NO_CONTENT);

    let (status, body) = revoke(&app, &headers, &key).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (status, body) = claim_with(&app, raw).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(body["message"], "unknown or inactive API key");
    assert_eq!(list_keys(&app, &headers).await, json!([]));

    let (status, body) = set_active(&app, &headers, &key, true).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "a revoked key cannot be brought back: {body}");
    assert_eq!(body["message"], "no such API key");
    assert_eq!(claim_with(&app, raw).await.0, StatusCode::UNAUTHORIZED);
}

/// A-ACCOUNT-4b: the `Bearer` scheme is matched without regard to case (RFC
/// 7235). `bearer <key>` was read as no credential: a deactivated key's claim
/// minted an anonymous identity and was answered 204, and a live key's work
/// was credited to nobody.
#[tokio::test]
async fn the_bearer_scheme_is_read_in_any_case() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let user = db.user("lowercase", false).await;
    let headers = signed_in(&db, user);
    let key = create_key(&app, &headers, "worker").await;
    let raw = key["key"].as_str().unwrap();
    let claim = |scheme: &'static str| {
        let app = app.clone();
        let bearer = format!("{scheme} {raw}");
        async move {
            send(&app, post_json("/api/worker/task", &[("authorization", &bearer)], claim_body("1.0.0", &[])))
                .await
        }
    };

    // A live key in lower case is the key: looked up, and its use recorded.
    let (status, body) = claim("bearer").await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let used: bool = sqlx::query_scalar("SELECT last_used_at IS NOT NULL FROM api_keys WHERE user_id = $1")
        .bind(user)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert!(used, "a key sent as `bearer` was not looked up");

    let (status, body) = set_active(&app, &headers, &key, false).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    for scheme in ["bearer", "BEARER"] {
        let (status, body) = claim(scheme).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{scheme}: {body}");
        assert_eq!(body["message"], "unknown or inactive API key", "{scheme}");
    }
    let minted: i64 = sqlx::query_scalar("SELECT count(*) FROM anonymous_workers")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(minted, 0, "a key in another case minted an anonymous identity");
}

/// A-ACCOUNT-5: one account cannot list, deactivate or revoke another's keys;
/// each attempt answers as if the key did not exist, and the key is untouched.
#[tokio::test]
async fn one_user_cannot_see_or_change_anothers_keys() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let owner = db.user("owner", false).await;
    let intruder = db.user("intruder", false).await;
    let owner_headers = signed_in(&db, owner);
    let intruder_headers = signed_in(&db, intruder);

    let key = create_key(&app, &owner_headers, "mine").await;
    let intruders_own = create_key(&app, &intruder_headers, "theirs").await;

    let listed = list_keys(&app, &intruder_headers).await;
    let ids: Vec<&str> = listed.as_array().unwrap().iter().map(|k| k["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec![intruders_own["id"].as_str().unwrap()], "{listed}");

    let (status, body) = set_active(&app, &intruder_headers, &key, false).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["message"], "no such API key");
    let (status, body) = revoke(&app, &intruder_headers, &key).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["message"], "no such API key");

    let mine = list_keys(&app, &owner_headers).await;
    assert_eq!(mine.as_array().unwrap().len(), 1);
    assert_eq!(mine[0]["id"], key["id"]);
    assert_eq!(mine[0]["is_active"], true);
    assert_eq!(claim_with(&app, key["key"].as_str().unwrap()).await.0, StatusCode::NO_CONTENT);
}

/// `scripts/scrub.sql` as `dev-restore.sh` applies it (with `-v dev_copy=1`):
/// psql's own commands left out, and with them the refusal psql runs only
/// when that variable is missing.
async fn scrub(db: &TestDb) {
    let script = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../scripts/scrub.sql")).unwrap();
    let mut refusal = false;
    let sql: String = script
        .lines()
        .filter(|line| {
            if line.starts_with("\\else") {
                refusal = true;
            } else if line.starts_with("\\endif") {
                refusal = false;
            }
            !refusal && !line.starts_with('\\')
        })
        .collect::<Vec<_>>()
        .join("\n");
    sqlx::raw_sql(&sql).execute(&db.pool).await.unwrap();
}

/// S-SCRUB-1: a scrubbed dump holds no credential. An anonymous worker's UUID
/// is one -- `X-Worker-UUID` alone authenticates it -- and scrubbing left every
/// one in place, so a laptop dump or a published sample could submit as any
/// anonymous contributor (thirty-first audit). Each is replaced, and its
/// claims, ban and audit rows follow it; an open claim's token is replaced
/// too. Run twice, as the script promises it may be.
#[tokio::test]
async fn a_scrubbed_dump_keeps_no_worker_credential() {
    let db = TestDb::new().await;
    let admin = db.user("root", true).await;
    let job = db.games_job(10).await;
    let worker = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO anonymous_workers (uuid, tasks_completed, compute_ms, movegens)
         VALUES ($1, 7, 8000, 9)",
    )
        .bind(worker)
        .execute(&db.pool)
        .await
        .unwrap();
    let task: Uuid = sqlx::query_scalar("INSERT INTO tasks (job_id, seed) VALUES ($1, 1) RETURNING id")
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let token = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO task_claims (task_id, job_id, claim_token, claimed_by_anon_uuid, state)
         VALUES ($1, $2, $3, $4, 'claimed')",
    )
    .bind(task)
    .bind(job)
    .bind(token)
    .bind(worker)
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO worker_bans (anon_uuid, banned_by, reason) VALUES ($1, $2, 'x')")
        .bind(worker)
        .bind(admin)
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO audit_log (action, actor_user_id, actor_anon_uuid, target_type, target_id, reason)
         VALUES ('worker.banned', $1, $2, 'worker', $3, 'Bob at 203.0.113.7')",
    )
    .bind(admin)
    .bind(worker)
    .bind(worker.to_string())
    .execute(&db.pool)
    .await
    .unwrap();

    for _ in 0..2 {
        scrub(&db).await;
    }

    let (uuid, tasks_completed, compute_ms, movegens): (Uuid, i64, i64, i64) = sqlx::query_as(
        "SELECT uuid, tasks_completed, compute_ms, movegens FROM anonymous_workers",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_ne!(uuid, worker, "the credential is gone");
    assert_eq!((tasks_completed, compute_ms, movegens), (7, 8000, 9), "what it did is kept");
    let (claimed_by, claim_token): (Uuid, Uuid) =
        sqlx::query_as("SELECT claimed_by_anon_uuid, claim_token FROM task_claims").fetch_one(&db.pool).await.unwrap();
    assert_eq!(claimed_by, uuid, "its claim follows it");
    assert_ne!(claim_token, token, "an open claim's token is replaced");
    let banned: Uuid = sqlx::query_scalar("SELECT anon_uuid FROM worker_bans").fetch_one(&db.pool).await.unwrap();
    assert_eq!(banned, uuid, "so does its ban");
    // A ban's reason is an admin's free text about a person: blanked, in the
    // ban and in its audit row (the audit's pass 23).
    let reasons: Vec<Option<String>> = sqlx::query_scalar(
        "SELECT reason FROM worker_bans UNION ALL
         SELECT reason FROM audit_log WHERE action = 'worker.banned' AND reason IS NOT NULL",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(reasons.len(), 2, "the ban and its audit row: {reasons:?}");
    assert!(reasons.iter().all(|r| r.as_deref() == Some("[scrubbed]")), "{reasons:?}");
    let (actor, target): (Uuid, String) =
        sqlx::query_as("SELECT actor_anon_uuid, target_id FROM audit_log").fetch_one(&db.pool).await.unwrap();
    assert_eq!((actor, target), (uuid, uuid.to_string()), "and its audit rows");

    // The script copies each identity's row column by column: a column added
    // to the table must be added there too, or scrubbing resets it.
    let columns: Vec<String> = sqlx::query_scalar(
        "SELECT column_name::text FROM information_schema.columns
         WHERE table_name = 'anonymous_workers' ORDER BY ordinal_position",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        columns,
        [
            "uuid", "first_seen_at", "last_seen_at", "tasks_completed", "compute_ms",
            "movegens", "last_completed_at",
        ]
    );
}

/// A-ACCOUNT-8: a key's whole life is on record. Issuing, suspending, resuming
/// and revoking a key wrote no audit row, and a revoked key's row is deleted,
/// so nothing said afterwards that it had existed -- the trail a takeover
/// leaves, and what a restore undoes (the audit's pass 22). Each writes one,
/// by the key's id -- not its label, the owner's free text, which would
/// outlive the account's deletion.
#[tokio::test]
async fn a_keys_life_is_on_record() {
    let db = TestDb::new().await;
    let user = db.user("keyholder", false).await;
    let headers = signed_in(&db, user);
    let app = birdtest::app(db.state().await);
    let key = create_key(&app, &headers, "laptop").await;
    // A request that changes nothing is answered and not logged: a row per
    // call let one account grow the log at will (A-ACCOUNT-9 limits changes).
    assert_eq!(set_active(&app, &headers, &key, true).await.0, StatusCode::NO_CONTENT);
    assert_eq!(set_active(&app, &headers, &key, false).await.0, StatusCode::NO_CONTENT);
    assert_eq!(set_active(&app, &headers, &key, false).await.0, StatusCode::NO_CONTENT);
    assert_eq!(set_active(&app, &headers, &key, true).await.0, StatusCode::NO_CONTENT);
    assert_eq!(revoke(&app, &headers, &key).await.0, StatusCode::NO_CONTENT);
    let rows: Vec<(String, Option<Uuid>, String, Option<String>)> = sqlx::query_as(
        "SELECT action, actor_user_id, target_type, reason FROM audit_log
         WHERE target_id = $1 ORDER BY id",
    )
    .bind(key["id"].as_str().unwrap())
    .fetch_all(&db.pool)
    .await
    .unwrap();
    let expected: Vec<(String, Option<Uuid>, String, Option<String>)> =
        ["api_key.created", "api_key.deactivated", "api_key.reactivated", "api_key.revoked"]
            .iter()
            .map(|a| (a.to_string(), Some(user), "api_key".to_string(), None))
            .collect();
    assert_eq!(rows, expected);
}

/// A-ACCOUNT-9: toggling a key is rate limited. Each change writes an audit
/// row, and a key toggled back and forth -- a change every time -- wrote 4,000
/// in ten seconds (the audit's pass 23). Resuming is limited (a burst of 100),
/// suspending and revoking are not: an owner suspending or revoking after a
/// takeover must not find the bucket drained by the thief.
#[tokio::test]
async fn key_changes_are_rate_limited_per_account() {
    let db = TestDb::new().await;
    let user = db.user("toggler", false).await;
    let headers = signed_in(&db, user);
    let app = birdtest::app(db.state().await);
    let key = create_key(&app, &headers, "laptop").await;
    let mut limited = 0;
    for i in 0..300 {
        let (status, _) = set_active(&app, &headers, &key, i % 2 == 1).await;
        if status == StatusCode::TOO_MANY_REQUESTS {
            limited += 1;
            assert!(i % 2 == 1, "a suspend was refused at {i}");
        } else {
            assert_eq!(status, StatusCode::NO_CONTENT);
        }
    }
    assert_eq!(limited, 50, "the burst is 100 resumes");
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log WHERE action LIKE 'api_key.%activated'")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(rows, 201, "a suspend and a resume each for 100 resumes, and the last suspend");
    // The owner can still suspend and revoke.
    assert_eq!(set_active(&app, &headers, &key, false).await.0, StatusCode::NO_CONTENT);
    assert_eq!(revoke(&app, &headers, &key).await.0, StatusCode::NO_CONTENT);
}
