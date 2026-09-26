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

/// The body limit of routes that take a handful of short strings.
pub(crate) const SMALL_BODY_BYTES: usize = 16 * 1024;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/register", post(register))
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/sign-out-everywhere", post(sign_out_everywhere))
        .route("/confirm-email", post(confirm_email))
        .route("/reset-password/request", post(request_password_reset))
        .route("/reset-password/confirm", post(confirm_password_reset))
        // Every body here is a few short strings; axum's default 2 MB let a
        // caller send a megabyte-long "username" to every limiter keyed on one.
        .layer(axum::extract::DefaultBodyLimit::max(SMALL_BODY_BYTES))
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

/// How many passwords may be scored at once, and how long a request waits
/// for a turn before it is told to come back. Scoring has turns of its own:
/// a crafted password costs zxcvbn close to a second, and on sign-in's four
/// Argon2 turns a reset link replayed with weak passwords from eight
/// addresses held every sign-in at `503` (the audit's pass 6).
const SCORING_CONCURRENCY: usize = 2;
const SCORING_QUEUE_WAIT: std::time::Duration = std::time::Duration::from_secs(10);
static SCORING_PERMITS: tokio::sync::Semaphore =
    tokio::sync::Semaphore::const_new(SCORING_CONCURRENCY);

/// [`too_weak`] off the executor, at most `SCORING_CONCURRENCY` at a time: on
/// the executor a crafted password stalled every request (`/health` 8.5 s).
/// The turn goes with the run, and a run whose requester has gone is skipped.
async fn too_weak_off_the_executor(password: &str, username: &str, email: &str) -> AppResult<bool> {
    score_in_turn(scoring_turn().await?, password, username, email).await
}

/// A turn to score a password, waited for at most `SCORING_QUEUE_WAIT`.
async fn scoring_turn() -> AppResult<tokio::sync::SemaphorePermit<'static>> {
    match tokio::time::timeout(SCORING_QUEUE_WAIT, SCORING_PERMITS.acquire()).await {
        Ok(Ok(turn)) => Ok(turn),
        Ok(Err(_)) => Err(AppError::internal("the password scoring queue is closed")),
        Err(_) => {
            Err(AppError {
                retry_after: Some(10),
                ..AppError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "unavailable",
                    "the server is busy checking passwords; try again in a few seconds",
                )
            })
        }
    }
}

/// [`too_weak`] on the blocking pool, in the turn given. The turn goes with
/// the run, and a run whose requester has gone is skipped.
async fn score_in_turn(
    turn: tokio::sync::SemaphorePermit<'static>,
    password: &str,
    username: &str,
    email: &str,
) -> AppResult<bool> {
    let (password, username, email) = (password.to_owned(), username.to_owned(), email.to_owned());
    let (sender, receiver) = tokio::sync::oneshot::channel();
    tokio::task::spawn_blocking(move || {
        let _turn = turn;
        if !sender.is_closed() {
            let _ = sender.send(too_weak(&password, &username, &email));
        }
    });
    receiver.await.map_err(|_| AppError::internal("scoring a password did not finish"))?
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

/// A character a username may not hold: a control character, a line or
/// paragraph separator, a format character (Unicode's Cf: bidi overrides,
/// zero-width marks and the like), or one that shows as nothing (the other
/// default-ignorable characters, variation selectors among them, and the
/// Hangul fillers and blank Braille pattern, which render as blanks). A username goes into mail to an address's
/// owner — the reset mail, the taken-address notice — and a stranger can
/// register someone's address under a name of their choosing (KL-34): with
/// line breaks it was a message of their own in birdtest's mail, and with
/// bidi overrides a name that reads as another.
fn is_hidden_or_breaking(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00AD}'
                | '\u{0600}'..='\u{0605}'
                | '\u{061C}'
                | '\u{06DD}'
                | '\u{070F}'
                | '\u{0890}'..='\u{0891}'
                | '\u{08E2}'
                | '\u{180E}'
                | '\u{200B}'
                | '\u{200E}'..='\u{200F}'
                | '\u{2028}'..='\u{202E}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2065}'..='\u{206F}'
                | '\u{FEFF}'
                | '\u{FFF0}'..='\u{FFFB}'
                | '\u{110BD}'
                | '\u{110CD}'
                | '\u{13430}'..='\u{1343F}'
                | '\u{1BCA0}'..='\u{1BCA3}'
                | '\u{1D173}'..='\u{1D17A}'
                // Tags and unassigned default-ignorables, around the
                // ideographic variation selectors placed below.
                | '\u{E0000}'..='\u{E00FF}'
                | '\u{E01F0}'..='\u{E0FFF}'
                // Default-ignorable, but not Cf.
                | '\u{034F}'
                | '\u{115F}'..='\u{1160}'
                | '\u{17B4}'..='\u{17B5}'
                | '\u{180B}'..='\u{180D}'
                | '\u{180F}'
                | '\u{3164}'
                | '\u{FFA0}'
                // Blank.
                | '\u{2800}'
        )
}

/// Whether `name` holds an invisible mark where no script or emoji puts one.
///
/// A zero-width joiner or non-joiner (U+200D, U+200C) belongs between two
/// letters of a script that writes with them -- Arabic and Persian (a Persian
/// keyboard types one on shift+space), Syriac, NKo, Mongolian, the Brahmic
/// scripts (`ශ්‍රී`) -- or between the pictographs of an emoji sequence
/// (`🏳️‍🌈`). A variation selector (U+FE00-FE0F) belongs after a pictograph
/// (`❤️`), an ideograph, or on a keycap (`1️⃣`), and an ideographic one
/// (U+E0100-E01EF) after an ideograph. Anywhere else -- beside `r`, but also
/// beside `ë` or a Cyrillic letter, which "not ASCII" let through -- they only
/// make a second name look like the first (the audit's pass 7).
fn misplaced_joiner(name: &str) -> bool {
    let chars: Vec<char> = name.chars().collect();
    let at = |i: usize| chars.get(i).copied();
    let joins = |c: Option<char>| c.is_some_and(writes_with_joiners);
    let picture = |c: Option<char>| c.is_some_and(pictographic);
    let ideograph = |c: Option<char>| c.is_some_and(ideographic);
    chars.iter().enumerate().any(|(i, &c)| {
        let before = if i > 0 { at(i - 1) } else { None };
        // An emoji's presentation selector sits between it and a joiner.
        let base = if before == Some('\u{FE0F}') && i > 1 { at(i - 2) } else { before };
        let after = at(i + 1);
        match c {
            '\u{200C}' | '\u{200D}' => {
                let word_end = after.is_none_or(char::is_whitespace);
                !((joins(before) && joins(after))
                    || (before.is_some_and(virama) && word_end)
                    || (picture(base) && picture(after)))
            }
            '\u{FE00}'..='\u{FE0F}' => {
                let keycap = before.is_some_and(|b| b.is_ascii_digit() || b == '#' || b == '*')
                    && after == Some('\u{20E3}');
                !(picture(before) || ideograph(before) || keycap)
            }
            '\u{E0100}'..='\u{E01EF}' => !ideograph(before),
            _ => false,
        }
    })
}

/// The joiners and variation selectors a name may hold, as a Postgres regex
/// class: names that differ only in these are one name.
const INVISIBLE_MARKS: &str = "[\\u200C\\u200D\\uFE00-\\uFE0F\\U000E0100-\\U000E01EF]";

/// A virama: after one, a joiner may end a word (Malayalam's chillu letters in
/// their older encoding, `ന്‍`).
fn virama(c: char) -> bool {
    matches!(c, '\u{094D}' | '\u{09CD}' | '\u{0A4D}' | '\u{0ACD}' | '\u{0B4D}' | '\u{0BCD}'
        | '\u{0C4D}' | '\u{0CCD}' | '\u{0D4D}' | '\u{0DCA}')
}

/// Letters (and their signs) of the scripts that are written with joiners.
fn writes_with_joiners(c: char) -> bool {
    matches!(c,
        '\u{0600}'..='\u{06FF}' | '\u{0750}'..='\u{077F}' | '\u{08A0}'..='\u{08FF}'
        | '\u{FB50}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFF}' // Arabic
        | '\u{0700}'..='\u{074F}' // Syriac
        | '\u{07C0}'..='\u{07FF}' // NKo
        | '\u{1800}'..='\u{18AF}' // Mongolian
        | '\u{0900}'..='\u{0DFF}' // Devanagari to Sinhala
        | '\u{0F00}'..='\u{0FFF}' // Tibetan
        | '\u{1000}'..='\u{109F}' // Myanmar
        | '\u{1780}'..='\u{17FF}' // Khmer
        | '\u{A840}'..='\u{A8FF}' | '\u{11000}'..='\u{111FF}' // other Brahmic
    ) && !is_hidden_or_breaking(c)
}

/// Pictographs and symbols emoji are made of.
fn pictographic(c: char) -> bool {
    matches!(c,
        '\u{00A9}' | '\u{00AE}' | '\u{203C}' | '\u{2049}' | '\u{2122}' | '\u{2139}'
        | '\u{2190}'..='\u{21FF}' | '\u{2300}'..='\u{23FF}' | '\u{2460}'..='\u{27BF}'
        | '\u{2900}'..='\u{297F}' | '\u{2B00}'..='\u{2BFF}' | '\u{3030}' | '\u{303D}'
        | '\u{3297}' | '\u{3299}' | '\u{1F000}'..='\u{1FAFF}'
    )
}

/// CJK ideographs.
fn ideographic(c: char) -> bool {
    matches!(c,
        '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}'
        | '\u{20000}'..='\u{3134F}'
    )
}

/// A username as a mail shows it: an account named before the rule above
/// keeps its name, but its hidden and breaking characters are shown as `?`.
fn as_mailed(username: &str) -> String {
    username.chars().map(|c| if is_hidden_or_breaking(c) { '?' } else { c }).collect()
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
    // Characters, as the message says: counted in bytes, an 11-character
    // name in a CJK script was "longer than 32".
    let username_chars = username.chars().count();
    if !(3..=32).contains(&username_chars) {
        err = err.with_field("username", "must be between 3 and 32 characters");
    } else if username.chars().any(is_hidden_or_breaking) || misplaced_joiner(&username) {
        err = err.with_field("username", "must not contain line breaks or invisible characters");
    }
    if !is_bare_address(&email) {
        err = err.with_field("email", "must be a valid email address");
    }
    // Scored here only: the form shows a length hint, not this score.
    if too_weak_off_the_executor(&body.password, &username, &email).await? {
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
        // Matched as the taken check below matches: an expired twin that
        // differs only in joiners or selectors held the name for good, the
        // check refusing it and this never releasing it.
        "DELETE FROM users u
         WHERE (lower(u.username) = lower($1)
                OR regexp_replace(lower(u.username), $3, '', 'g')
                   = regexp_replace(lower($1), $3, '', 'g')
                OR u.email = $2)
           AND u.email_confirmed_at IS NULL AND u.deleted_at IS NULL AND NOT u.is_admin
           AND NOT EXISTS (
               SELECT 1 FROM email_confirmations c
               WHERE c.user_id = u.id AND c.used_at IS NULL AND c.expires_at > now()
           )",
    )
    .bind(&username)
    .bind(&email)
    .bind(INVISIBLE_MARKS)
    .execute(&state.pool)
    .await?;

    let taken = sqlx::query_as::<_, (bool, bool, bool)>(
        // Taken also by a name that differs only in joiners and variation
        // selectors: where a script allows them they may still change nothing
        // a reader sees, so `ب‍ببب` would sit beside `بببب` as a second
        // account (the audit's pass 8).
        "SELECT EXISTS (SELECT 1 FROM users
                        WHERE lower(username) = lower($1)
                           OR regexp_replace(lower(username), $3, '', 'g')
                              = regexp_replace(lower($1), $3, '', 'g')),
                EXISTS (SELECT 1 FROM users WHERE email = $2),
                EXISTS (SELECT 1 FROM users WHERE email = $2 AND email_confirmed_at IS NOT NULL)",
    )
    .bind(&username)
    .bind(&email)
    .bind(INVISIBLE_MARKS)
    .fetch_one(&state.pool)
    .await?;
    let (username_taken, email_taken, email_confirmed) = taken;

    // A taken username is reported plainly: the user has to choose another one
    // to get anywhere, and `GET /api/users` publishes the whole list anyway, so
    // there is nothing here to protect -- and it is answered before the hash,
    // which bought nothing here but a free Argon2 run per request.
    if username_taken {
        return Err(AppError::conflict("registration details are invalid")
            .with_field("username", "that username is taken"));
    }

    // Hashed before the email branch, not after, so both paths pay the same
    // Argon2 cost. Returning an identical body for a taken address and then
    // answering in tens of milliseconds less would give the answer back through
    // timing, which is exactly the flaw this branch exists to avoid.
    let password_hash = api_key::hash_password_off_the_executor(body.password.clone()).await?;

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
            // `{username}` is filled in off the request, below: looking it up
            // here would be a query only this branch makes.
            format!(
                "Someone tried to create a birdtest account with this \
                 address, but it already has one: {{username}}.\n\n\
                 If that was you, sign in at {url}/login as {{username}}, or \
                 reset your password at {url}/reset-password if you have \
                 forgotten it.\n\n\
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
            let (pool, mailer, to) = (state.pool.clone(), state.mailer.clone(), email.clone());
            tokio::spawn(async move {
                let owner: Option<String> =
                    sqlx::query_scalar("SELECT username FROM users WHERE email = $1")
                        .bind(&to)
                        .fetch_optional(&pool)
                        .await
                        .ok()
                        .flatten();
                let owner = owner.as_deref().map(as_mailed);
                let body = notice.replace("{username}", owner.as_deref().unwrap_or("your account"));
                if let Err(err) =
                    mailer.send(&to, "Someone tried to register with your email address", &body).await
                {
                    tracing::error!(error = %err.message, "registration email failed to send");
                }
            });
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
        // Not named: the username is the registrant's own text, and this mail
        // goes to any address they type, so naming it made birdtest carry
        // their words to strangers (the audit's adversarial check).
        format!(
            "Welcome to birdtest.\n\nConfirm your account:\n{link}\n\n\
             If you did not register, ignore this mail.\n"
        ),
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
    // username bounds many guessers aimed at one account. Both are checked
    // before the Argon2 verify, so a limited attempt costs none; the address
    // half before the lookup as well.
    //
    // The username half is ten times the address half. At the per-address
    // rate it let one caller trying a wrong password every six seconds hold
    // any account -- an admin's, whose name `GET /api/users` publishes -- out
    // of signing in for as long as it kept going, correct password or not; at
    // this rate a lockout takes a fleet of addresses, and the cap still bounds
    // how fast a distributed guesser can try. (A per-username-per-address
    // bucket at the address's own rate was tried and could never trip: it
    // only ever saw requests the address's bucket had already let through.)
    ratelimit::check(&state.limits.login, &format!("ip:{ip}"))?;

    let row = sqlx::query_as::<_, (Uuid, String, String, bool, Option<chrono::DateTime<Utc>>, i32)>(
        "SELECT id, username, password_hash, is_admin, email_confirmed_at, session_generation
         FROM users WHERE lower(username) = lower($1) AND deleted_at IS NULL",
    )
    .bind(body.username.trim())
    .fetch_optional(&state.pool)
    .await?;

    // An account's bucket is keyed by the account, not by the name as sent:
    // the lookup matches with the database's `lower`, which Rust's
    // `to_lowercase` does not agree with (`İ` is `i` to one and `i̇` to the
    // other), so one account answered to several buckets. A name that matches
    // no account keeps a bucket of its own, so a 429 says nothing about which
    // names exist.
    let bucket = match &row {
        Some((id, ..)) => format!("id:{id}"),
        None => format!("name:{}", body.username.trim().to_lowercase()),
    };
    ratelimit::check(&state.limits.login_account, &bucket)?;

    // Identical response whether the username is unknown or the password is
    // wrong, so the endpoint cannot be used to enumerate accounts.
    let invalid = || AppError::unauthorized("incorrect username or password");
    let Some((id, username, password_hash, is_admin, confirmed_at, generation)) = row else {
        return Err(invalid());
    };
    if !api_key::verify_password_off_the_executor(body.password.clone(), password_hash.clone()).await? {
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
    ClientIp(ip): ClientIp,
    ApiJson(body): ApiJson<ConfirmEmailBody>,
) -> AppResult<Json<MessageBody>> {
    ratelimit::check(&state.limits.redeem, &format!("ip:{ip}"))?;
    let code_hash = api_key::hash_code(body.code.trim());

    let mut tx = state.pool.begin().await?;
    // The account first, then its code: the order an admin's delete takes
    // them in. Code first, a confirm and a delete of one account deadlocked.
    sqlx::query(
        "SELECT u.id FROM users u JOIN email_confirmations c ON c.user_id = u.id
         WHERE c.code_hash = $1 FOR NO KEY UPDATE OF u",
    )
    .bind(&code_hash)
    .fetch_optional(&mut *tx)
    .await?;
    let user_id = sqlx::query_scalar::<_, Uuid>(
        "UPDATE email_confirmations SET used_at = now()
         WHERE code_hash = $1 AND used_at IS NULL AND expires_at > now()
         RETURNING user_id",
    )
    .bind(&code_hash)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(user_id) = user_id else {
        // The same link opened again -- a double click, a second tab, a mail
        // scanner that followed it first -- is answered as the success it
        // already was. It said "invalid or has expired" and offered to
        // register again, which then said the name was taken. The code is a
        // secret, so answering for it tells nobody else anything.
        let already: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM email_confirmations c
                              JOIN users u ON u.id = c.user_id
                             WHERE c.code_hash = $1 AND c.used_at IS NOT NULL
                               AND u.email_confirmed_at IS NOT NULL AND u.deleted_at IS NULL)",
        )
        .bind(&code_hash)
        .fetch_one(&mut *tx)
        .await?;
        if already {
            return Ok(Json(MessageBody { message: "email already confirmed" }));
        }
        return Err(AppError::bad_request("that confirmation link is invalid or has expired"));
    };

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

/// A reset request's one answer, for every address.
const RESET_REQUESTED: &str = "if that address has an account, a reset link is on its way";

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
    let user = sqlx::query_as::<_, (Uuid, String)>(
        "SELECT id, username FROM users
         WHERE email = $1 AND email_confirmed_at IS NOT NULL AND deleted_at IS NULL",
    )
    .bind(&email)
    .fetch_optional(&state.pool)
    .await?;

    if let Some((user_id, username)) = user {
        let raw_token = api_key::generate_code();
        // Only for an account still there: the lock waits for a delete in
        // progress and then sees its tombstone, where a plain insert waited
        // for it and then went in, and mailed a link to a deleted account.
        let inserted = sqlx::query(
            "INSERT INTO password_reset_tokens (user_id, token_hash, expires_at)
             SELECT id, $2, $3 FROM users WHERE id = $1 AND deleted_at IS NULL FOR KEY SHARE",
        )
        .bind(user_id)
        .bind(api_key::hash_code(&raw_token))
        .bind(Utc::now() + Duration::minutes(RESET_TTL_MINUTES))
        .execute(&state.pool)
        .await?
        .rows_affected();
        if inserted == 0 {
            return Ok(Json(MessageBody { message: RESET_REQUESTED }));
        }

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
                    // The username too: signing in asks for it, and someone who
                    // has forgotten their password may have forgotten that as
                    // well. The mail goes only to the address's owner.
                    &format!(
                        "Your birdtest username is {}.\n\n\
                         Reset your password (valid for {RESET_TTL_MINUTES} minutes):\n{link}\n",
                        as_mailed(&username)
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
    Ok(Json(MessageBody { message: RESET_REQUESTED }))
}

#[derive(Deserialize)]
struct ResetConfirmBody {
    token: String,
    password: String,
}

async fn confirm_password_reset(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    jar: CookieJar,
    ApiJson(body): ApiJson<ResetConfirmBody>,
) -> AppResult<(CookieJar, Json<MessageBody>)> {
    ratelimit::check(&state.limits.redeem, &format!("ip:{ip}"))?;
    let weak = || {
        AppError::bad_request("password is invalid")
            .with_field("password", "too weak — choose a longer, less predictable password")
    };
    let token_hash = api_key::hash_code(body.token.trim());

    // The link first, read without a lock: a wrong or spent one costs neither
    // a score nor a hash, which are close to a second and tens of
    // milliseconds of a password thread's turn. Scored first, a wrong link
    // was enough to spend them.
    let account = sqlx::query_as::<_, (String, String)>(
        "SELECT u.username, u.email FROM password_reset_tokens t JOIN users u ON u.id = t.user_id
         WHERE t.token_hash = $1 AND t.used_at IS NULL AND t.expires_at > now()
           AND u.deleted_at IS NULL",
    )
    .bind(&token_hash)
    .fetch_optional(&state.pool)
    .await?;
    let invalid = || AppError::bad_request("that reset link is invalid or has expired");
    let Some((username, email)) = account else {
        return Err(invalid());
    };
    // A link refused a weak password still works, so each scoring is also
    // counted against the link: replayed from many addresses it was a queue
    // of scorings no per-address limit bounded. Counted once a turn is held,
    // so a request turned away busy (`503`) does not spend the owner's tries.
    let turn = scoring_turn().await?;
    ratelimit::check(&state.limits.reset, &format!("tok:{token_hash}"))?;

    // The same rule registration applies: not the username, not the address.
    // Refused here, the link still works for a better password.
    if score_in_turn(turn, &body.password, &username, &email).await? {
        return Err(weak());
    }

    // Scored and hashed before the transaction: the wait for a turn can be
    // ten seconds under a flood, and inside the transaction it held the
    // account's row locked all that while — against the account's own
    // submissions, which count its tasks on that row.
    let password_hash = api_key::hash_password_off_the_executor(body.password.clone()).await?;

    let mut tx = state.pool.begin().await?;
    // The account first, then its token: the order an admin's delete takes
    // them in. Token first, a reset and a delete deadlocked whenever the
    // account had two reset links out (the delete took the other first). A
    // delete that commits while this waits leaves no account to lock.
    sqlx::query(
        "SELECT u.id FROM users u JOIN password_reset_tokens t ON t.user_id = u.id
         WHERE t.token_hash = $1 AND u.deleted_at IS NULL FOR NO KEY UPDATE OF u",
    )
    .bind(&token_hash)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(invalid)?;
    let user_id = sqlx::query_scalar::<_, Uuid>(
        "UPDATE password_reset_tokens SET used_at = now()
         WHERE token_hash = $1 AND used_at IS NULL AND expires_at > now()
         RETURNING user_id",
    )
    .bind(&token_hash)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(invalid)?;

    // The generation bump signs out every existing session, an attacker's
    // included: resetting a password is what someone does when they suspect
    // another person has access.
    sqlx::query(
        "UPDATE users SET password_hash = $1, session_generation = session_generation + 1
         WHERE id = $2 AND deleted_at IS NULL",
    )
        .bind(&password_hash)
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
    fn a_username_holds_no_line_break_or_hidden_character() {
        for good in ["walker", "Jane Doe", "李小龍", "o'brien-2", "émile"] {
            assert!(!good.chars().any(super::is_hidden_or_breaking), "{good}");
            assert_eq!(super::as_mailed(good), good);
        }
        for bad in [
            "a\nb", "a\r\nb", "tab\there", "a\u{2028}b", "a\u{2029}b", "abc\u{202E}fdp.exe",
            "zero\u{200B}width", "a\u{2066}b", "bom\u{FEFF}", "del\u{7F}", "nel\u{85}", "tag\u{E0041}",
            "walker\u{3164}", "walk\u{034F}er", "\u{2800}\u{2800}\u{2800}",
        ] {
            assert!(bad.chars().any(super::is_hidden_or_breaking), "{bad:?}");
            assert!(!super::as_mailed(bad).chars().any(super::is_hidden_or_breaking), "{bad:?}");
        }
        assert_eq!(super::as_mailed("Prize!\n\nevil.xyz"), "Prize!??evil.xyz");

        // Joiners and selectors belong where scripts and emoji put them.
        for good in [
            "\u{0645}\u{06CC}\u{200C}\u{062E}\u{0648}\u{0627}\u{0647}\u{0645}", // Persian
            "\u{0915}\u{094D}\u{200D}\u{0937}",                                   // Devanagari
            "\u{0DC1}\u{0DCA}\u{200D}\u{0DBB}\u{0DD3}",                            // Sinhala ශ්‍රී
            "\u{1F468}\u{200D}\u{1F469}",
            "\u{1F3F3}\u{FE0F}\u{200D}\u{1F308}",                                  // 🏳️‍🌈
            "\u{2764}\u{FE0F}\u{200D}\u{1F525}",                                   // ❤️‍🔥
            "\u{1F441}\u{FE0F}\u{200D}\u{1F5E8}\u{FE0F}",                           // 👁️‍🗨️
            "\u{2764}\u{FE0F}",
            "1\u{FE0F}\u{20E3}",                                                  // keycap
            "\u{845B}\u{E0100}",                                                  // an ideograph's variant
            "\u{0D2C}\u{0D3E}\u{0D32}\u{0D28}\u{0D4D}\u{200D}",                           // Malayalam, a chillu at the end
        ] {
            assert!(!good.chars().any(super::is_hidden_or_breaking), "{good:?}");
            assert!(!super::misplaced_joiner(good), "{good:?}");
        }
        for bad in [
            "walker\u{200C}", "wal\u{200D}ker", "\u{200C}walker", "a \u{200C}\u{0628}",
            "\u{0628}\u{200C}\u{200C}\u{0628}", "walker\u{FE0F}", "\u{FE0F}walker", "walker\u{E0100}",
            // Non-ASCII Latin and Cyrillic are not scripts that join.
            "zo\u{00EB}\u{FE0F}", "jos\u{00E9}\u{E0100}", "M\u{00FC}\u{200D}\u{00DF}ig",
            "\u{0414}\u{200C}\u{043C}\u{0438}\u{0442}\u{0440}\u{0438}\u{0439}",
            "Mu\u{0308}\u{200D}\u{00DF}ig", "1\u{FE0F}", "\u{1F468}\u{200D}x",
        ] {
            assert!(super::misplaced_joiner(bad), "{bad:?}");
        }
        for hidden in ["walker\u{2065}", "walk\u{FFF0}ers", "wal\u{E0000}ker", "walkerz\u{E0080}", "wal\u{E01F0}ker"] {
            assert!(hidden.chars().any(super::is_hidden_or_breaking), "{hidden:?}");
        }
    }

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
