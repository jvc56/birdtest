use crate::auth::{api_key, csrf, session, CurrentUser};
use crate::clientip::ClientIp;
use crate::error::{AppError, AppResult};
use crate::ratelimit;
use crate::state::AppState;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use chrono::{Duration, Utc};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/register", post(register))
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/sign-out-everywhere", post(sign_out_everywhere))
        .route("/confirm-email", post(confirm_email))
        .route("/reset-password/request", post(request_password_reset))
        .route("/reset-password/confirm", post(confirm_password_reset))
}

const MIN_PASSWORD_SCORE: u8 = 3;
const CONFIRMATION_TTL_HOURS: i64 = 24;
const RESET_TTL_MINUTES: i64 = 30;

fn session_cookie(state: &AppState, value: String) -> Cookie<'static> {
    let mut cookie = Cookie::new(session::SESSION_COOKIE, value);
    cookie.set_http_only(true);
    cookie.set_secure(state.cfg.secure_cookies);
    cookie.set_same_site(SameSite::Strict);
    cookie.set_path("/");
    cookie
}

/// Readable by JavaScript on purpose — the frontend echoes it back in the
/// `X-CSRF-Token` header, which is what makes the double-submit check work.
fn csrf_cookie(state: &AppState, value: String) -> Cookie<'static> {
    let mut cookie = Cookie::new(csrf::CSRF_COOKIE, value);
    cookie.set_http_only(false);
    cookie.set_secure(state.cfg.secure_cookies);
    cookie.set_same_site(SameSite::Strict);
    cookie.set_path("/");
    cookie
}

#[derive(Deserialize)]
struct RegisterBody {
    username: String,
    email: String,
    password: String,
}

#[derive(Serialize)]
struct MessageBody {
    message: &'static str,
}

async fn register(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    Json(body): Json<RegisterBody>,
) -> AppResult<(StatusCode, Json<MessageBody>)> {
    ratelimit::check(&state.limits.register, &ip.to_string())?;

    let username = body.username.trim().to_string();
    let email = body.email.trim().to_lowercase();

    let mut err = AppError::bad_request("registration details are invalid");
    if username.len() < 3 || username.len() > 32 {
        err = err.with_field("username", "must be between 3 and 32 characters");
    }
    if !email.contains('@') || email.len() < 3 {
        err = err.with_field("email", "must be a valid email address");
    }
    // Scored server-side; the client shows the same feedback but is not trusted.
    let entropy = zxcvbn::zxcvbn(&body.password, &[username.as_str(), email.as_str()])
        .map_err(|e| AppError::bad_request(format!("could not score password: {e}")))?;
    if entropy.score() < MIN_PASSWORD_SCORE {
        err = err.with_field("password", "too weak — choose a longer, less predictable password");
    }
    if !err.fields.is_empty() {
        return Err(err);
    }

    let taken = sqlx::query_as::<_, (bool, bool)>(
        "SELECT EXISTS (SELECT 1 FROM users WHERE username = $1),
                EXISTS (SELECT 1 FROM users WHERE email = $2)",
    )
    .bind(&username)
    .bind(&email)
    .fetch_one(&state.pool)
    .await?;
    let (username_taken, email_taken) = taken;

    // Hashed before the collision branch, not after, so both paths pay the same
    // Argon2 cost. Returning an identical body for a taken address and then
    // answering in tens of milliseconds less would give the answer back through
    // timing, which is exactly the flaw this branch exists to avoid.
    let password_hash = api_key::hash_password(&body.password)?;

    // A taken username is reported plainly: the user has to choose another one
    // to get anywhere, and `GET /api/users` publishes the whole list anyway, so
    // there is nothing here to protect.
    if username_taken {
        return Err(AppError::conflict("registration details are invalid")
            .with_field("username", "that username is taken"));
    }

    // A taken *email* is not reported. Answering "that address is already
    // registered" turns this endpoint into an oracle for whether a given person
    // has an account -- something login and password reset both go out of their
    // way not to reveal, and which registration should not undo. The caller sees
    // exactly what a new registration sees; the address owner is told someone
    // tried, so a real person who has forgotten they signed up still finds out.
    if email_taken {
        state
            .mailer
            .send(
                &email,
                "Someone tried to register with your email address",
                &format!(
                    "Someone tried to create a birdtest account with this \
                     address, but it already has one.\n\n\
                     If that was you, sign in at {url}/login, or reset your \
                     password at {url}/reset-password if you have forgotten \
                     it.\n\n\
                     If it was not you, no account was created and nothing \
                     has changed.\n",
                    url = state.cfg.public_url
                ),
            )
            .await?;
        return Ok((
            StatusCode::CREATED,
            Json(MessageBody { message: "check your email to confirm" }),
        ));
    }
    let raw_code = api_key::generate_code();

    let mut tx = state.pool.begin().await?;
    let user_id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO users (username, email, password_hash) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(&username)
    .bind(&email)
    .bind(&password_hash)
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO email_confirmations (user_id, code_hash, expires_at) VALUES ($1, $2, $3)",
    )
    .bind(user_id)
    .bind(api_key::hash_code(&raw_code))
    .bind(Utc::now() + Duration::hours(CONFIRMATION_TTL_HOURS))
    .execute(&mut *tx)
    .await?;

    crate::audit::log(
        &mut tx,
        "user.registered",
        Some(user_id),
        None,
        Some("user"),
        Some(user_id.to_string()),
        None,
    )
    .await?;
    tx.commit().await?;

    // raw_code is hex today, already URL-safe, but encoding it anyway means
    // this link stays correct even if generate_code's alphabet ever changes,
    // rather than relying on that alphabet implicitly forever.
    let encoded_code = utf8_percent_encode(&raw_code, NON_ALPHANUMERIC);
    let link = format!("{}/confirm-email?code={encoded_code}", state.cfg.public_url);
    state
        .mailer
        .send(
            &email,
            "Confirm your birdtest account",
            &format!("Welcome to birdtest.\n\nConfirm your account:\n{link}\n"),
        )
        .await?;

    // No session is created yet — email is confirmed before the first login.
    Ok((StatusCode::CREATED, Json(MessageBody { message: "check your email to confirm" })))
}

#[derive(Deserialize)]
struct LoginBody {
    username: String,
    password: String,
}

#[derive(Serialize)]
struct LoginResponse {
    username: String,
    is_admin: bool,
}

async fn login(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    jar: CookieJar,
    Json(body): Json<LoginBody>,
) -> AppResult<(CookieJar, Json<LoginResponse>)> {
    // Both halves, like password reset: per IP bounds one guesser, per
    // username bounds many guessers aimed at one account. Checked before the
    // lookup, so a limited attempt costs no Argon2 verify.
    ratelimit::check(&state.limits.login, &format!("ip:{ip}"))?;
    ratelimit::check(
        &state.limits.login,
        &format!("user:{}", body.username.trim().to_lowercase()),
    )?;

    let row = sqlx::query_as::<_, (Uuid, String, String, bool, Option<chrono::DateTime<Utc>>, i32)>(
        "SELECT id, username, password_hash, is_admin, email_confirmed_at, session_generation
         FROM users WHERE username = $1 AND deleted_at IS NULL",
    )
    .bind(body.username.trim())
    .fetch_optional(&state.pool)
    .await?;

    // Identical response whether the username is unknown or the password is
    // wrong, so the endpoint cannot be used to enumerate accounts.
    let invalid = || AppError::unauthorized("incorrect username or password");
    let Some((id, username, password_hash, is_admin, confirmed_at, generation)) = row else {
        return Err(invalid());
    };
    if !api_key::verify_password(&body.password, &password_hash) {
        return Err(invalid());
    }
    if confirmed_at.is_none() {
        return Err(AppError::forbidden(
            "confirm your email address before signing in — check your inbox",
        ));
    }

    let token = session::issue(&state.cfg, id, &username, is_admin, generation)?;
    let jar = jar
        .add(session_cookie(&state, token))
        .add(csrf_cookie(&state, csrf::generate_token()));

    Ok((jar, Json(LoginResponse { username, is_admin })))
}

async fn logout(
    State(state): State<AppState>,
    method: Method,
    headers: HeaderMap,
    jar: CookieJar,
) -> AppResult<(CookieJar, StatusCode)> {
    csrf::verify(&method, &headers, &jar)?;
    let jar = jar
        .remove(Cookie::from(session::SESSION_COOKIE))
        .remove(Cookie::from(csrf::CSRF_COOKIE));
    let _ = state;
    Ok((jar, StatusCode::NO_CONTENT))
}

/// Revokes every session this account has, including the caller's: bumping
/// the generation makes every token minted before it fail `CurrentUser`.
async fn sign_out_everywhere(
    State(state): State<AppState>,
    user: CurrentUser,
    method: Method,
    headers: HeaderMap,
    jar: CookieJar,
) -> AppResult<(CookieJar, StatusCode)> {
    csrf::verify(&method, &headers, &jar)?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("UPDATE users SET session_generation = session_generation + 1 WHERE id = $1")
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    crate::audit::log(
        &mut tx,
        "user.signed_out_everywhere",
        Some(user.id),
        None,
        Some("user"),
        Some(user.id.to_string()),
        None,
    )
    .await?;
    tx.commit().await?;
    let jar = jar
        .remove(Cookie::from(session::SESSION_COOKIE))
        .remove(Cookie::from(csrf::CSRF_COOKIE));
    Ok((jar, StatusCode::NO_CONTENT))
}

#[derive(Deserialize)]
struct ConfirmEmailBody {
    code: String,
}

async fn confirm_email(
    State(state): State<AppState>,
    Json(body): Json<ConfirmEmailBody>,
) -> AppResult<Json<MessageBody>> {
    let code_hash = api_key::hash_code(body.code.trim());

    let mut tx = state.pool.begin().await?;
    let user_id = sqlx::query_scalar::<_, Uuid>(
        "UPDATE email_confirmations SET used_at = now()
         WHERE code_hash = $1 AND used_at IS NULL AND expires_at > now()
         RETURNING user_id",
    )
    .bind(&code_hash)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::bad_request("that confirmation link is invalid or has expired"))?;

    sqlx::query("UPDATE users SET email_confirmed_at = now() WHERE id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    Ok(Json(MessageBody { message: "email confirmed" }))
}

#[derive(Deserialize)]
struct ResetRequestBody {
    email: String,
}

async fn request_password_reset(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    Json(body): Json<ResetRequestBody>,
) -> AppResult<Json<MessageBody>> {
    let email = body.email.trim().to_lowercase();

    // Checked against the caller and against the address they named. Both
    // halves are load-bearing: the first bounds bulk probing, the second stops
    // one address being buried in reset mail from many sources.
    ratelimit::check(&state.limits.reset, &format!("ip:{ip}"))?;
    ratelimit::check(&state.limits.reset, &format!("em:{email}"))?;
    let user = sqlx::query_as::<_, (Uuid,)>(
        "SELECT id FROM users
         WHERE email = $1 AND email_confirmed_at IS NOT NULL AND deleted_at IS NULL",
    )
    .bind(&email)
    .fetch_optional(&state.pool)
    .await?;

    if let Some((user_id,)) = user {
        let raw_token = api_key::generate_code();
        sqlx::query(
            "INSERT INTO password_reset_tokens (user_id, token_hash, expires_at)
             VALUES ($1, $2, $3)",
        )
        .bind(user_id)
        .bind(api_key::hash_code(&raw_token))
        .bind(Utc::now() + Duration::minutes(RESET_TTL_MINUTES))
        .execute(&state.pool)
        .await?;

        // See the same encoding note on the confirm-email link above.
        let encoded_token = utf8_percent_encode(&raw_token, NON_ALPHANUMERIC);
        let link =
            format!("{}/reset-password/confirm?token={encoded_token}", state.cfg.public_url);

        // Sent off the request path, which is what makes the identical body
        // below actually mean something. Awaiting an SES round trip here and
        // returning immediately when the address is unknown answers the
        // question by timing: hundreds of milliseconds against a sub-millisecond
        // index miss is not a subtle signal. Spawning also keeps a slow or
        // failing mail provider out of the caller's latency.
        let mailer = state.mailer.clone();
        tokio::spawn(async move {
            if let Err(err) = mailer
                .send(
                    &email,
                    "Reset your birdtest password",
                    &format!(
                        "Reset your password (valid for {RESET_TTL_MINUTES} minutes):\n{link}\n"
                    ),
                )
                .await
            {
                // Nothing to report to the caller -- they were told the same
                // thing either way -- so this is the only record that the mail
                // did not go out.
                tracing::error!(error = %err.message, "password reset email failed to send");
            }
        });
    }

    // Always 200, whether or not the address is registered — otherwise this
    // endpoint would confirm which addresses have accounts.
    Ok(Json(MessageBody { message: "if that address has an account, a reset link is on its way" }))
}

#[derive(Deserialize)]
struct ResetConfirmBody {
    token: String,
    password: String,
}

async fn confirm_password_reset(
    State(state): State<AppState>,
    jar: CookieJar,
    Json(body): Json<ResetConfirmBody>,
) -> AppResult<(CookieJar, Json<MessageBody>)> {
    let entropy = zxcvbn::zxcvbn(&body.password, &[])
        .map_err(|e| AppError::bad_request(format!("could not score password: {e}")))?;
    if entropy.score() < MIN_PASSWORD_SCORE {
        return Err(AppError::bad_request("password is invalid")
            .with_field("password", "too weak — choose a longer, less predictable password"));
    }

    let mut tx = state.pool.begin().await?;
    let user_id = sqlx::query_scalar::<_, Uuid>(
        "UPDATE password_reset_tokens SET used_at = now()
         WHERE token_hash = $1 AND used_at IS NULL AND expires_at > now()
         RETURNING user_id",
    )
    .bind(api_key::hash_code(body.token.trim()))
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::bad_request("that reset link is invalid or has expired"))?;

    // The generation bump signs out every existing session, an attacker's
    // included: resetting a password is what someone does when they suspect
    // another person has access.
    sqlx::query(
        "UPDATE users SET password_hash = $1, session_generation = session_generation + 1
         WHERE id = $2 AND deleted_at IS NULL",
    )
        .bind(api_key::hash_password(&body.password)?)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;

    // Every other outstanding reset token for this account is spent too, so a
    // second link from an earlier request cannot be replayed.
    sqlx::query(
        "UPDATE password_reset_tokens SET used_at = now()
         WHERE user_id = $1 AND used_at IS NULL",
    )
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    // Every session was revoked above; dropping the cookie just tidies the
    // caller's browser.
    let jar = jar.remove(Cookie::from(session::SESSION_COOKIE));
    Ok((jar, Json(MessageBody { message: "password updated" })))
}
