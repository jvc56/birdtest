use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// Every fallible handler returns this. It carries the HTTP status, a stable
/// machine-readable code, and an optional map of field-level errors so form
/// endpoints (registration in particular) can report per-field problems.
#[derive(Debug)]
pub struct AppError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    pub fields: Vec<(String, String)>,
    /// Seconds to put in a `Retry-After` header. Only rate-limit rejections set it.
    pub retry_after: Option<u64>,
    /// The Postgres SQLSTATE, when this error came from the database. Callers
    /// that treat a particular violation as an ordinary outcome -- the
    /// scheduler losing a race on a unique index -- match on this rather than
    /// on the message text, which is localised by the server's `lc_messages`
    /// and is not an interface.
    pub db_code: Option<String>,
}

/// SQLSTATE for a unique-constraint violation.
pub const UNIQUE_VIOLATION: &str = "23505";
/// SQLSTATE for a foreign-key violation.
pub const FOREIGN_KEY_VIOLATION: &str = "23503";
/// SQLSTATE for a statement that gave up waiting for a lock (`lock_timeout`).
pub const LOCK_NOT_AVAILABLE: &str = "55P03";
/// SQLSTATE for a statement Postgres cancelled, which here means the display
/// pool's `statement_timeout` (`db::connect_read`): nothing else sets one.
pub const QUERY_CANCELED: &str = "57014";

#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    fields: Vec<FieldError>,
}

#[derive(Serialize)]
struct FieldError {
    field: String,
    message: String,
}

impl AppError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            fields: Vec::new(),
            retry_after: None,
            db_code: None,
        }
    }

    pub fn with_field(mut self, field: impl Into<String>, message: impl Into<String>) -> Self {
        self.fields.push((field.into(), message.into()));
        self
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", message)
    }
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", message)
    }
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden", message)
    }
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", message)
    }
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, "conflict", message)
    }
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", message)
    }

    /// A 429 telling the caller to wait `retry_after_secs`. Never less than
    /// one second: `Retry-After: 0` reads as "retry now", which is the opposite
    /// of what a rate limit means, and a limiter that computes a sub-second
    /// wait rounds down to exactly that.
    pub fn rate_limited(retry_after_secs: u64) -> Self {
        Self {
            retry_after: Some(retry_after_secs.max(1)),
            ..Self::new(StatusCode::TOO_MANY_REQUESTS, "rate_limited", "too many requests")
        }
    }

    pub fn is_unique_violation(&self) -> bool {
        self.db_code.as_deref() == Some(UNIQUE_VIOLATION)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        if self.status.is_server_error() {
            tracing::error!(code = self.code, message = %self.message, "request failed");
        } else {
            tracing::debug!(code = self.code, message = %self.message, "request rejected");
        }
        let retry_after = self.retry_after;
        // The API conventions promise a client never sees a database error or
        // a stack trace; the detail is in the log line above.
        let message = if self.status.is_server_error() && self.db_code.is_some() {
            "internal error".to_string()
        } else {
            self.message
        };
        let body = ErrorBody {
            code: self.code,
            message,
            fields: self
                .fields
                .into_iter()
                .map(|(field, message)| FieldError { field, message })
                .collect(),
        };
        let mut response = (self.status, axum::Json(body)).into_response();
        if let Some(seconds) = retry_after {
            if let Ok(value) = seconds.to_string().parse() {
                response.headers_mut().insert(axum::http::header::RETRY_AFTER, value);
            }
        }
        response
    }
}

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        match err {
            sqlx::Error::RowNotFound => AppError::not_found("resource not found"),
            sqlx::Error::Database(db) => {
                let code = db.code().map(|c| c.into_owned());
                let mut error = match code.as_deref() {
                    // Two requests racing to create the same row: the caller
                    // lost, and there is nothing wrong with the server.
                    Some(UNIQUE_VIOLATION) => AppError::conflict("that already exists"),
                    Some(FOREIGN_KEY_VIOLATION) => {
                        AppError::conflict("that is still referenced by other records")
                    }
                    // A display read that outran its bound. Not a fault in the
                    // request and not one a retry is sure to fix, but it is
                    // load rather than a bug, and a 500 says the opposite.
                    Some(QUERY_CANCELED) => AppError {
                        retry_after: Some(30),
                        ..AppError::new(
                            StatusCode::SERVICE_UNAVAILABLE,
                            "unavailable",
                            "that read took too long and was cancelled",
                        )
                    },
                    // A statement that gave up waiting for a row lock (the
                    // bounded waits on the worker paths): something long-held
                    // the row -- a purge, a delete -- and a retry after it
                    // finishes gets a clean answer. MAGPIE retries a 5xx.
                    Some(LOCK_NOT_AVAILABLE) => AppError {
                        retry_after: Some(5),
                        ..AppError::new(
                            StatusCode::SERVICE_UNAVAILABLE,
                            "unavailable",
                            "that claim is busy; try again shortly",
                        )
                    },
                    _ => AppError::internal(format!("database error: {db}")),
                };
                if error.status.is_server_error() {
                    error.message = format!("database error: {db}");
                }
                error.db_code = code;
                error
            }
            // Every connection of the pool asked was busy for the whole
            // acquire timeout. That is load, not a fault, and the honest answer
            // is "later": MAGPIE's client backs off and retries a 5xx, and a
            // browser gets a status that says what happened. It used to be a
            // 500 whose body was sqlx's own message.
            sqlx::Error::PoolTimedOut => AppError {
                retry_after: Some(5),
                ..AppError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "unavailable",
                    "the server is busy; try again shortly",
                )
            },
            // A driver-level failure -- a dropped connection, a protocol
            // error, a decode mismatch. Logged in full here, because the
            // response must not carry it: the API conventions promise a client
            // never sees a database error, and only errors with a SQLSTATE
            // were being scrubbed.
            other => {
                tracing::error!(error = %other, "database driver error");
                AppError::internal("internal error")
            }
        }
    }
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        AppError::internal(err.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    /// A saturated pool is load, not a fault: the caller is told to come back,
    /// and is not shown sqlx's own message.
    use axum::body::to_bytes;
    use std::borrow::Cow;

    /// What the client actually receives: status, headers and the JSON body.
    async fn rendered(err: AppError) -> (StatusCode, axum::http::HeaderMap, serde_json::Value) {
        let response = err.into_response();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, headers, serde_json::from_slice(&bytes).unwrap())
    }

    /// A Postgres error as sqlx reports one, carrying the query text and the
    /// connection string the way a real driver message can.
    #[derive(Debug)]
    struct FakeDbError {
        code: &'static str,
        message: String,
    }

    impl std::fmt::Display for FakeDbError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.message)
        }
    }

    impl std::error::Error for FakeDbError {}

    impl sqlx::error::DatabaseError for FakeDbError {
        fn message(&self) -> &str {
            &self.message
        }
        fn code(&self) -> Option<Cow<'_, str>> {
            Some(Cow::Borrowed(self.code))
        }
        fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
            self
        }
        fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
            self
        }
        fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
            self
        }
        fn kind(&self) -> sqlx::error::ErrorKind {
            sqlx::error::ErrorKind::Other
        }
    }

    fn db_error(code: &'static str, message: &str) -> sqlx::Error {
        sqlx::Error::Database(Box::new(FakeDbError { code, message: message.to_string() }))
    }

    /// U-ERR-1: every constructor answers with its documented status and code.
    #[tokio::test]
    async fn each_constructor_maps_to_its_documented_status() {
        let cases = [
            (AppError::bad_request("x"), StatusCode::BAD_REQUEST, "bad_request"),
            (AppError::unauthorized("x"), StatusCode::UNAUTHORIZED, "unauthorized"),
            (AppError::forbidden("x"), StatusCode::FORBIDDEN, "forbidden"),
            (AppError::not_found("x"), StatusCode::NOT_FOUND, "not_found"),
            (AppError::conflict("x"), StatusCode::CONFLICT, "conflict"),
            (AppError::rate_limited(3), StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
            (AppError::internal("x"), StatusCode::INTERNAL_SERVER_ERROR, "internal"),
        ];
        for (err, status, code) in cases {
            let (got_status, _, body) = rendered(err).await;
            assert_eq!(got_status, status, "{code}");
            assert_eq!(body["code"], code);
        }
    }

    /// U-ERR-2: the body always carries `code` and `message`, and field
    /// errors appear under `fields` -- and only when there are some.
    #[tokio::test]
    async fn the_body_carries_code_message_and_any_field_errors() {
        let (_, _, body) = rendered(AppError::bad_request("check the form")).await;
        assert_eq!(body["code"], "bad_request");
        assert_eq!(body["message"], "check the form");
        assert!(body.get("fields").is_none(), "no empty `fields` array: {body}");

        let err = AppError::bad_request("check the form")
            .with_field("username", "is taken")
            .with_field("password", "is too short");
        let (_, _, body) = rendered(err).await;
        assert_eq!(body["code"], "bad_request");
        assert_eq!(body["message"], "check the form");
        assert_eq!(
            body["fields"],
            serde_json::json!([
                {"field": "username", "message": "is taken"},
                {"field": "password", "message": "is too short"},
            ])
        );
    }

    /// U-ERR-3: `rate_limited(n)` sets `Retry-After: n`, never below 1.
    #[tokio::test]
    async fn a_rate_limit_sets_retry_after_and_never_below_one_second() {
        let (_, headers, _) = rendered(AppError::rate_limited(7)).await;
        assert_eq!(headers[axum::http::header::RETRY_AFTER], "7");
        let (_, headers, _) = rendered(AppError::rate_limited(0)).await;
        assert_eq!(headers[axum::http::header::RETRY_AFTER], "1");
        let (_, headers, _) = rendered(AppError::bad_request("x")).await;
        assert!(headers.get(axum::http::header::RETRY_AFTER).is_none());
    }

    /// U-ERR-4: a database failure is a 500 whose public message carries
    /// neither the SQL nor the database URL, while the server-side message
    /// (what is logged) keeps the detail.
    #[tokio::test]
    async fn a_database_error_does_not_leak_the_query_or_the_url() {
        let secret = "syntax error at or near \"SELEC\" in SELEC * FROM users \
                      (postgres://birdtest:hunter2@db.internal:5432/birdtest)";
        let err: AppError = db_error("42601", secret).into();
        assert_eq!(err.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(err.message.contains("SELEC"), "the log keeps the detail: {}", err.message);
        let (status, _, body) = rendered(err).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["code"], "internal");
        let text = body.to_string();
        for leak in ["SELEC", "users", "postgres://", "hunter2", "db.internal"] {
            assert!(!text.contains(leak), "{leak:?} leaked in {text}");
        }
    }

    /// The two violations callers treat as ordinary outcomes keep their
    /// SQLSTATE, and neither says more than that something conflicted.
    #[tokio::test]
    async fn constraint_violations_are_conflicts_without_the_constraint_text() {
        let unique: AppError =
            db_error(UNIQUE_VIOLATION, "duplicate key value violates \"users_email_key\"").into();
        assert!(unique.is_unique_violation());
        let (status, _, body) = rendered(unique).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(!body.to_string().contains("users_email_key"), "{body}");

        let fk: AppError = db_error(FOREIGN_KEY_VIOLATION, "violates \"jobs_config_fk\"").into();
        let (status, _, body) = rendered(fk).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(!body.to_string().contains("jobs_config_fk"), "{body}");
    }

    #[test]
    fn a_pool_timeout_is_a_503_with_retry_after() {
        let err: AppError = sqlx::Error::PoolTimedOut.into();
        assert_eq!(err.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(err.code, "unavailable");
        assert_eq!(err.retry_after, Some(5));
    }

    /// Only errors carrying a SQLSTATE were scrubbed from the response, so a
    /// driver-level failure reached the client verbatim.
    #[test]
    fn a_driver_error_is_not_shown_to_the_client() {
        let err: AppError = sqlx::Error::Protocol("unexpected message 0x45 at byte 7".into()).into();
        assert_eq!(err.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(err.message, "internal error");
    }
}
