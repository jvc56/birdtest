//! The birdtest server as a library. `main.rs` is a thin binary over it, and
//! the integration tests in `tests/` build the same router the binary serves
//! rather than a lookalike.

pub mod artifacts;
pub mod audit;
pub mod auth;
pub mod backups;
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
    Router::new()
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
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
