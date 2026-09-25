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

/// A-ACCOUNT-6: an account creates at most ten keys an hour. Each key is a
/// worker rate-limit bucket of its own, so unmetered creation was unmetered
/// submission; the hundred-key cap alone did not bound it, as revoking frees
/// a slot.
#[tokio::test]
async fn key_creation_is_rate_limited_per_account() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let user = db.user("churner", false).await;
    let headers = signed_in(&db, user);
    for n in 0..10 {
        create_key(&app, &headers, &format!("key {n}")).await;
    }
    let (status, body) =
        send(&app, request("POST", "/api/me/api-keys", &headers, Some(json!({ "label": "one more" })))).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");

    // Another account is not held back by this one.
    let other = db.user("bystander", false).await;
    create_key(&app, &signed_in(&db, other), "first").await;
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
