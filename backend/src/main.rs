mod artifacts;
mod audit;
mod backups;
mod auth;
mod compat;
mod config;
mod db;
mod email;
mod error;
mod jobs;
mod jobstats;
mod models;
mod ratelimit;
mod ratings;
mod routes;
mod scheduler;
mod sse;
mod state;
mod inputdata;
mod stats;
mod version;

use anyhow::Result;
use axum::routing::get;
use axum::Router;
use state::AppState;
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::trace::TraceLayer;

#[tokio::main]
async fn main() -> Result<()> {
    // Local development reads `.env`; in ECS the same variables arrive from the
    // task definition, so a missing file is not an error.
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "birdtest=info,tower_http=info".into()),
        )
        .init();

    let cfg = Arc::new(config::Config::from_env()?);
    let pool = db::connect(&cfg.database_url).await?;

    // Migrations run before the server binds, so a container never serves
    // traffic against an out-of-date schema.
    db::migrate(&pool).await?;

    let state = AppState {
        pool,
        cfg: cfg.clone(),
        sse: sse::SseBroadcaster::new(),
        limits: ratelimit::RateLimiters::new(),
        mailer: email::Mailer::new(cfg.clone()).await,
        artifacts: artifacts::ArtifactStore::new(cfg.clone()).await,
        http: reqwest::Client::builder()
            .user_agent("birdtest")
            .connect_timeout(std::time::Duration::from_secs(30))
            .read_timeout(std::time::Duration::from_secs(120))
            .build()
            .expect("HTTP client"),
    };

    // Single instance: an import row left `running` belongs to a process that
    // is gone, so nothing else can be working on it.
    match inputdata::fail_orphaned_imports(&state.pool).await {
        Ok(0) => {}
        Ok(n) => tracing::warn!(count = n, "failed input data imports left running by a restart"),
        Err(err) => tracing::error!(error = %err.message, "could not reap orphaned imports"),
    }

    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .nest("/api/worker", routes::worker::router())
        .nest("/api/auth", routes::auth::router())
        .merge(routes::account::router())
        .nest("/api/admin", routes::admin::router())
        .nest("/api", routes::public::router())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr: SocketAddr = cfg.bind_addr.parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "birdtest listening");

    // `ConnectInfo` is what per-IP registration rate limiting keys on.
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await?;
    Ok(())
}
