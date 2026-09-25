use crate::auth::{api_key, csrf, session, CurrentUser};
use crate::clientip::ClientIp;
use crate::extract::ApiJson;
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

/// The removal of a cookie set by the two functions around this one. A
/// browser replaces a cookie only with one of the same path, and gives a
/// cookie that names none the directory of the request that set it -- here
/// `/api/auth` -- so a removal without `Path=/` removed nothing and the
/// session cookie outlived the logout.
fn removal(name: &'static str) -> Cookie<'static> {
    let mut cookie = Cookie::from(name);
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

/// Whether `password` scores below the minimum, given the account's username
/// and address as context. The address's local part is context of its own:
/// zxcvbn matches each input whole, so the address alone let `jsmith` through
/// for `jsmith@example.com`.
fn too_weak(password: &str, username: &str, email: &str) -> AppResult<bool> {
    let local_part = email.split('@').next().unwrap_or_default();
    let context: Vec<&str> =
        [username, email, local_part].into_iter().filter(|c| !c.is_empty()).collect();
    let entropy = zxcvbn::zxcvbn(password, &context)
        .map_err(|e| AppError::bad_request(format!("could not score password: {e}")))?;
    Ok(entropy.score() < MIN_PASSWORD_SCORE)
}


/// One bare address: `local@domain`, nothing else. Mail goes to whatever this
/// string is, so the check is on what a mail API would do with it rather than
/// on RFC 5322: `x <victim@example.com>` and `victim@example.com,x@y` were
/// accepted, each a different string -- past the taken-address check and the
/// per-address notice limit -- that mailed the same inbox.
fn is_bare_address(email: &str) -> bool {
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    email.len() <= 254
        && !local.is_empty()
        && local.len() <= 64
        && !domain.contains('@')
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.contains("..")
        && email.chars().all(|c| {
            c.is_ascii_graphic() && !matches!(c, '<' | '>' | ',' | ';' | ':' | '"' | '(' | ')' | '[' | ']' | '\\')
        })
}

async fn register(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    ApiJson(body): ApiJson<RegisterBody>,
) -> AppResult<(StatusCode, Json<MessageBody>)> {
    ratelimit::check(&state.limits.register, &ip.to_string())?;

    let username = body.username.trim().to_string();
    let email = body.email.trim().to_lowercase();

    let mut err = AppError::bad_request("registration details are invalid");
    if username.len() < 3 || username.len() > 32 {
        err = err.with_field("username", "must be between 3 and 32 characters");
    }
    if !is_bare_address(&email) {
        err = err.with_field("email", "must be a valid email address");
    }
    // Scored server-side; the client shows the same feedback but is not trusted.
    if too_weak(&body.password, &username, &email)? {
        err = err.with_field("password", "too weak — choose a longer, less predictable password");
    }
    if !err.fields.is_empty() {
        return Err(err);
    }

    // An account that never confirmed its address, and whose confirmation has
    // expired, gives up its username and address to whoever registers them
    // next. Held for ever, it was a dead end with no way out -- it cannot sign
    // in, reset its password or be sent a new code -- and a way to squat
    // anyone's address: register it first, and its owner could never sign up
    // unless they happened to click the stranger's link within the day.
    // Nothing else can hang off such an account (it has never signed in), and
    // an admin is never released this way.
    sqlx::query(
        "DELETE FROM users u
         WHERE (lower(u.username) = lower($1) OR u.email = $2)
           AND u.email_confirmed_at IS NULL AND u.deleted_at IS NULL AND NOT u.is_admin
           AND NOT EXISTS (
               SELECT 1 FROM email_confirmations c
               WHERE c.user_id = u.id AND c.used_at IS NULL AND c.expires_at > now()
           )",
    )
    .bind(&username)
    .bind(&email)
    .execute(&state.pool)
    .await?;

    let taken = sqlx::query_as::<_, (bool, bool, bool)>(
        "SELECT EXISTS (SELECT 1 FROM users WHERE lower(username) = lower($1)),
                EXISTS (SELECT 1 FROM users WHERE email = $2),
                EXISTS (SELECT 1 FROM users WHERE email = $2 AND email_confirmed_at IS NOT NULL)",
    )
    .bind(&username)
    .bind(&email)
    .fetch_one(&state.pool)
    .await?;
    let (username_taken, email_taken, email_confirmed) = taken;

    // Hashed before the collision branch, not after, so both paths pay the same
    // Argon2 cost. Returning an identical body for a taken address and then
    // answering in tens of milliseconds less would give the answer back through
    // timing, which is exactly the flaw this branch exists to avoid.
    let password_hash = api_key::hash_password_off_the_executor(body.password.clone()).await?;

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
        // Limited per address, and skipped rather than refused when limited:
        // the per-IP limit alone let anyone bury an address in these notices
        // from enough IPs, and a refusal here -- and only here -- would answer
        // the question this branch hides.
        let notice = if email_confirmed {
            format!(
                "Someone tried to create a birdtest account with this \
                 address, but it already has one.\n\n\
                 If that was you, sign in at {url}/login, or reset your \
                 password at {url}/reset-password if you have forgotten \
                 it.\n\n\
                 If it was not you, no account was created and nothing \
                 has changed.\n",
                url = state.cfg.public_url
            )
        } else {
            format!(
                "Someone tried to create a birdtest account with this \
                 address, but an account with it is already waiting for the \
                 address to be confirmed.\n\n\
                 If that was you, use the confirmation link sent when it was \
                 created. It works for {CONFIRMATION_TTL_HOURS} hours; once it \
                 has expired, register again and the address is yours.\n\n\
                 If it was not you, nothing has changed, and the waiting \
                 account cannot be used without the link sent to you.\n"
            )
        };
        if ratelimit::check(&state.limits.reset, &format!("reg-em:{email}")).is_ok() {
            send_in_background(
                &state,
                email.clone(),
                "Someone tried to register with your email address",
                notice,
            );
        }
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
    .await
    .map_err(|err| {
        // Two registrations of one name (in any case) at once: the loser is
        // told what the check above would have told it.
        let username_taken = matches!(
            &err,
            sqlx::Error::Database(db)
                if matches!(db.constraint(), Some("users_username_key" | "users_username_lower_idx"))
        );
        if username_taken {
            AppError::conflict("registration details are invalid")
                .with_field("username", "that username is taken")
        } else {
            err.into()
        }
    })?;

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
    send_in_background(
        &state,
        email,
        "Confirm your birdtest account",
        format!("Welcome to birdtest.\n\nConfirm your account:\n{link}\n"),
    );

    // No session is created yet — email is confirmed before the first login.
    Ok((StatusCode::CREATED, Json(MessageBody { message: "check your email to confirm" })))
}

/// Registration's mail, sent off the request as password reset's is. Both
/// branches of a registration -- a new account's confirmation, a taken
/// address's notice -- used to await their send, except that a notice skipped
/// by its per-address limit awaited nothing, and answered that much sooner:
/// the timing gave back what the identical bodies hide. A failed send is
/// logged; the caller was told the same thing either way.
fn send_in_background(state: &AppState, to: String, subject: &'static str, body: String) {
    let mailer = state.mailer.clone();
    tokio::spawn(async move {
        if let Err(err) = mailer.send(&to, subject, &body).await {
            tracing::error!(error = %err.message, subject, "registration email failed to send");
        }
    });
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
    ApiJson(body): ApiJson<LoginBody>,
) -> AppResult<(CookieJar, Json<LoginResponse>)> {
    // Both halves, like password reset: per IP bounds one guesser, per
    // username bounds many guessers aimed at one account. Checked before the
    // lookup, so a limited attempt costs no Argon2 verify.
    //
    // The username half is ten times the address half. At the per-address
    // rate it let one caller trying a wrong password every six seconds hold
    // any account -- an admin's, whose name `GET /api/users` publishes -- out
    // of signing in for as long as it kept going, correct password or not; at
    // this rate a lockout takes a fleet of addresses, and the cap still bounds
    // how fast a distributed guesser can try. (A per-username-per-address
    // bucket at the address's own rate was tried and could never trip: it
    // only ever saw requests the address's bucket had already let through.)
    let account = body.username.trim().to_lowercase();
    ratelimit::check(&state.limits.login, &format!("ip:{ip}"))?;
    ratelimit::check(&state.limits.login_account, &format!("user:{account}"))?;

    let row = sqlx::query_as::<_, (Uuid, String, String, bool, Option<chrono::DateTime<Utc>>, i32)>(
        "SELECT id, username, password_hash, is_admin, email_confirmed_at, session_generation
         FROM users WHERE lower(username) = lower($1) AND deleted_at IS NULL",
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
    if !api_key::verify_password_off_the_executor(body.password.clone(), password_hash.clone()).await {
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
        .remove(removal(session::SESSION_COOKIE))
        .remove(removal(csrf::CSRF_COOKIE));
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
        .remove(removal(session::SESSION_COOKIE))
        .remove(removal(csrf::CSRF_COOKIE));
    Ok((jar, StatusCode::NO_CONTENT))
}

#[derive(Deserialize)]
struct ConfirmEmailBody {
    code: String,
}

async fn confirm_email(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<ConfirmEmailBody>,
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
    ApiJson(body): ApiJson<ResetRequestBody>,
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
    ApiJson(body): ApiJson<ResetConfirmBody>,
) -> AppResult<(CookieJar, Json<MessageBody>)> {
    let weak = || {
        AppError::bad_request("password is invalid")
            .with_field("password", "too weak — choose a longer, less predictable password")
    };
    // Scored once before the token is looked at, so a weak password costs no
    // database work, and again below against the account it resets.
    if too_weak(&body.password, "", "")? {
        return Err(weak());
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

    // The same rule registration applies: not the username, not the address.
    // Returning here rolls the transaction back, so the link still works for
    // a better password.
    let (username, email) =
        sqlx::query_as::<_, (String, String)>("SELECT username, email FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;
    if too_weak(&body.password, &username, &email)? {
        return Err(weak());
    }

    // The generation bump signs out every existing session, an attacker's
    // included: resetting a password is what someone does when they suspect
    // another person has access.
    sqlx::query(
        "UPDATE users SET password_hash = $1, session_generation = session_generation + 1
         WHERE id = $2 AND deleted_at IS NULL",
    )
        .bind(api_key::hash_password_off_the_executor(body.password.clone()).await?)
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
    let jar = jar.remove(removal(session::SESSION_COOKIE));
    Ok((jar, Json(MessageBody { message: "password updated" })))
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_a_bare_address_is_an_email() {
        for good in ["a@example.com", "first.last+tag@mail.example.co.uk", "x_y-z@b.io"] {
            assert!(super::is_bare_address(good), "{good}");
        }
        for bad in [
            "x <victim@example.com>", "victim@example.com,other@x.com", "a@b", "@example.com",
            "a@@example.com", "a@example..com", "a b@example.com", "a@example.com.", "\"a\"@example.com",
            "a@.example.com", "a;b@example.com", "é@example.com",
        ] {
            assert!(!super::is_bare_address(bad), "{bad}");
        }
    }
}
