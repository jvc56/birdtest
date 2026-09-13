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

    pub fn rate_limited(retry_after_secs: u64) -> Self {
        Self {
            retry_after: Some(retry_after_secs),
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
                    _ => AppError::internal(format!("database error: {db}")),
                };
                if error.status.is_server_error() {
                    error.message = format!("database error: {db}");
                }
                error.db_code = code;
                error
            }
            other => AppError::internal(format!("database error: {other}")),
        }
    }
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        AppError::internal(err.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;
