//! Sessions against a real database: revocation through
//! `users.session_generation`.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::*;

/// Bug: sessions were stateless and outlived a password reset for up to their
/// full TTL. Every token now carries the account's session generation.
#[tokio::test]
async fn bumping_the_session_generation_revokes_earlier_sessions() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let user = db.user("someone", false).await;
    let headers = admin_headers(&state.cfg, user);

    let (status, _) = send(&app, get_request("/api/me", &headers)).await;
    assert_eq!(status, StatusCode::OK);

    // What a password reset does.
    sqlx::query("UPDATE users SET session_generation = session_generation + 1 WHERE id = $1")
        .bind(user)
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, _) = send(&app, get_request("/api/me", &headers)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn signing_out_everywhere_revokes_the_callers_own_session_too() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let user = db.user("someone", false).await;
    let headers = admin_headers(&state.cfg, user);

    let mut builder = Request::post("/api/auth/sign-out-everywhere");
    for (name, value) in &headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    let (status, body) = send(&app, builder.body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    let (status, _) = send(&app, get_request("/api/me", &headers)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // A session minted at the new generation works.
    let fresh = birdtest::auth::session::issue(&state.cfg, user, "someone", false, 1).unwrap();
    let (status, _) = send(
        &app,
        get_request("/api/me", &[("cookie".into(), format!("birdtest_session={fresh}"))]),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

/// Bug: `num_plays_recorded` was optional, and "unset" meant 10 moves to MAGPIE
/// but "everything" to the server. It is now required and at least 1.
#[tokio::test]
async fn a_player_config_must_say_how_many_plays_to_report() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let admin = db.user("root", true).await;
    let kwg = db.input_data("kwg", "NWL23").await;
    let klv = db.input_data("klv", "NWL23").await;
    let headers = admin_headers(&state.cfg, admin);
    let headers: Vec<(&str, &str)> = headers.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect();

    let body = |plays: serde_json::Value| {
        let mut body = serde_json::json!({
            "name": format!("p{}", uuid::Uuid::new_v4().simple()),
            "recorder_type": "best", "sort_strategy": "equity",
            "kwg_id": kwg, "klv_id": klv,
        });
        if !plays.is_null() {
            body["num_plays_recorded"] = plays;
        }
        body
    };
    let (status, _) =
        send(&app, post_json("/api/admin/player-configs", &headers, body(serde_json::Value::Null))).await;
    assert!(status.is_client_error(), "omitted: {status}");
    let (status, _) = send(&app, post_json("/api/admin/player-configs", &headers, body(0.into()))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, created) =
        send(&app, post_json("/api/admin/player-configs", &headers, body(10.into()))).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["num_plays_recorded"], 10);
}

/// The account page reads the same running total the contributor lists do,
/// rather than counting the account's claims: the count walked every claim the
/// account ever made, and disagreed with `/api/users` after a purge had given
/// some of them back.
#[tokio::test]
async fn the_account_page_reads_the_contribution_counter() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let user = db.user("counted", false).await;
    sqlx::query("UPDATE users SET tasks_completed = 7 WHERE id = $1")
        .bind(user)
        .execute(&db.pool)
        .await
        .unwrap();

    let (status, body) = send(&app, get_request("/api/me", &admin_headers(&state.cfg, user))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["tasks_completed"], 7);
}

/// Race: the hundred-key limit was a `COUNT` and then an `INSERT`, two
/// statements with nothing between them, so requests arriving together each
/// read the same count and each inserted. The limit lives in the application,
/// not the schema, so that check is all there is.
#[tokio::test]
async fn concurrent_key_requests_cannot_exceed_the_key_limit() {
    let db = TestDb::new().await;
    let cfg = db.config();
    let user = db.user("keyhoarder", false).await;
    // Two short of the limit, written directly.
    sqlx::query(
        "INSERT INTO api_keys (user_id, key_hash)
         SELECT $1, 'seeded-' || g FROM generate_series(1, 98) g",
    )
    .bind(user)
    .execute(&db.pool)
    .await
    .unwrap();
    let app = birdtest::app(db.state().await);

    let headers = admin_headers(&cfg, user);
    let request = || {
        let mut builder = Request::post("/api/me/api-keys").header("content-type", "application/json");
        for (name, value) in &headers {
            builder = builder.header(name.as_str(), value.as_str());
        }
        builder.body(Body::from("{}")).unwrap()
    };
    // Ten: the most an account may create in a burst (A-ACCOUNT-6).
    let attempts = futures::future::join_all((0..10).map(|_| send(&app, request()))).await;

    let created = attempts.iter().filter(|(status, _)| *status == StatusCode::CREATED).count();
    let refused = attempts.iter().filter(|(status, _)| *status == StatusCode::CONFLICT).count();
    assert_eq!((created, refused), (2, 8), "{attempts:?}");
    let held: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM api_keys WHERE user_id = $1")
        .bind(user)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(held, 100);
}
