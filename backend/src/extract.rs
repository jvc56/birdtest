//! The JSON request body, with this API's errors.
//!
//! PLAN.md, "Error responses": *every* failure is `{ code, message, fields }`,
//! whatever the status. Handlers return [`AppError`] and get that for free --
//! but a body that does not parse never reaches a handler. With axum's own
//! `Json` extractor it was answered by axum: `text/plain`, and three statuses
//! (`400` malformed, `415` no content type, `422` wrong shape) that are not in
//! the API's list. The frontend's error path and MAGPIE's both read the
//! documented shape, and a claim from a MAGPIE too old to send a body -- the
//! one error PLAN.md singles out as having to name its own fix, because it is
//! what a contributor sees after launch -- was a bare `422`.
//!
//! [`ApiJson`] is that extractor with [`AppError`] as its rejection, and one
//! other difference: **a large body is parsed on the blocking pool.** A result
//! may be up to `MAX_RESULT_BYTES` (64 MiB), and parsing that is a long stretch
//! of computation with no `await` in it; an async worker thread that does not
//! yield can stall every other request the server has (see
//! `exports::upload_rows`). Small bodies -- every claim, heartbeat and decline,
//! and any ordinary result -- are parsed where they stand, since a thread hop
//! costs more than they do.

use crate::error::AppError;
use axum::body::Bytes;
use axum::extract::{FromRequest, Request};
use axum::http::{header, StatusCode};
use serde::de::DeserializeOwned;

/// Bodies at or above this many bytes are parsed on the blocking pool.
const PARSE_INLINE_BELOW: usize = 256 * 1024;

pub struct ApiJson<T>(pub T);

fn is_json(request: &Request) -> bool {
    request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(|mime| {
            let mime = mime.trim().to_ascii_lowercase();
            mime == "application/json" || (mime.starts_with("application/") && mime.ends_with("+json"))
        })
        .unwrap_or(false)
}

fn malformed(err: serde_json::Error) -> AppError {
    AppError::bad_request(format!("the request body is not the JSON this endpoint expects: {err}"))
}

#[axum::async_trait]
impl<S, T> FromRequest<S> for ApiJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Send + 'static,
{
    type Rejection = AppError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        if !is_json(&request) {
            return Err(AppError::bad_request(
                "this endpoint takes a JSON body: send `Content-Type: application/json`",
            ));
        }
        // `Bytes` honours the route's body limit (`DefaultBodyLimit`), which is
        // what refuses an oversized result before it is read, let alone parsed.
        let body = Bytes::from_request(request, state).await.map_err(|rejection| {
            if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
                AppError::new(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "payload_too_large",
                    "the request body is larger than this endpoint accepts",
                )
            } else {
                AppError::bad_request("the request body could not be read")
            }
        })?;

        let value = if body.len() < PARSE_INLINE_BELOW {
            serde_json::from_slice::<T>(&body).map_err(malformed)?
        } else {
            tokio::task::spawn_blocking(move || serde_json::from_slice::<T>(&body))
                .await
                .map_err(|e| AppError::internal(format!("parsing a request body failed: {e}")))?
                .map_err(malformed)?
        };
        Ok(Self(value))
    }
}

/// `axum::extract::Path`, rejecting with an `AppError`.
///
/// axum's own rejection is a plain-text 400: a malformed id in a URL
/// (`/api/jobs/not-a-uuid`) broke the API's promise that every failure is
/// JSON with a `code` and a `message`, and the frontend showed nothing for it.
/// A path that does not parse names nothing, so it is a 404.
pub struct ApiPath<T>(pub T);

#[axum::async_trait]
impl<S, T> axum::extract::FromRequestParts<S> for ApiPath<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Send,
{
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        axum::extract::Path::<T>::from_request_parts(parts, state)
            .await
            .map(|axum::extract::Path(value)| ApiPath(value))
            .map_err(|rejection| {
                use axum::extract::rejection::PathRejection;
                match rejection {
                    // A route and its handler that disagree about the path:
                    // the server's fault, not the request's.
                    PathRejection::MissingPathParams(_) => {
                        AppError::internal(format!("path extraction failed: {rejection}"))
                    }
                    // The parser's own words (UUID grammar and the like)
                    // meant nothing on a page; they go to the debug log.
                    _ => {
                        tracing::debug!(%rejection, "a path that does not parse");
                        AppError::not_found("no such resource")
                    }
                }
            })
    }
}

/// `axum::extract::Query`, rejecting with an `AppError` (400) for the same
/// reason as [`ApiPath`].
pub struct ApiQuery<T>(pub T);

#[axum::async_trait]
impl<S, T> axum::extract::FromRequestParts<S> for ApiQuery<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        axum::extract::Query::<T>::from_request_parts(parts, state)
            .await
            .map(|axum::extract::Query(value)| ApiQuery(value))
            .map_err(|rejection| AppError::bad_request(format!("the query string is invalid: {rejection}")))
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::extract::DefaultBodyLimit;
    use axum::routing::post;
    use axum::Router;
    use tower::ServiceExt;

    #[derive(serde::Deserialize)]
    struct Probe {
        name: String,
    }

    async fn echo(ApiJson(probe): ApiJson<Probe>) -> String {
        probe.name.len().to_string()
    }

    async fn send(limit: usize, content_type: Option<&str>, body: Vec<u8>) -> (StatusCode, serde_json::Value) {
        let app = Router::new().route("/", post(echo).layer(DefaultBodyLimit::max(limit)));
        let mut request = axum::http::Request::post("/");
        if let Some(content_type) = content_type {
            request = request.header(header::CONTENT_TYPE, content_type);
        }
        let response = app.oneshot(request.body(Body::from(body)).unwrap()).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| serde_json::Value::String(String::from_utf8_lossy(&bytes).into_owned()));
        (status, body)
    }

    /// Every way a body can fail to be the JSON a route expects is answered in
    /// the API's own shape and with a status from its list -- not axum's
    /// plain-text 400, 415 and 422.
    #[tokio::test]
    async fn a_body_that_does_not_parse_is_an_api_error() {
        for (what, content_type, body) in [
            ("no body", Some("application/json"), b"".to_vec()),
            ("not json", Some("application/json"), b"not json".to_vec()),
            ("the wrong shape", Some("application/json"), b"{\"name\": 5}".to_vec()),
            ("a missing field", Some("application/json; charset=utf-8"), b"{}".to_vec()),
            ("no content type", None, b"{\"name\": \"x\"}".to_vec()),
        ] {
            let (status, body) = send(1024, content_type, body).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{what}: {body}");
            assert_eq!(body["code"], "bad_request", "{what}: {body}");
            assert!(body["message"].is_string(), "{what}: {body}");
        }
    }

    #[tokio::test]
    async fn a_body_over_the_routes_limit_is_a_413_in_the_same_shape() {
        let body = format!("{{\"name\": \"{}\"}}", "x".repeat(4096)).into_bytes();
        let (status, body) = send(1024, Some("application/json"), body).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
        assert_eq!(body["code"], "payload_too_large", "{body}");
    }

    /// Both sides of the threshold parse to the same thing; the large one does
    /// it on the blocking pool.
    #[tokio::test]
    async fn small_and_large_bodies_parse_alike() {
        for length in [8, PARSE_INLINE_BELOW * 2] {
            let body = format!("{{\"name\": \"{}\"}}", "x".repeat(length)).into_bytes();
            let (status, answer) = send(PARSE_INLINE_BELOW * 4, Some("application/json"), body).await;
            assert_eq!(status, StatusCode::OK, "{answer}");
            assert_eq!(answer, serde_json::json!(length), "{answer}");
        }
    }
}
