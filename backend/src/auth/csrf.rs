use crate::error::{AppError, AppResult};
use axum::http::{HeaderMap, Method};
use axum_extra::extract::CookieJar;
use rand::RngCore;

pub const CSRF_COOKIE: &str = "birdtest_csrf";
pub const CSRF_HEADER: &str = "x-csrf-token";

pub fn generate_token() -> String {
    let mut bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Double-submit cookie check. Applies only to the session-cookie-backed APIs
/// (auth, account, admin); worker endpoints authenticate with a bearer token or
/// an `X-Worker-UUID` header, neither of which a browser attaches automatically,
/// so they are exempt.
pub fn verify(method: &Method, headers: &HeaderMap, jar: &CookieJar) -> AppResult<()> {
    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return Ok(());
    }
    let cookie = jar
        .get(CSRF_COOKIE)
        .map(|c| c.value().to_string())
        .ok_or_else(|| AppError::forbidden("missing CSRF cookie"))?;
    let header = headers
        .get(CSRF_HEADER)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::forbidden("missing CSRF header"))?;
    if cookie != header {
        return Err(AppError::forbidden("CSRF token mismatch"));
    }
    Ok(())
}
