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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use axum_extra::extract::cookie::Cookie;

    fn jar(token: Option<&str>) -> CookieJar {
        match token {
            Some(t) => CookieJar::new().add(Cookie::new(CSRF_COOKIE, t.to_string())),
            None => CookieJar::new(),
        }
    }

    fn headers(token: Option<&str>) -> HeaderMap {
        let mut h = HeaderMap::new();
        if let Some(t) = token {
            h.insert(CSRF_HEADER, HeaderValue::from_str(t).unwrap());
        }
        h
    }

    /// U-AUTH-7, the safe half: reads need no token at all.
    #[test]
    fn safe_methods_pass_without_any_token() {
        for method in [Method::GET, Method::HEAD, Method::OPTIONS] {
            verify(&method, &headers(None), &jar(None)).unwrap();
        }
    }

    /// U-AUTH-7, the unsafe half: a write needs cookie and header, equal, and
    /// each way of failing says which.
    #[test]
    fn writes_need_a_matching_cookie_and_header() {
        let token = generate_token();
        for method in [Method::POST, Method::PATCH, Method::PUT, Method::DELETE] {
            verify(&method, &headers(Some(&token)), &jar(Some(&token))).unwrap();

            let err = verify(&method, &headers(Some(&token)), &jar(None)).unwrap_err();
            assert_eq!(err.status, axum::http::StatusCode::FORBIDDEN);
            assert_eq!(err.message, "missing CSRF cookie");

            let err = verify(&method, &headers(None), &jar(Some(&token))).unwrap_err();
            assert_eq!(err.message, "missing CSRF header");

            let err = verify(&method, &headers(Some("forged")), &jar(Some(&token))).unwrap_err();
            assert_eq!(err.message, "CSRF token mismatch");
        }
    }

    #[test]
    fn tokens_are_unpredictable() {
        let a = generate_token();
        assert_eq!(hex::decode(&a).unwrap().len(), 24);
        assert_ne!(a, generate_token());
    }
}
