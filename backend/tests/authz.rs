//! Authorization applied to every route (TESTING.md, `A-AUTHZ-*`).
//!
//! The route table below is checked against the router's own source: the
//! `.route(...)` calls in `src/routes/*.rs`, placed under the prefixes
//! `src/lib.rs` nests them at. A route added to the router and not to the
//! table fails `the_route_table_is_every_route_the_router_serves`, so none
//! can be left out of the table-driven checks by forgetting it.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::*;
use serde_json::json;
use std::collections::BTreeSet;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// The router, read from its source
// ---------------------------------------------------------------------------

const LIB_RS: &str = include_str!("../src/lib.rs");

/// Every module `lib.rs` takes routes from. One added there and not here fails
/// the enumeration rather than being skipped.
const ROUTE_SOURCES: [(&str, &str); 6] = [
    ("account", include_str!("../src/routes/account.rs")),
    ("admin", include_str!("../src/routes/admin.rs")),
    ("auth", include_str!("../src/routes/auth.rs")),
    ("public", include_str!("../src/routes/public.rs")),
    ("ratings", include_str!("../src/routes/ratings.rs")),
    ("worker", include_str!("../src/routes/worker.rs")),
];

const METHODS: [&str; 7] = ["get", "post", "put", "patch", "delete", "head", "options"];

/// The body of `fn name` in `source`, without its comment lines.
fn fn_body(source: &str, name: &str) -> String {
    let start = source
        .find(&format!("fn {name}("))
        .unwrap_or_else(|| panic!("no fn {name} in the router source"));
    let open = start + source[start..].find('{').unwrap();
    let mut depth = 0;
    let mut end = open;
    for (i, c) in source[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = open + i;
                    break;
                }
            }
            _ => {}
        }
    }
    source[open + 1..end]
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The text between the parenthesis at `open` and its match.
fn parenthesized(text: &str, open: usize) -> &str {
    let mut depth = 0;
    for (i, c) in text[open..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return &text[open + 1..open + i];
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced parentheses in the router source");
}

fn first_string_literal(text: &str) -> &str {
    let start = text.find('"').expect("a route path") + 1;
    let len = text[start..].find('"').unwrap();
    &text[start..start + len]
}

/// `(METHOD, path)` for every `.route(...)` in a router function's body.
fn routes_in(body: &str, prefix: &str) -> Vec<(String, String)> {
    for forbidden in [".nest(", ".merge(", ".route_service(", ".fallback(", ".nest_service("] {
        assert!(
            !body.contains(forbidden),
            "a router function uses {forbidden}; teach this parser about it so its routes are \
             enumerated too"
        );
    }
    let mut routes = Vec::new();
    for (at, _) in body.match_indices(".route(") {
        let args = parenthesized(body, at + ".route".len());
        let path = first_string_literal(args);
        let handlers = &args[args.find(path).unwrap() + path.len() + 1..];
        let mut found = 0;
        for method in METHODS {
            for (i, _) in handlers.match_indices(&format!("{method}(")) {
                let before = handlers[..i].chars().next_back();
                if before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':') {
                    continue;
                }
                routes.push((method.to_uppercase(), format!("{prefix}{path}")));
                found += 1;
            }
        }
        assert!(found > 0, "no method found for route {path:?}: {args}");
    }
    routes
}

/// Every `(METHOD, path)` the application router serves, read from source.
fn served_routes() -> BTreeSet<(String, String)> {
    let app = fn_body(LIB_RS, "app");
    let mut routes: Vec<(String, String)> = Vec::new();
    // Routes declared on the application router itself (`/health`).
    let own: String = app
        .lines()
        .filter(|line| line.contains(".route("))
        .collect::<Vec<_>>()
        .join("\n");
    routes.extend(routes_in(&own, ""));

    let mut mounted = 0;
    for line in app.lines().filter(|l| l.contains("routes::")) {
        let (prefix, call) = if line.contains(".nest(") {
            let prefix = first_string_literal(line).to_string();
            (prefix, &line[line.find("routes::").unwrap()..])
        } else {
            assert!(line.contains(".merge("), "unrecognized router mount: {line}");
            (String::new(), &line[line.find("routes::").unwrap()..])
        };
        let call = call.trim_start_matches("routes::");
        let (module, rest) = call.split_once("::").unwrap();
        let function = &rest[..rest.find('(').unwrap()];
        let source = ROUTE_SOURCES
            .iter()
            .find(|(name, _)| *name == module)
            .unwrap_or_else(|| panic!("lib.rs mounts routes::{module}; add it to ROUTE_SOURCES"))
            .1;
        routes.extend(routes_in(&fn_body(source, function), &prefix));
        mounted += 1;
    }
    assert!(mounted >= 7, "found only {mounted} router mounts in lib.rs; did its shape change?");
    let set: BTreeSet<_> = routes.iter().cloned().collect();
    assert_eq!(set.len(), routes.len(), "a route is declared twice: {routes:?}");
    set
}

// ---------------------------------------------------------------------------
// The table
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Access {
    /// Takes `AdminUser`.
    Admin,
    /// Takes `CurrentUser`.
    Session,
    /// Reads the session cookies without requiring a session (logout).
    Cookie,
    /// The unauthenticated half of the Auth API: there is no session yet, so
    /// nothing a browser attaches on its own authorizes these.
    PreSession,
    /// `/api/worker/*`, authenticated by `WorkerIdentity`.
    Worker,
    /// `/api/worker/client-version`: public, no identity.
    WorkerOpen,
    /// Public reads.
    Public,
}
use Access::*;

const ID: &str = "00000000-0000-4000-8000-000000000001";

/// `(method, path, access, body)`. The body is one the route's extractor
/// accepts, so that a rejection is the authorization check's and not the JSON
/// parser's; `""` for a route that reads none.
const ROUTES: &[(&str, &str, Access, &str)] = &[
    ("GET", "/health", Public, ""),
    // --- Admin API ----------------------------------------------------------
    ("GET", "/api/admin/player-configs", Admin, ""),
    (
        "POST",
        "/api/admin/player-configs",
        Admin,
        r#"{"name":"p","recorder_type":"best","kwg_id":"00000000-0000-4000-8000-000000000001","klv_id":"00000000-0000-4000-8000-000000000001","num_plays_recorded":10}"#,
    ),
    ("GET", "/api/admin/player-configs/:id", Admin, ""),
    ("DELETE", "/api/admin/player-configs/:id", Admin, ""),
    (
        "POST",
        "/api/admin/jobs",
        Admin,
        r#"{"job_type":"opening_rack","variant":"classic","letterdist_id":"00000000-0000-4000-8000-000000000001","layout_id":"00000000-0000-4000-8000-000000000001","player_config_id":"00000000-0000-4000-8000-000000000001"}"#,
    ),
    ("POST", "/api/admin/jobs/:id/activate", Admin, r#"{"allocation":50}"#),
    ("POST", "/api/admin/jobs/:id/deactivate", Admin, ""),
    ("POST", "/api/admin/jobs/:id/complete", Admin, ""),
    ("POST", "/api/admin/jobs/:id/purge", Admin, ""),
    ("DELETE", "/api/admin/jobs/:id", Admin, ""),
    ("DELETE", "/api/admin/users/:id", Admin, ""),
    ("GET", "/api/admin/workers", Admin, ""),
    ("POST", "/api/admin/workers/ban", Admin, r#"{"user_id":"00000000-0000-4000-8000-000000000001"}"#),
    ("DELETE", "/api/admin/workers/ban/:id", Admin, ""),
    ("GET", "/api/admin/audit-log", Admin, ""),
    ("GET", "/api/admin/input-data", Admin, ""),
    ("DELETE", "/api/admin/input-data/:id", Admin, ""),
    ("POST", "/api/admin/input-data/imports", Admin, r#"{"tarball_date":"20251004"}"#),
    ("GET", "/api/admin/input-data/imports/:id", Admin, ""),
    ("POST", "/api/admin/input-data/imports/:id/confirm", Admin, ""),
    ("GET", "/api/admin/jobs/:id/data-gaps", Admin, ""),
    ("GET", "/api/admin/jobs/:id/results/stream", Admin, ""),
    ("POST", "/api/admin/jobs/:id/export", Admin, ""),
    ("GET", "/api/admin/jobs/:id/export", Admin, ""),
    ("POST", "/api/admin/jobs/:id/rebuild-artifacts", Admin, ""),
    ("POST", "/api/admin/jobs/:id/merge-progress", Admin, ""),
    ("GET", "/api/admin/derived-data", Admin, ""),
    ("POST", "/api/admin/derived-data/retry", Admin, r#"{"role":"wmp","name":"NWL23"}"#),
    ("GET", "/api/admin/backups", Admin, ""),
    ("GET", "/api/admin/fleet", Admin, ""),
    (
        "POST",
        "/api/admin/rating-pools",
        Admin,
        r#"{"name":"pool","variant":"classic","letterdist_id":"00000000-0000-4000-8000-000000000001","layout_id":"00000000-0000-4000-8000-000000000001","anchor_player_config_id":"00000000-0000-4000-8000-000000000001"}"#,
    ),
    (
        "POST",
        "/api/admin/rating-pools/:id/members",
        Admin,
        r#"{"player_config_id":"00000000-0000-4000-8000-000000000001"}"#,
    ),
    ("DELETE", "/api/admin/rating-pools/:id/members/:config_id", Admin, ""),
    ("POST", "/api/admin/rating-pools/:id/recompute", Admin, ""),
    // --- Account API --------------------------------------------------------
    ("GET", "/api/me", Session, ""),
    ("GET", "/api/me/api-keys", Session, ""),
    ("POST", "/api/me/api-keys", Session, r#"{"label":"laptop"}"#),
    ("PATCH", "/api/me/api-keys/:id", Session, r#"{"is_active":false}"#),
    ("DELETE", "/api/me/api-keys/:id", Session, ""),
    // --- Auth API -----------------------------------------------------------
    ("POST", "/api/auth/register", PreSession, ""),
    ("POST", "/api/auth/login", PreSession, ""),
    ("POST", "/api/auth/logout", Cookie, ""),
    ("POST", "/api/auth/sign-out-everywhere", Session, ""),
    ("POST", "/api/auth/confirm-email", PreSession, ""),
    ("POST", "/api/auth/reset-password/request", PreSession, ""),
    ("POST", "/api/auth/reset-password/confirm", PreSession, ""),
    // --- Worker API ---------------------------------------------------------
    ("GET", "/api/worker/client-version", WorkerOpen, ""),
    ("POST", "/api/worker/task", Worker, r#"{"magpie_version":"1.0.0","unsupported_jobs":[]}"#),
    (
        "POST",
        "/api/worker/decline",
        Worker,
        r#"{"claim_token":"00000000-0000-4000-8000-000000000001","reason":"task_failed"}"#,
    ),
    ("POST", "/api/worker/heartbeat", Worker, r#"{"claim_token":"00000000-0000-4000-8000-000000000001"}"#),
    (
        "POST",
        "/api/worker/result",
        Worker,
        r#"{"claim_token":"00000000-0000-4000-8000-000000000001","result":{}}"#,
    ),
    ("GET", "/api/worker/artifact", Worker, ""),
    // --- Public API ---------------------------------------------------------
    ("GET", "/api/jobs", Public, ""),
    ("GET", "/api/jobs/:id", Public, ""),
    ("GET", "/api/jobs/:id/results", Public, ""),
    ("GET", "/api/jobs/:id/stream", Public, ""),
    ("GET", "/api/users", Public, ""),
    ("GET", "/api/workers", Public, ""),
    ("GET", "/api/rating-pools", Public, ""),
    ("GET", "/api/rating-pools/:id", Public, ""),
    ("GET", "/api/rating-pools/:id/history", Public, ""),
];

fn routes_with(access: &[Access]) -> impl Iterator<Item = &'static (&'static str, &'static str, Access, &'static str)> + '_ {
    ROUTES.iter().filter(move |(_, _, a, _)| access.contains(a))
}

fn is_mutating(method: &str) -> bool {
    !matches!(method, "GET" | "HEAD" | "OPTIONS")
}

/// `path` with every `:param` filled in with a well-formed id.
fn concrete(path: &str) -> String {
    path.split('/')
        .map(|segment| if segment.starts_with(':') { ID } else { segment })
        .collect::<Vec<_>>()
        .join("/")
}

fn request(method: &str, path: &str, headers: &[(String, String)], body: &str) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(concrete(path));
    for (name, value) in headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    if body.is_empty() {
        builder.body(Body::empty()).unwrap()
    } else {
        builder.header("content-type", "application/json").body(Body::from(body.to_string())).unwrap()
    }
}

/// A session cookie for `user` with the CSRF pair set as the test asks:
/// `cookie` is the CSRF cookie's value and `header` the header's, either
/// absent.
fn session(
    cfg: &birdtest::config::Config,
    user: Uuid,
    cookie: Option<&str>,
    header: Option<&str>,
) -> Vec<(String, String)> {
    let token = birdtest::auth::session::issue(cfg, user, "someone", false, 0).unwrap();
    let mut cookies = format!("birdtest_session={token}");
    if let Some(csrf) = cookie {
        cookies.push_str(&format!("; birdtest_csrf={csrf}"));
    }
    let mut headers = vec![("cookie".to_string(), cookies)];
    if let Some(csrf) = header {
        headers.push(("x-csrf-token".to_string(), csrf.to_string()));
    }
    headers
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// A-AUTHZ-1..4 (the enumeration they rest on): the table is exactly the
/// routes the router serves, so a route added without a row fails here instead
/// of escaping every check below. Admin routes are all `Admin`, account routes
/// all `Session`, and nothing public writes.
#[test]
fn the_route_table_is_every_route_the_router_serves() {
    let served = served_routes();
    let table: BTreeSet<(String, String)> =
        ROUTES.iter().map(|(m, p, _, _)| (m.to_string(), p.to_string())).collect();
    assert_eq!(table.len(), ROUTES.len(), "a route is in the table twice");
    let missing: Vec<_> = served.difference(&table).collect();
    let stale: Vec<_> = table.difference(&served).collect();
    assert!(
        missing.is_empty() && stale.is_empty(),
        "routes served but not in the table: {missing:?}\nin the table but not served: {stale:?}"
    );

    for (method, path, access, _) in ROUTES {
        if path.starts_with("/api/admin/") {
            assert_eq!(*access, Admin, "{method} {path}");
        }
        if *path == "/api/me" || path.starts_with("/api/me/") {
            assert_eq!(*access, Session, "{method} {path}");
        }
        if path.starts_with("/api/worker/") {
            assert!(matches!(access, Worker | WorkerOpen), "{method} {path}");
        }
        if is_mutating(method) {
            assert_ne!(*access, Public, "a public route that writes: {method} {path}");
        }
    }
}

/// A-AUTHZ-1: every Admin API route refuses a signed-in non-admin with 403,
/// before anything else about the request is looked at.
#[tokio::test]
async fn every_admin_route_refuses_a_signed_in_non_admin() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let user = db.user("ordinary", false).await;
    // A valid CSRF pair, so that the only thing wrong is the missing privilege.
    let headers = session(&state.cfg, user, Some("tok"), Some("tok"));

    let mut checked = 0;
    for (method, path, _, body) in routes_with(&[Admin]) {
        let (status, response) = send(&app, request(method, path, &headers, body)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}: {response}");
        assert_eq!(response["message"], "admin privileges required", "{method} {path}");
        checked += 1;
    }
    assert!(checked >= 34, "{checked}");
}

/// A-AUTHZ-2: every Admin API and Account API route, and every other route
/// that takes a session, answers an anonymous caller 401.
#[tokio::test]
async fn every_session_route_refuses_an_anonymous_caller() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);

    for (method, path, _, body) in routes_with(&[Admin, Session]) {
        let (status, response) = send(&app, request(method, path, &[], body)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {path}: {response}");
        assert_eq!(response["code"], "unauthorized", "{method} {path}");
        assert_eq!(response["message"], "not signed in", "{method} {path}");
    }
}

/// A-AUTHZ-3: every route that writes on the strength of the session cookie
/// refuses a request without the double-submit token -- no header, a header
/// that does not match the cookie, and a header with no cookie to match --
/// even from a signed-in admin with a well-formed body.
#[tokio::test]
async fn every_cookie_backed_write_requires_the_csrf_pair() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let admin = db.user("root", true).await;

    let cases = [
        ("no header", session(&state.cfg, admin, Some("tok"), None), "missing CSRF header"),
        ("mismatch", session(&state.cfg, admin, Some("tok"), Some("forged")), "CSRF token mismatch"),
        ("no cookie", session(&state.cfg, admin, None, Some("tok")), "missing CSRF cookie"),
    ];
    let mut checked = 0;
    for (method, path, _, body) in routes_with(&[Admin, Session, Cookie]) {
        if !is_mutating(method) {
            continue;
        }
        for (case, headers, message) in &cases {
            let (status, response) = send(&app, request(method, path, headers, body)).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path} ({case}): {response}");
            assert_eq!(response["message"], *message, "{method} {path} ({case})");
        }
        checked += 1;
    }
    // 22 admin writes, 3 account writes, logout and sign-out-everywhere.
    assert_eq!(checked, 27);

    // Nothing was done under any of them: the admin is still signed in, and
    // the audit log is empty.
    let (status, _) =
        send(&app, get_request("/api/me", &session(&state.cfg, admin, None, None))).await;
    assert_eq!(status, StatusCode::OK);
    let audited: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(audited, 0);
}

/// A-AUTHZ-4: the Worker API is exempt from CSRF by design. A worker request
/// that also carries a browser's session and CSRF cookies, and no CSRF header,
/// is served normally by every worker endpoint that writes.
#[tokio::test]
async fn worker_writes_need_no_csrf_token_even_alongside_session_cookies() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    db.games_job(1, 2).await;
    let user = db.user("contributor", false).await;
    let raw_key = birdtest::auth::api_key::generate_raw_key();
    sqlx::query("INSERT INTO api_keys (user_id, key_hash) VALUES ($1, $2)")
        .bind(user)
        .bind(birdtest::auth::api_key::hash_key(&raw_key))
        .execute(&db.pool)
        .await
        .unwrap();

    let mut headers = session(&state.cfg, user, Some("tok"), None);
    headers.push(("authorization".into(), format!("Bearer {raw_key}")));
    let post = |path: &str, body: serde_json::Value| request("POST", path, &headers, &body.to_string());

    let (status, first) = send(&app, post("/api/worker/task", claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let token = first["claim_token"].clone();

    let mut covered = BTreeSet::new();
    for (method, path, _, _) in routes_with(&[Worker]) {
        if !is_mutating(method) {
            continue;
        }
        let (status, body) = match *path {
            "/api/worker/task" => (status, first.clone()),
            "/api/worker/heartbeat" => send(&app, post(path, json!({ "claim_token": token }))).await,
            "/api/worker/result" => {
                send(&app, post(path, json!({ "claim_token": token, "result": games_result(2, 1) })))
                    .await
            }
            "/api/worker/decline" => {
                let (status, next) =
                    send(&app, post("/api/worker/task", claim_body("1.0.0", &[]))).await;
                assert_eq!(status, StatusCode::OK, "{next}");
                send(
                    &app,
                    post(path, json!({ "claim_token": next["claim_token"], "reason": "task_failed" })),
                )
                .await
            }
            other => panic!("a new worker write, {other}: add it to this test"),
        };
        assert!(status.is_success(), "{method} {path}: {status} {body}");
        covered.insert(*path);
    }
    assert_eq!(covered.len(), 4);
    let accepted: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM task_claims WHERE state = 'completed'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(accepted, 1, "the result submitted without a CSRF token was accepted");
}

/// A-AUTHZ-5: admin status is read from the account on every request, so a
/// demoted admin loses the Admin API at once, with the same session; a deleted
/// account's session stops working altogether, whether it was deleted through
/// the Admin API or only marked deleted.
#[tokio::test]
async fn a_demoted_or_deleted_account_loses_access_without_its_session_expiring() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());

    let admin = db.user("formeradmin", true).await;
    let headers = session(&state.cfg, admin, None, None);
    let (status, body) = send(&app, get_request("/api/admin/fleet", &headers)).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    sqlx::query("UPDATE users SET is_admin = false WHERE id = $1")
        .bind(admin)
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, body) = send(&app, get_request("/api/admin/fleet", &headers)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["message"], "admin privileges required");
    let (status, body) = send(&app, get_request("/api/me", &headers)).await;
    assert_eq!(status, StatusCode::OK, "still signed in, as a non-admin: {body}");
    assert_eq!(body["is_admin"], false);

    // Marked deleted, and nothing else: the check is on `deleted_at` itself.
    let marked = db.user("markeddeleted", false).await;
    let marked_headers = session(&state.cfg, marked, None, None);
    assert_eq!(send(&app, get_request("/api/me", &marked_headers)).await.0, StatusCode::OK);
    sqlx::query("UPDATE users SET deleted_at = now() WHERE id = $1")
        .bind(marked)
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, body) = send(&app, get_request("/api/me", &marked_headers)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(body["message"], "session is no longer valid; sign in again");

    // Deleted through the Admin API.
    let root = db.user("root", true).await;
    let deleted = db.user("deleted", false).await;
    let deleted_headers = session(&state.cfg, deleted, None, None);
    assert_eq!(send(&app, get_request("/api/me", &deleted_headers)).await.0, StatusCode::OK);
    let (status, body) = send(
        &app,
        request("DELETE", &format!("/api/admin/users/{deleted}"), &admin_headers(&state.cfg, root), ""),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (status, body) = send(&app, get_request("/api/me", &deleted_headers)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(body["message"], "session is no longer valid; sign in again");
}

/// A-AUTHZ-6: a deactivated key, and a revoked one, are refused by every
/// worker endpoint that takes an identity -- with 401, not served as if the
/// request had no identity at all.
#[tokio::test]
async fn a_deactivated_or_revoked_api_key_is_refused_by_every_worker_endpoint() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let user = db.user("keyholder", false).await;
    let raw_key = birdtest::auth::api_key::generate_raw_key();
    sqlx::query("INSERT INTO api_keys (user_id, key_hash) VALUES ($1, $2)")
        .bind(user)
        .bind(birdtest::auth::api_key::hash_key(&raw_key))
        .execute(&db.pool)
        .await
        .unwrap();
    let bearer = vec![("authorization".to_string(), format!("Bearer {raw_key}"))];

    // Works while active.
    let (status, body) = send(
        &app,
        request("POST", "/api/worker/task", &bearer, &claim_body("1.0.0", &[]).to_string()),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "an active key is served (nothing to do): {body}");

    let refused_everywhere = |label: &'static str| {
        let app = app.clone();
        let bearer = bearer.clone();
        async move {
            let mut checked = 0;
            for (method, path, _, body) in routes_with(&[Worker]) {
                let (status, response) = send(&app, request(method, path, &bearer, body)).await;
                assert_eq!(status, StatusCode::UNAUTHORIZED, "{label}: {method} {path}: {response}");
                assert_eq!(response["message"], "unknown or inactive API key", "{label}: {path}");
                checked += 1;
            }
            assert_eq!(checked, 5);
        }
    };

    sqlx::query("UPDATE api_keys SET is_active = false WHERE user_id = $1")
        .bind(user)
        .execute(&db.pool)
        .await
        .unwrap();
    refused_everywhere("deactivated").await;

    sqlx::query("DELETE FROM api_keys WHERE user_id = $1").bind(user).execute(&db.pool).await.unwrap();
    refused_everywhere("revoked").await;
}
