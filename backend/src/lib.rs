//! The birdtest server as a library. `main.rs` is a thin binary over it, and
//! the integration tests in `tests/` build the same router the binary serves
//! rather than a lookalike.

pub mod artifacts;
pub mod audit;
pub mod auth;
pub mod backups;
pub mod board;
pub mod clientip;
pub mod compat;
pub mod config;
pub mod db;
pub mod derived;
pub mod email;
pub mod error;
pub mod exports;
pub mod extract;
pub mod inputdata;
pub mod jobs;
pub mod jobstats;
pub mod magpie;
pub mod magpie_defaults;
pub mod models;
pub mod ratelimit;
pub mod ratings;
pub mod routes;
pub mod scheduler;
pub mod sse;
pub mod state;
pub mod stats;
pub mod version;

use axum::routing::get;
use axum::Router;
use tower_http::trace::TraceLayer;

/// Every route the server answers, with its state attached.
pub fn app(state: state::AppState) -> Router {
    let mut router = Router::new();
    // The local stack's sign-in without a password; absent everywhere else.
    if state.cfg.dev_login {
        router = router.nest("/api/dev", routes::auth::dev_router());
    }
    router
        .route("/health", get(|| async { "ok" }))
        .nest("/api/worker", routes::worker::router())
        .nest("/api/auth", routes::auth::router())
        .merge(routes::account::router())
        .nest("/api/admin", routes::admin::router())
        .nest("/api/admin", routes::ratings::admin_router())
        .nest("/api", routes::public::router())
        .nest("/api", routes::ratings::public_router())
        // Answered in the API's shape, like every other failure: axum's own
        // were an empty 404 and an empty 405.
        .fallback(|| async { error::AppError::not_found("no such endpoint") })
        .method_not_allowed_fallback(|| async {
            error::AppError::new(
                axum::http::StatusCode::METHOD_NOT_ALLOWED,
                "method_not_allowed",
                "that endpoint does not take this method",
            )
        })
        // The pages get their headers from Nginx; the API, which the ALB
        // serves straight from here, did not have this one. Nothing it
        // answers should be sniffed into something else.
        .layer(tower_http::set_header::SetResponseHeaderLayer::if_not_present(
            axum::http::header::X_CONTENT_TYPE_OPTIONS,
            axum::http::HeaderValue::from_static("nosniff"),
        ))
        .layer(compression())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Gzip for the API's JSON and NDJSON, for a client that asks for it.
///
/// Here and not in Nginx: deployed, the ALB sends `/api/*` straight to this
/// process, so a compression Nginx did reached only the local stacks. The
/// paginated results and the results stream compress to a sixth or less --
/// their long key names (`blended_utility`) are most of what gzip takes out.
///
/// Only those two types. Never `text/event-stream`: gzip holds back what it
/// has not yet filled a block with, so a compressed event would sit in the
/// encoder until enough others followed it, and the live pages would stop
/// being live. Nor a redirect, an empty answer or an error of a few bytes:
/// nothing under 256 bytes is worth the header, and those carry no JSON type
/// or no body. A compressed stream is still chunked, so a cut one still shows.
fn compression() -> tower_http::compression::CompressionLayer<impl tower_http::compression::Predicate> {
    use tower_http::compression::predicate::{NotForContentType, Predicate, SizeAbove};
    let json = |_: axum::http::StatusCode,
                _: axum::http::Version,
                headers: &axum::http::HeaderMap,
                _: &axum::http::Extensions| {
        let essence = headers
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .map(str::trim);
        matches!(essence, Some("application/json" | "application/x-ndjson"))
    };
    tower_http::compression::CompressionLayer::new()
        .compress_when(SizeAbove::new(256).and(NotForContentType::SSE).and(json))
}
