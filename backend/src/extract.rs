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

/// How the server reads request bodies, and what that costs it.
///
/// A route's body limit bounds one request, and nothing bounded how many were
/// read at once. The ALB streams `/api` straight to the backend, so a caller
/// with no credentials could hold a few dozen 64 MiB result uploads open, a
/// byte at a time, and the one 2 GB web task was killed for memory with the
/// whole fleet's server in it (thirty-first audit). So bodies are read in two
/// tiers:
///
/// - **Small** -- every body declaring at most [`LARGE_BODY_BYTES`], or
///   declaring nothing: read with no shared budget, so nothing a caller does
///   can make one wait, but each has [`SMALL_BODY_DEADLINE`] to arrive, and one
///   that declares nothing is cut off at [`LARGE_BODY_BYTES`]. The memory they
///   hold together is what callers can send in that time: holding it costs
///   the bytes. (A budget shared with the large tier, as this was first
///   written, let 192 identity-less claims declaring 64 MiB fill it, and every
///   heartbeat, login and ban waited and got `503`: heartbeats are not
///   retried, so in five minutes every claim in the fleet lapsed.)
/// - **Large** -- a body declaring more, which only the result route takes,
///   from an identity it has already checked: its declared length is reserved
///   from [`LargeBodies`] whole before a byte is read, or it is refused at once
///   with `503` and `Retry-After` (MAGPIE retries 5xx for a quarter of an
///   hour). No body holds part of the budget while waiting for more, so honest
///   uploads cannot hold-and-wait each other. One identity may hold
///   [`LARGE_BODY_SHARE_KIB`] of the budget at once, and each body has a
///   deadline in proportion to its size and may not stall.
pub const LARGE_BODY_BYTES: usize = 1024 * 1024;

/// The time a small body has to arrive.
const SMALL_BODY_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30);

/// The large-body budget: three maximum-size results. With the three decodes
/// a large result may run at once (`registry::large_result_turn`), the worst
/// case stays under half the web task's 2 GB.
pub struct LargeBodies {
    kib: tokio::sync::Semaphore,
    /// What each identity with a large body in flight has reserved, in KiB:
    /// at most [`LARGE_BODY_SHARE_KIB`] each.
    owners: std::sync::Mutex<std::collections::BTreeMap<String, u32>>,
    /// A large body has this long, plus its size at [`LARGE_BODY_MIN_RATE`].
    grace: std::time::Duration,
    /// No gap between two of a large body's chunks may be longer.
    stall: std::time::Duration,
}

impl LargeBodies {
    pub const fn new(bytes: usize, grace: std::time::Duration, stall: std::time::Duration) -> Self {
        Self {
            kib: tokio::sync::Semaphore::const_new(bytes / 1024),
            owners: std::sync::Mutex::new(std::collections::BTreeMap::new()),
            grace,
            stall,
        }
    }

    /// What is not reserved right now, in KiB.
    pub fn available_kib(&self) -> usize {
        self.kib.available_permits()
    }
}

/// The most one identity may have reserved at once: one maximum-size result,
/// a third of the budget, or several smaller ones at a time -- machines that
/// share a key share this. (One body per identity, as it was first written,
/// made a fleet on one key send its large results one at a time.)
pub const LARGE_BODY_SHARE_KIB: u32 = (crate::routes::worker::MAX_RESULT_BYTES / 1024) as u32;

pub static LARGE_BODIES: LargeBodies = LargeBodies::new(
    3 * crate::routes::worker::MAX_RESULT_BYTES,
    std::time::Duration::from_secs(30),
    std::time::Duration::from_secs(30),
);

/// The slowest upload a large body's deadline allows for, in bytes a second:
/// half a megabit, so a 64 MiB result has some seventeen minutes. MAGPIE itself
/// gives up only on 120 seconds below a byte a second.
const LARGE_BODY_MIN_RATE: u64 = 64 * 1024;

/// Who a large body belongs to, for its owner's share of the budget: put in the
/// request's extensions by the extractor that checked the caller
/// (`auth::RegisteredWorker`), which runs before the body is read.
#[derive(Clone)]
pub struct BodyOwner(pub String);

/// A large body's reservation, given back when dropped.
pub struct LargeCharge<'a> {
    budget: &'a LargeBodies,
    kib: u32,
    owner: String,
}

impl Drop for LargeCharge<'_> {
    fn drop(&mut self) {
        self.budget.kib.add_permits(self.kib as usize);
        let mut owners = self.budget.owners.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(held) = owners.get_mut(&self.owner) {
            *held -= self.kib;
            if *held == 0 {
                owners.remove(&self.owner);
            }
        }
    }
}

fn unavailable(message: &'static str, retry_after: u64) -> AppError {
    AppError {
        retry_after: Some(retry_after),
        ..AppError::new(StatusCode::SERVICE_UNAVAILABLE, "unavailable", message)
    }
}

fn too_large() -> AppError {
    AppError::new(
        StatusCode::PAYLOAD_TOO_LARGE,
        "payload_too_large",
        "the request body is larger than this endpoint accepts",
    )
}

fn is_length_limit(err: &axum::Error) -> bool {
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(err);
    while let Some(err) = source {
        if err.is::<http_body_util::LengthLimitError>() {
            return true;
        }
        source = err.source();
    }
    false
}

/// Reads `request`'s body under the route's limit and the tier its declared
/// length puts it in. `large` is the budget a large body is reserved from, for
/// the one route that takes them; without it a body declaring more than
/// [`LARGE_BODY_BYTES`] is refused before it is read.
pub async fn read_body<'a>(
    request: Request,
    large: Option<&'a LargeBodies>,
) -> Result<(Bytes, Option<LargeCharge<'a>>), AppError> {
    use axum::RequestExt;
    use tokio_stream::StreamExt;

    // The header, or failing it a length the body knows exactly (one built in
    // memory, as the tests' are). A body cannot send more than it declared:
    // the connection ends it there.
    let declared = request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .or_else(|| axum::body::HttpBody::size_hint(request.body()).exact());
    let started = tokio::time::Instant::now();

    let (charge, deadline, stall, cap) = match declared {
        Some(bytes) if bytes > LARGE_BODY_BYTES as u64 => {
            let (Some(budget), Some(BodyOwner(owner))) = (large, request.extensions().get::<BodyOwner>().cloned())
            else {
                return Err(too_large());
            };
            // More than any route takes is refused by the route's limit as the
            // bytes pass it; nothing past the largest is reserved for it.
            let bytes = bytes.min(crate::routes::worker::MAX_RESULT_BYTES as u64);
            let kib = bytes.div_ceil(1024) as u32;
            let charge = {
                let mut owners = budget.owners.lock().unwrap_or_else(|e| e.into_inner());
                let held = owners.get(&owner).copied().unwrap_or(0);
                if held + kib > LARGE_BODY_SHARE_KIB {
                    return Err(unavailable(
                        "this worker is already sending as much as it may at once; try again shortly",
                        30,
                    ));
                }
                let Ok(permit) = budget.kib.try_acquire_many(kib) else {
                    return Err(unavailable("the server is receiving other large results; try again shortly", 30));
                };
                permit.forget();
                owners.insert(owner.clone(), held + kib);
                LargeCharge { budget, kib, owner }
            };
            let deadline = started + budget.grace + std::time::Duration::from_secs(bytes / LARGE_BODY_MIN_RATE);
            (Some(charge), deadline, Some(budget.stall), bytes as usize)
        }
        _ => (None, started + SMALL_BODY_DEADLINE, None, LARGE_BODY_BYTES),
    };

    let mut stream = request.with_limited_body().into_body().into_data_stream();
    // Reserved, not touched: the pages are resident only as bytes arrive.
    let mut body = Vec::with_capacity(declared.map_or(0, |bytes| (bytes as usize).min(cap)));
    loop {
        let wait_until = match stall {
            Some(stall) => deadline.min(tokio::time::Instant::now() + stall),
            None => deadline,
        };
        let chunk = match tokio::time::timeout_at(wait_until, stream.next()).await {
            Err(_) => return Err(unavailable("the request body did not arrive in time; send it again", 5)),
            Ok(None) => break,
            Ok(Some(Err(err))) if is_length_limit(&err) => return Err(too_large()),
            Ok(Some(Err(_))) => return Err(AppError::bad_request("the request body could not be read")),
            Ok(Some(Ok(chunk))) => chunk,
        };
        if body.len() + chunk.len() > cap {
            // A small body past the small tier's bound (it declared nothing, or
            // less than it sent), or a large one past what it declared.
            return Err(too_large());
        }
        body.extend_from_slice(&chunk);
    }
    Ok((Bytes::from(body), charge))
}

async fn parse<T: DeserializeOwned + Send + 'static>(body: Bytes) -> Result<T, AppError> {
    if body.len() < PARSE_INLINE_BELOW {
        serde_json::from_slice::<T>(&body).map_err(malformed)
    } else {
        tokio::task::spawn_blocking(move || serde_json::from_slice::<T>(&body))
            .await
            .map_err(|e| AppError::task_failed("parsing a request body", e))?
            .map_err(malformed)
    }
}

fn require_json(request: &Request) -> Result<(), AppError> {
    if is_json(request) {
        Ok(())
    } else {
        Err(AppError::bad_request("this endpoint takes a JSON body: send `Content-Type: application/json`"))
    }
}

#[axum::async_trait]
impl<S, T> FromRequest<S> for ApiJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Send + 'static,
{
    type Rejection = AppError;

    async fn from_request(request: Request, _state: &S) -> Result<Self, Self::Rejection> {
        require_json(&request)?;
        // The route's body limit (`DefaultBodyLimit`) refuses an oversized
        // body as it arrives, before it is parsed. A small body only: see
        // `read_body`.
        let (body, _) = read_body(request, None).await?;
        Ok(Self(parse(body).await?))
    }
}

/// [`ApiJson`] for the result route: a body declaring more than
/// [`LARGE_BODY_BYTES`] is reserved from [`LARGE_BODIES`], and the reservation
/// is held until the handler drops it -- the result is kept as text until its
/// job type is known, and then while it waits for its turn to be stored.
pub struct ChargedJson<T>(pub T, pub Option<LargeCharge<'static>>);

#[axum::async_trait]
impl<S, T> FromRequest<S> for ChargedJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Send + 'static,
{
    type Rejection = AppError;

    async fn from_request(request: Request, _state: &S) -> Result<Self, Self::Rejection> {
        require_json(&request)?;
        let (body, charge) = read_body(request, Some(&LARGE_BODIES)).await?;
        Ok(Self(parse(body).await?, charge))
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
                use axum::extract::path::ErrorKind;
                let server_fault = match &rejection {
                    PathRejection::MissingPathParams(_) => true,
                    PathRejection::FailedToDeserializePathParams(failed) => matches!(
                        failed.kind(),
                        ErrorKind::WrongNumberOfParameters { .. } | ErrorKind::UnsupportedType { .. }
                    ),
                    _ => false,
                };
                match rejection {
                    // A route and its handler that disagree about the path:
                    // the server's fault, not the request's.
                    _ if server_fault => {
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

    /// An upload whose bytes the test sends, declaring `declared` (none at
    /// all when `None`), from `owner` if given.
    fn upload(
        declared: Option<usize>,
        owner: Option<&str>,
    ) -> (Request, tokio::sync::mpsc::Sender<Result<Vec<u8>, std::io::Error>>) {
        let (sender, receiver) = tokio::sync::mpsc::channel(4);
        let mut request = axum::http::Request::post("/").header(header::CONTENT_TYPE, "application/json");
        if let Some(declared) = declared {
            request = request.header(header::CONTENT_LENGTH, declared.to_string());
        }
        let mut request =
            request.body(Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(receiver))).unwrap();
        if let Some(owner) = owner {
            request.extensions_mut().insert(BodyOwner(owner.to_string()));
        }
        (request, sender)
    }

    fn large_bodies(mib: usize, stall: std::time::Duration) -> &'static LargeBodies {
        Box::leak(Box::new(LargeBodies::new(mib * 1024 * 1024, std::time::Duration::from_secs(30), stall)))
    }

    const MIB: usize = 1024 * 1024;

    /// A large body is reserved whole before a byte is read -- declared
    /// length, not bytes sent -- and given back, owner and all, when dropped.
    #[tokio::test]
    async fn a_large_body_is_reserved_whole_and_given_back() {
        let budget = large_bodies(4, std::time::Duration::from_secs(5));
        let (request, sender) = upload(Some(2 * MIB), Some("a:one"));
        let reading = tokio::spawn(read_body(request, Some(budget)));
        while budget.available_kib() == 4096 {
            tokio::task::yield_now().await;
        }
        assert_eq!(budget.available_kib(), 2048, "reserved before anything was sent");
        sender.send(Ok(vec![b' '; 2 * MIB])).await.unwrap();
        drop(sender);
        let (body, charge) = reading.await.unwrap().unwrap();
        assert_eq!(body.len(), 2 * MIB);
        drop(charge);
        assert_eq!(budget.available_kib(), 4096);
        assert!(budget.owners.lock().unwrap().is_empty());
    }

    /// Past the budget, and past one owner's share, a large body is refused at
    /// once -- a `503` with `Retry-After`, which MAGPIE retries -- rather than
    /// waiting with part of the budget held: honest uploads could hold-and-wait
    /// each other, and a waiter at the head of the queue held up small ones.
    #[tokio::test]
    async fn a_large_body_that_cannot_be_reserved_is_refused_at_once() {
        let budget = large_bodies(4, std::time::Duration::from_secs(5));
        let (held, _holder) = upload(Some(3 * MIB), Some("a:one"));
        let holding = tokio::spawn(read_body(held, Some(budget)));
        while budget.available_kib() == 4096 {
            tokio::task::yield_now().await;
        }
        for (owner, bytes, what) in [
            ("a:two", 2 * MIB, "past the budget"),
            ("a:one", 62 * MIB, "past one owner's share"),
        ] {
            let (request, _sender) = upload(Some(bytes), Some(owner));
            let started = std::time::Instant::now();
            let err = read_body(request, Some(budget)).await.err().expect(what);
            assert!(started.elapsed() < std::time::Duration::from_secs(1), "{what}: waited");
            assert_eq!(err.status, StatusCode::SERVICE_UNAVAILABLE, "{what}");
            assert!(err.retry_after.is_some(), "{what}");
        }
        assert_eq!(budget.available_kib(), 1024, "the refused gave back nothing they did not hold");
        holding.abort();
    }

    /// A small body never touches the budget, so a full one makes it wait for
    /// nothing; and one declaring nothing is cut off at the small tier's bound,
    /// as is one declaring more than a route without a budget takes.
    #[tokio::test]
    async fn a_small_body_never_waits_and_an_undeclared_one_is_bounded() {
        let budget = large_bodies(4, std::time::Duration::from_secs(5));
        let (held, _holder) = upload(Some(4 * MIB), Some("a:one"));
        let holding = tokio::spawn(read_body(held, Some(budget)));
        while budget.available_kib() > 0 {
            tokio::task::yield_now().await;
        }
        let (request, sender) = upload(Some(100), Some("a:two"));
        sender.send(Ok(vec![b' '; 100])).await.unwrap();
        drop(sender);
        let (body, charge) = read_body(request, Some(budget)).await.unwrap();
        assert_eq!((body.len(), charge.is_none()), (100, true));

        let (request, sender) = upload(None, None);
        let reading = tokio::spawn(read_body(request, Some(budget)));
        sender.send(Ok(vec![b' '; MIB])).await.unwrap();
        sender.send(Ok(vec![b' '; 1])).await.unwrap();
        assert_eq!(reading.await.unwrap().err().expect("cut off").status, StatusCode::PAYLOAD_TOO_LARGE);

        let (request, _sender) = upload(Some(2 * MIB), Some("a:three"));
        let err = read_body(request, None).await.err().expect("no budget, no large body");
        assert_eq!(err.status, StatusCode::PAYLOAD_TOO_LARGE);
        holding.abort();
    }

    /// A large body that stalls is let go, with its reservation.
    #[tokio::test]
    async fn a_stalled_large_body_is_let_go_with_its_reservation() {
        let budget = large_bodies(4, std::time::Duration::from_millis(300));
        let (request, sender) = upload(Some(2 * MIB), Some("a:one"));
        let reading = tokio::spawn(read_body(request, Some(budget)));
        sender.send(Ok(vec![b' '; 1024])).await.unwrap();
        let err = reading.await.unwrap().err().expect("let go");
        assert_eq!(err.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(budget.available_kib(), 4096);
        assert!(budget.owners.lock().unwrap().is_empty());
        drop(sender);
    }
}
