//! The Auth API through the real router (TESTING.md, `A-AUTH-*`): registration,
//! confirmation, login, logout, password reset and their rate limits, reading
//! the mail the server sends from a file outbox.

mod common;

use axum::body::Body;
use axum::http::header::{RETRY_AFTER, SET_COOKIE};
use axum::http::{HeaderMap, Request, StatusCode};
use axum::Router;
use axum_extra::extract::cookie::Cookie;
use birdtest::config::MailBackend;
use birdtest::state::AppState;
use common::*;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;
use tower::ServiceExt;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A mail outbox directory of this test's own, removed when dropped.
struct Outbox(PathBuf);

impl Outbox {
    fn new() -> Self {
        Outbox(std::env::temp_dir().join(format!("birdtest-outbox-{}", Uuid::new_v4().simple())))
    }

    /// The file-name suffix the mailer gives messages to `to`.
    fn suffix(to: &str) -> String {
        let at = chrono::Utc::now();
        let name = birdtest::email::outbox_file_name(to, at);
        let stamp_len = birdtest::email::outbox_file_name("", at).len() - "-.txt".len();
        name[stamp_len..].to_string()
    }

    /// Every message to `to` so far, oldest first.
    fn messages_to(&self, to: &str) -> Vec<String> {
        let suffix = Self::suffix(to);
        let Ok(entries) = std::fs::read_dir(&self.0) else { return Vec::new() };
        let mut names: Vec<String> = entries
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .filter(|name| !name.starts_with('.') && name.ends_with(&suffix))
            .collect();
        names.sort();
        names.iter().map(|name| std::fs::read_to_string(self.0.join(name)).unwrap()).collect()
    }

    /// Waits for the `count`th message to `to`: mail may be sent off the
    /// request path, so it can land after the response.
    async fn wait_for(&self, to: &str, count: usize) -> Vec<String> {
        for _ in 0..500 {
            let messages = self.messages_to(to);
            if messages.len() >= count {
                return messages;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("no message #{count} to {to} within 10 s: {:?}", self.messages_to(to));
    }
}

impl Drop for Outbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A state that writes its mail to a fresh outbox and, with `hops` = 1,
/// takes the client address from `X-Forwarded-For`.
async fn mail_state(db: &TestDb, hops: usize) -> (AppState, Outbox) {
    let outbox = Outbox::new();
    let mut cfg = db.config();
    cfg.mail_backend = MailBackend::File;
    cfg.mail_outbox_dir = Some(outbox.0.clone());
    cfg.trusted_proxy_hops = hops;
    (db.state_with(cfg).await, outbox)
}

struct Response {
    status: StatusCode,
    headers: HeaderMap,
    bytes: Vec<u8>,
}

impl Response {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.bytes).unwrap_or(Value::Null)
    }
}

impl std::fmt::Debug for Response {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.status, String::from_utf8_lossy(&self.bytes))
    }
}

/// Like `common::send`, keeping the headers and the exact bytes.
async fn send_raw(app: &Router, request: Request<Body>) -> Response {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap().to_vec();
    Response { status, headers, bytes }
}

async fn post(app: &Router, path: &str, headers: &[(&str, &str)], body: Value) -> Response {
    send_raw(app, post_json(path, headers, body)).await
}

async fn register(app: &Router, username: &str, email: &str, password: &str, ip: &str) -> Response {
    post(
        app,
        "/api/auth/register",
        &[("x-forwarded-for", ip)],
        json!({ "username": username, "email": email, "password": password }),
    )
    .await
}

async fn login(app: &Router, username: &str, password: &str) -> Response {
    post(app, "/api/auth/login", &[], json!({ "username": username, "password": password })).await
}

const PASSWORD: &str = "vivid-otter-launches-quartz-72";
const NEW_PASSWORD: &str = "amber-kestrel-folds-lantern-19";

/// A confirmed account with a real password hash, email `<username>@example.invalid`.
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

/// The value of `param=` in the link a message carries.
fn link_param(message: &str, param: &str) -> String {
    let marker = format!("{param}=");
    let start = message.find(&marker).unwrap_or_else(|| panic!("no {marker} in {message}")) + marker.len();
    message[start..].split_whitespace().next().unwrap().to_string()
}

/// The cookies a browser would hold: keyed by name and path, set and removed
/// as RFC 6265 says, and sent to the paths they match. What matters here is
/// that a cookie is only replaced or removed by one with the same path, and
/// that a cookie with no `Path` gets the directory of the request that set it.
#[derive(Default)]
struct Browser {
    cookies: Vec<(String, String, String)>,
}

impl Browser {
    fn absorb(&mut self, request_path: &str, headers: &HeaderMap) {
        for value in headers.get_all(SET_COOKIE) {
            let cookie = Cookie::parse(value.to_str().unwrap().to_string()).unwrap();
            let path = match cookie.path() {
                Some(path) => path.to_string(),
                None => match request_path.rfind('/') {
                    Some(0) | None => "/".to_string(),
                    Some(i) => request_path[..i].to_string(),
                },
            };
            let name = cookie.name().to_string();
            self.cookies.retain(|(n, p, _)| !(*n == name && *p == path));
            let removed = cookie.max_age().is_some_and(|age| age.whole_seconds() <= 0);
            if !removed {
                self.cookies.push((name, path, cookie.value().to_string()));
            }
        }
    }

    fn sent_to(&self, request_path: &str) -> Vec<(String, String)> {
        self.cookies
            .iter()
            .filter(|(_, path, _)| {
                request_path == path
                    || (request_path.starts_with(path.as_str())
                        && (path.ends_with('/') || request_path[path.len()..].starts_with('/')))
            })
            .map(|(name, _, value)| (name.clone(), value.clone()))
            .collect()
    }

    fn get(&self, request_path: &str, name: &str) -> Option<String> {
        self.sent_to(request_path).into_iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    fn cookie_header(&self, request_path: &str) -> (String, String) {
        let pairs: Vec<String> =
            self.sent_to(request_path).iter().map(|(n, v)| format!("{n}={v}")).collect();
        ("cookie".to_string(), pairs.join("; "))
    }
}

fn set_cookies(response: &Response) -> Vec<String> {
    response.headers.get_all(SET_COOKIE).iter().map(|v| v.to_str().unwrap().to_string()).collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// A-AUTH-1: register, confirm with the code the confirmation mail carries,
/// sign in, and the session cookie that sets authenticates the account.
#[tokio::test]
async fn register_confirm_and_login_gives_a_working_session() {
    let db = TestDb::new().await;
    let (state, outbox) = mail_state(&db, 0).await;
    let app = birdtest::app(state);

    let response = register(&app, "newcomer", "Newcomer@Example.invalid ", PASSWORD, "").await;
    assert_eq!(response.status, StatusCode::CREATED, "{response:?}");
    assert_eq!(response.json()["message"], "check your email to confirm");

    // The address is stored trimmed and lowercased, and mailed there.
    let mail = outbox.wait_for("newcomer@example.invalid", 1).await;
    assert!(mail[0].contains("Subject: Confirm your birdtest account"), "{}", mail[0]);
    let code = link_param(&mail[0], "code");

    let response = post(&app, "/api/auth/confirm-email", &[], json!({ "code": code })).await;
    assert_eq!(response.status, StatusCode::OK, "{response:?}");
    assert_eq!(response.json()["message"], "email confirmed");

    let response = login(&app, "newcomer", PASSWORD).await;
    assert_eq!(response.status, StatusCode::OK, "{response:?}");
    assert_eq!(response.json(), json!({ "username": "newcomer", "is_admin": false }));
    let cookies = set_cookies(&response);
    let session = cookies
        .iter()
        .find(|c| c.starts_with("birdtest_session="))
        .unwrap_or_else(|| panic!("no session cookie: {cookies:?}"));
    for attribute in ["HttpOnly", "SameSite=Strict", "Path=/"] {
        assert!(session.contains(attribute), "{attribute} missing: {session}");
    }
    assert!(cookies.iter().any(|c| c.starts_with("birdtest_csrf=")), "{cookies:?}");

    let mut browser = Browser::default();
    browser.absorb("/api/auth/login", &response.headers);
    let (status, me) = send(&app, get_request("/api/me", &[browser.cookie_header("/api/me")])).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(me["username"], "newcomer");
    assert_eq!(me["email"], "newcomer@example.invalid");
}

/// A-AUTH-2: signing in before confirming the address is refused with 403, a
/// message that says what to do, and no session.
#[tokio::test]
async fn an_unconfirmed_login_is_refused_with_the_fix_named() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let user = confirmed_user(&db, "unconfirmed", PASSWORD).await;
    sqlx::query("UPDATE users SET email_confirmed_at = NULL WHERE id = $1")
        .bind(user)
        .execute(&db.pool)
        .await
        .unwrap();

    let response = login(&app, "unconfirmed", PASSWORD).await;
    assert_eq!(response.status, StatusCode::FORBIDDEN, "{response:?}");
    let body = response.json();
    assert_eq!(body["code"], "forbidden");
    let message = body["message"].as_str().unwrap();
    assert!(
        message.contains("confirm your email address") && message.contains("check your inbox"),
        "{message}"
    );
    assert!(set_cookies(&response).is_empty(), "no session for an unconfirmed account");
}

/// A-AUTH-3: a wrong password and an unknown username get the same 401 and
/// the same bytes, so login cannot be used to find out who has an account.
#[tokio::test]
async fn a_wrong_password_and_an_unknown_username_answer_identically() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    confirmed_user(&db, "existing", PASSWORD).await;

    let wrong_password = login(&app, "existing", "not-the-password-at-all-91").await;
    let unknown_user = login(&app, "nobodyatall", "not-the-password-at-all-91").await;
    assert_eq!(wrong_password.status, StatusCode::UNAUTHORIZED, "{wrong_password:?}");
    assert_eq!(unknown_user.status, wrong_password.status);
    assert_eq!(unknown_user.bytes, wrong_password.bytes, "{unknown_user:?} / {wrong_password:?}");
    assert_eq!(wrong_password.json()["message"], "incorrect username or password");
    assert!(set_cookies(&wrong_password).is_empty() && set_cookies(&unknown_user).is_empty());

    // And the account is real: its own password signs in.
    assert_eq!(login(&app, "existing", PASSWORD).await.status, StatusCode::OK);
}

/// A-AUTH-4: registering with an address that already has an account gets
/// byte-for-byte what a new registration gets. No account is created; the
/// address's owner is told someone tried.
#[tokio::test]
async fn registering_a_taken_address_answers_exactly_like_a_new_registration() {
    let db = TestDb::new().await;
    let (state, outbox) = mail_state(&db, 0).await;
    let app = birdtest::app(state);

    let fresh = register(&app, "firstowner", "owner@example.invalid", PASSWORD, "").await;
    assert_eq!(fresh.status, StatusCode::CREATED, "{fresh:?}");
    // The same address, differently cased and padded.
    let taken = register(&app, "secondcomer", " OWNER@example.invalid", PASSWORD, "").await;
    assert_eq!(taken.status, fresh.status, "{taken:?}");
    assert_eq!(taken.bytes, fresh.bytes, "{taken:?} / {fresh:?}");
    assert_eq!(taken.headers.get("content-type"), fresh.headers.get("content-type"));

    let accounts: Vec<String> = sqlx::query_scalar("SELECT username FROM users")
        .fetch_all(&db.pool)
        .await
        .unwrap();
    assert_eq!(accounts, vec!["firstowner".to_string()]);

    let mail = outbox.wait_for("owner@example.invalid", 2).await;
    let notice = mail
        .iter()
        .find(|m| m.contains("Subject: Someone tried to register with your email address"))
        .unwrap_or_else(|| panic!("no notice to the owner: {mail:?}"));
    assert!(!notice.contains("code="), "the notice carries no confirmation code: {notice}");
}

/// A-AUTH-4b: an account that never confirmed its address holds the address
/// and the username only while its confirmation link works. Registering over
/// it while it does is answered like any taken address, and the notice says
/// what is waiting; once the link has expired, the next registration takes
/// both. Held for ever, it was a dead end -- no sign-in, no reset, no new code
/// -- and a way to squat anyone's address.
#[tokio::test]
async fn an_expired_unconfirmed_account_gives_up_its_address_and_username() {
    let db = TestDb::new().await;
    let (state, outbox) = mail_state(&db, 0).await;
    let app = birdtest::app(state);

    let first = register(&app, "squatter", "owner@example.invalid", PASSWORD, "").await;
    assert_eq!(first.status, StatusCode::CREATED, "{first:?}");
    let early = register(&app, "realowner", "owner@example.invalid", PASSWORD, "").await;
    assert_eq!(early.bytes, first.bytes, "{early:?}");
    let usernames = || async {
        sqlx::query_scalar::<_, String>("SELECT username FROM users ORDER BY username")
            .fetch_all(&db.pool)
            .await
            .unwrap()
    };
    assert_eq!(usernames().await, vec!["squatter".to_string()]);
    let mail = outbox.wait_for("owner@example.invalid", 2).await;
    assert!(
        mail.iter().any(|m| m.contains("already waiting for the address to be confirmed")),
        "the notice says an account is waiting, not to sign in: {mail:?}"
    );

    sqlx::query("UPDATE email_confirmations SET expires_at = now() - interval '1 minute'")
        .execute(&db.pool)
        .await
        .unwrap();
    let late = register(&app, "realowner", "owner@example.invalid", PASSWORD, "").await;
    assert_eq!(late.status, StatusCode::CREATED, "{late:?}");
    assert_eq!(usernames().await, vec!["realowner".to_string()]);
    let mail = outbox.wait_for("owner@example.invalid", 3).await;
    let codes = mail.iter().filter(|m| m.contains("code=")).count();
    assert_eq!(codes, 2, "the new account is sent its own code: {mail:?}");

    // The username went with it.
    let name = register(&app, "squatter", "someone@example.invalid", PASSWORD, "").await;
    assert_eq!(name.status, StatusCode::CREATED, "{name:?}");
}

/// A-AUTH-4c: a username is taken whatever its case -- "Josh" and "josh" side
/// by side on a public list is an impersonation. And, being one name, it signs
/// in whatever its case.
#[tokio::test]
async fn a_username_is_taken_and_signs_in_whatever_its_case() {
    let db = TestDb::new().await;
    let (state, _outbox) = mail_state(&db, 0).await;
    let app = birdtest::app(state);
    let first = register(&app, "Josh", "josh@example.invalid", PASSWORD, "").await;
    assert_eq!(first.status, StatusCode::CREATED, "{first:?}");
    let second = register(&app, "josh", "other@example.invalid", PASSWORD, "").await;
    assert_eq!(second.status, StatusCode::CONFLICT, "{second:?}");
    assert_eq!(second.json()["fields"][0]["field"], "username", "{second:?}");

    sqlx::query("UPDATE users SET email_confirmed_at = now()")
        .execute(&db.pool)
        .await
        .unwrap();
    let response = login(&app, "josh", PASSWORD).await;
    assert_eq!(response.status, StatusCode::OK, "{response:?}");
    assert_eq!(response.json()["username"], "Josh", "the account's own spelling");
}

/// A-AUTH-4d: the auth routes take small bodies. A megabyte "username" was
/// accepted and became a megabyte key in the per-username login limiter, kept
/// until its sweep -- a few addresses could run the task out of memory.
#[tokio::test]
async fn an_oversized_auth_body_is_refused() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let response = login(&app, &"x".repeat(64 * 1024), PASSWORD).await;
    assert_eq!(response.status, StatusCode::PAYLOAD_TOO_LARGE, "{response:?}");
}

/// A-AUTH-5: a weak password is refused, and so is one derived from the
/// username or the email address -- each of which would score as strong
/// without them as context, so it is the context that refuses it.
#[tokio::test]
async fn registration_refuses_weak_passwords_and_ones_built_from_the_username_or_email() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);

    let cases = [
        ("weak", "someone", "someone@example.invalid", "password123"),
        ("username", "vexmorlandtriq", "v@example.invalid", "Vexmorlandtriq!"),
        ("email", "someone", "zulvanqorbitex@example.invalid", "zulvanqorbitex@example.invalid1"),
        ("email local part", "someone", "zulvanqorbitex@example.invalid", "zulvanqorbitex"),
    ];
    for (case, username, email, password) in cases {
        if case != "weak" {
            let alone = zxcvbn::zxcvbn(password, &[]).unwrap().score();
            assert!(alone >= 3, "{case}: {password:?} scores {alone} on its own; pick a stronger one");
        }
        let response = register(&app, username, email, password, "").await;
        assert_eq!(response.status, StatusCode::BAD_REQUEST, "{case}: {response:?}");
        let body = response.json();
        assert_eq!(
            body["fields"],
            json!([{ "field": "password", "message": "too weak — choose a longer, less predictable password" }]),
            "{case}: {body}"
        );
    }
    let accounts: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM users").fetch_one(&db.pool).await.unwrap();
    assert_eq!(accounts, 0);

    // The same username and address with an unrelated strong password pass.
    let response = register(&app, "vexmorlandtriq", "zulvanqorbitex@example.invalid", PASSWORD, "").await;
    assert_eq!(response.status, StatusCode::CREATED, "{response:?}");
}

/// Registers `username` and returns the code its confirmation mail carries.
async fn registered_code(app: &Router, outbox: &Outbox, username: &str) -> String {
    let email = format!("{username}@example.invalid");
    let response = register(app, username, &email, PASSWORD, "").await;
    assert_eq!(response.status, StatusCode::CREATED, "{response:?}");
    link_param(&outbox.wait_for(&email, 1).await[0], "code")
}

async fn confirmed(db: &TestDb, username: &str) -> bool {
    sqlx::query_scalar("SELECT email_confirmed_at IS NOT NULL FROM users WHERE username = $1")
        .bind(username)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

/// A-AUTH-6: a confirmation code confirms once; replaying it is refused.
#[tokio::test]
async fn a_confirmation_code_works_once() {
    let db = TestDb::new().await;
    let (state, outbox) = mail_state(&db, 0).await;
    let app = birdtest::app(state);
    let code = registered_code(&app, &outbox, "onceonly").await;

    let first = post(&app, "/api/auth/confirm-email", &[], json!({ "code": code })).await;
    assert_eq!(first.status, StatusCode::OK, "{first:?}");
    assert!(confirmed(&db, "onceonly").await);

    let replay = post(&app, "/api/auth/confirm-email", &[], json!({ "code": code })).await;
    assert_eq!(replay.status, StatusCode::BAD_REQUEST, "{replay:?}");
    assert_eq!(replay.json()["message"], "that confirmation link is invalid or has expired");
}

/// A-AUTH-7: an expired confirmation code is refused and leaves the account
/// unconfirmed.
#[tokio::test]
async fn an_expired_confirmation_code_is_refused() {
    let db = TestDb::new().await;
    let (state, outbox) = mail_state(&db, 0).await;
    let app = birdtest::app(state);
    let code = registered_code(&app, &outbox, "toolate").await;
    sqlx::query("UPDATE email_confirmations SET expires_at = now() - interval '1 minute'")
        .execute(&db.pool)
        .await
        .unwrap();

    let response = post(&app, "/api/auth/confirm-email", &[], json!({ "code": code })).await;
    assert_eq!(response.status, StatusCode::BAD_REQUEST, "{response:?}");
    assert_eq!(response.json()["message"], "that confirmation link is invalid or has expired");
    assert!(!confirmed(&db, "toolate").await);
}

async fn reset_request(app: &Router, email: &str, ip: &str) -> Response {
    post(
        app,
        "/api/auth/reset-password/request",
        &[("x-forwarded-for", ip)],
        json!({ "email": email }),
    )
    .await
}

/// A-AUTH-8: a reset request answers a registered and an unregistered address
/// with the same bytes. Only the registered one is sent a link.
#[tokio::test]
async fn a_reset_request_answers_the_same_for_known_and_unknown_addresses() {
    let db = TestDb::new().await;
    let (state, outbox) = mail_state(&db, 0).await;
    let app = birdtest::app(state);
    confirmed_user(&db, "forgetful", PASSWORD).await;

    let unknown = reset_request(&app, "nobody@example.invalid", "").await;
    let known = reset_request(&app, "forgetful@example.invalid", "").await;
    assert_eq!(known.status, StatusCode::OK, "{known:?}");
    assert_eq!(unknown.status, known.status);
    assert_eq!(unknown.bytes, known.bytes, "{unknown:?} / {known:?}");
    assert_eq!(
        known.json()["message"],
        "if that address has an account, a reset link is on its way"
    );

    let mail = outbox.wait_for("forgetful@example.invalid", 1).await;
    assert!(mail[0].contains("/reset-password/confirm?token="), "{}", mail[0]);
    // The unknown address's request came first; by the time the known one's
    // mail has landed, anything it had sent would have too.
    assert!(outbox.messages_to("nobody@example.invalid").is_empty());
}

/// A-AUTH-8, the timing half: the reset mail is sent off the request path. With
/// a mailer that cannot send at all, the request still answers exactly as it
/// does for an unknown address, so the response cannot depend on how long the
/// send takes. Registration's mail goes the same way now: it waited on its
/// send, except for a taken address whose notice its per-address limit
/// skipped, which answered that much sooner.
#[tokio::test]
async fn a_reset_request_does_not_wait_on_the_mail_it_sends() {
    let db = TestDb::new().await;
    // An outbox under a regular file: every send fails.
    let blocker = Outbox::new();
    std::fs::create_dir_all(&blocker.0).unwrap();
    std::fs::write(blocker.0.join("file"), b"").unwrap();
    let mut cfg = db.config();
    cfg.mail_backend = MailBackend::File;
    cfg.mail_outbox_dir = Some(blocker.0.join("file").join("outbox"));
    let app = birdtest::app(db.state_with(cfg).await);
    confirmed_user(&db, "forgetful", PASSWORD).await;

    let registered = register(&app, "newcomer", "newcomer@example.invalid", PASSWORD, "").await;
    assert_eq!(registered.status, StatusCode::CREATED, "not held up by its mail: {registered:?}");

    let unknown = reset_request(&app, "nobody@example.invalid", "").await;
    let known = reset_request(&app, "forgetful@example.invalid", "").await;
    assert_eq!(known.status, StatusCode::OK, "{known:?}");
    assert_eq!(known.bytes, unknown.bytes);
    // The token was still issued; only its mail failed.
    let tokens: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM password_reset_tokens")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(tokens, 1);
}

async fn reset_confirm(app: &Router, token: &str, password: &str) -> Response {
    post(
        app,
        "/api/auth/reset-password/confirm",
        &[],
        json!({ "token": token, "password": password }),
    )
    .await
}

/// A-AUTH-9: a reset token works once; a successful reset spends every other
/// outstanding token for the account and signs out its sessions; an expired
/// token is refused.
#[tokio::test]
async fn a_reset_token_is_single_use_spent_by_any_reset_and_expires() {
    let db = TestDb::new().await;
    let (state, outbox) = mail_state(&db, 0).await;
    let app = birdtest::app(state);
    let user = confirmed_user(&db, "resetter", PASSWORD).await;
    let email = "resetter@example.invalid";

    let signed_in = login(&app, "resetter", PASSWORD).await;
    assert_eq!(signed_in.status, StatusCode::OK);
    let mut browser = Browser::default();
    browser.absorb("/api/auth/login", &signed_in.headers);
    let old_session = browser.cookie_header("/api/me");

    // Two links outstanding at once.
    assert_eq!(reset_request(&app, email, "").await.status, StatusCode::OK);
    assert_eq!(reset_request(&app, email, "").await.status, StatusCode::OK);
    let mail = outbox.wait_for(email, 2).await;
    let (first, second) = (link_param(&mail[0], "token"), link_param(&mail[1], "token"));
    assert_ne!(first, second);

    let response = reset_confirm(&app, &first, NEW_PASSWORD).await;
    assert_eq!(response.status, StatusCode::OK, "{response:?}");
    assert_eq!(response.json()["message"], "password updated");

    for (case, token) in [("replayed", &first), ("the other outstanding link", &second)] {
        let response = reset_confirm(&app, token, "another-fresh-passphrase-38").await;
        assert_eq!(response.status, StatusCode::BAD_REQUEST, "{case}: {response:?}");
        assert_eq!(response.json()["message"], "that reset link is invalid or has expired", "{case}");
    }

    // The reset took, and signed the earlier session out.
    let (status, _) = send(&app, get_request("/api/me", &[old_session])).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(login(&app, "resetter", PASSWORD).await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(login(&app, "resetter", NEW_PASSWORD).await.status, StatusCode::OK);

    // A fresh link that has expired.
    assert_eq!(reset_request(&app, email, "").await.status, StatusCode::OK);
    let mail = outbox.wait_for(email, 3).await;
    let spent = [first.as_str(), second.as_str()];
    let third = mail
        .iter()
        .map(|m| link_param(m, "token"))
        .find(|t| !spent.contains(&t.as_str()))
        .unwrap();
    sqlx::query(
        "UPDATE password_reset_tokens SET expires_at = now() - interval '1 minute'
         WHERE user_id = $1 AND used_at IS NULL",
    )
    .bind(user)
    .execute(&db.pool)
    .await
    .unwrap();
    let response = reset_confirm(&app, &third, "another-fresh-passphrase-38").await;
    assert_eq!(response.status, StatusCode::BAD_REQUEST, "{response:?}");
    assert_eq!(response.json()["message"], "that reset link is invalid or has expired");
    assert_eq!(login(&app, "resetter", NEW_PASSWORD).await.status, StatusCode::OK);
}

/// A-AUTH-10: logging out removes the session cookie from the browser -- the
/// removal names the path the cookie was set on, or a browser keeps the
/// original -- and the browser is then no longer signed in.
#[tokio::test]
async fn logout_clears_the_session_cookie_and_the_browser_is_signed_out() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    confirmed_user(&db, "leaver", PASSWORD).await;

    let response = login(&app, "leaver", PASSWORD).await;
    assert_eq!(response.status, StatusCode::OK, "{response:?}");
    let mut browser = Browser::default();
    browser.absorb("/api/auth/login", &response.headers);
    let (status, _) = send(&app, get_request("/api/me", &[browser.cookie_header("/api/me")])).await;
    assert_eq!(status, StatusCode::OK);

    let csrf = browser.get("/api/auth/logout", "birdtest_csrf").unwrap();
    let cookie = browser.cookie_header("/api/auth/logout");
    let response = send_raw(
        &app,
        Request::post("/api/auth/logout")
            .header(cookie.0, cookie.1)
            .header("x-csrf-token", csrf)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(response.status, StatusCode::NO_CONTENT, "{response:?}");
    browser.absorb("/api/auth/logout", &response.headers);

    for page in ["/api/me", "/api/me/api-keys", "/api/auth/sign-out-everywhere"] {
        assert_eq!(browser.get(page, "birdtest_session"), None, "{page}: {:?}", set_cookies(&response));
        assert_eq!(browser.get(page, "birdtest_csrf"), None, "{page}");
    }
    let (status, body) = send(&app, get_request("/api/me", &[browser.cookie_header("/api/me")])).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(body["message"], "not signed in");
}

fn retry_after(response: &Response) -> u64 {
    response
        .headers
        .get(RETRY_AFTER)
        .unwrap_or_else(|| panic!("no Retry-After: {response:?}"))
        .to_str()
        .unwrap()
        .parse()
        .unwrap()
}

/// A-AUTH-11: the eleventh registration from one address in an hour is 429
/// with a `Retry-After`; another address is unaffected.
#[tokio::test]
async fn the_eleventh_registration_from_one_address_is_rate_limited() {
    let db = TestDb::new().await;
    let (state, _outbox) = mail_state(&db, 1).await;
    let app = birdtest::app(state);

    for i in 0..10 {
        let response =
            register(&app, &format!("burst{i}"), &format!("burst{i}@example.invalid"), PASSWORD, "203.0.113.5")
                .await;
        assert_eq!(response.status, StatusCode::CREATED, "#{i}: {response:?}");
    }
    let limited =
        register(&app, "burst10", "burst10@example.invalid", PASSWORD, "203.0.113.5").await;
    assert_eq!(limited.status, StatusCode::TOO_MANY_REQUESTS, "{limited:?}");
    assert_eq!(limited.json()["code"], "rate_limited");
    assert!(retry_after(&limited) >= 1);

    let elsewhere =
        register(&app, "burst10", "burst10@example.invalid", PASSWORD, "198.51.100.9").await;
    assert_eq!(elsewhere.status, StatusCode::CREATED, "{elsewhere:?}");
}

/// A-AUTH-11, the half that matters: the sixth reset request for one address
/// within the hour is 429 even though every request came from a different
/// client address. Another address is unaffected.
#[tokio::test]
async fn the_sixth_reset_for_one_address_is_rate_limited_from_any_ip() {
    let db = TestDb::new().await;
    let (state, _outbox) = mail_state(&db, 1).await;
    let app = birdtest::app(state);
    confirmed_user(&db, "target", PASSWORD).await;

    for i in 0..5 {
        let response = reset_request(&app, "target@example.invalid", &format!("203.0.113.{i}")).await;
        assert_eq!(response.status, StatusCode::OK, "#{i}: {response:?}");
    }
    let limited = reset_request(&app, "target@example.invalid", "203.0.113.99").await;
    assert_eq!(limited.status, StatusCode::TOO_MANY_REQUESTS, "{limited:?}");
    assert_eq!(limited.json()["code"], "rate_limited");
    assert!(retry_after(&limited) >= 1);
    // Case and padding do not make it another address.
    let limited = reset_request(&app, " TARGET@example.invalid", "203.0.113.100").await;
    assert_eq!(limited.status, StatusCode::TOO_MANY_REQUESTS, "{limited:?}");

    let other = reset_request(&app, "bystander@example.invalid", "203.0.113.101").await;
    assert_eq!(other.status, StatusCode::OK, "{other:?}");
}

/// A-AUTH-5, on the reset path: a reset refuses a password built from the
/// account's username or address, exactly as registration does -- and the
/// refusal leaves the link usable for a better password.
#[tokio::test]
async fn a_reset_refuses_a_password_built_from_the_account_and_keeps_the_link() {
    let db = TestDb::new().await;
    let (state, outbox) = mail_state(&db, 0).await;
    let app = birdtest::app(state);
    confirmed_user(&db, "vexmorlandtriq", PASSWORD).await;
    let email = "vexmorlandtriq@example.invalid";

    assert_eq!(reset_request(&app, email, "").await.status, StatusCode::OK);
    let mail = outbox.wait_for(email, 1).await;
    let token = link_param(&mail[0], "token");

    for derived in ["Vexmorlandtriq!", "vexmorlandtriq@example.invalid1"] {
        let alone = zxcvbn::zxcvbn(derived, &[]).unwrap().score();
        assert!(alone >= 3, "{derived:?} scores {alone} on its own; the context must refuse it");
        let response = reset_confirm(&app, &token, derived).await;
        assert_eq!(response.status, StatusCode::BAD_REQUEST, "{derived}: {response:?}");
        assert_eq!(
            response.json()["fields"],
            json!([{ "field": "password", "message": "too weak — choose a longer, less predictable password" }]),
        );
    }
    assert_eq!(login(&app, "vexmorlandtriq", PASSWORD).await.status, StatusCode::OK);

    let response = reset_confirm(&app, &token, NEW_PASSWORD).await;
    assert_eq!(response.status, StatusCode::OK, "the refused attempts spent the link: {response:?}");
    assert_eq!(login(&app, "vexmorlandtriq", NEW_PASSWORD).await.status, StatusCode::OK);
}
