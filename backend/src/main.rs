use anyhow::Result;
use birdtest::state::AppState;
use birdtest::{artifacts, config, db, email, inputdata, ratelimit, ratings, sse};
use std::net::SocketAddr;
use std::sync::Arc;

/// How often to look for rating pools whose evidence has grown. Ratings are a
/// summary, not a control signal, so minutes of staleness cost nothing while
/// per-submission refits would be pure waste.
const RATING_SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(120);

/// How often to drop rate-limit buckets that have gone idle. The buckets
/// themselves refill in seconds to an hour, so this only decides how long an
/// unused entry lingers in memory, not how anyone is limited.
const RATE_LIMIT_SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(600);

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
        result_streams: std::sync::Arc::new(tokio::sync::Semaphore::new(
            birdtest::state::MAX_CONCURRENT_RESULT_STREAMS,
        )),
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
    match birdtest::exports::fail_orphaned(&state.pool).await {
        Ok(0) => {}
        Ok(n) => tracing::warn!(count = n, "failed job exports left running by a restart"),
        Err(err) => tracing::error!(error = %err.message, "could not reap orphaned exports"),
    }

    // Rating fits run on a periodic sweep rather than on result submission: a
    // fit is global to a pool, an active job submits results far faster than
    // any rating needs to move, and nothing in the submission path waits on the
    // answer. SPRT, which *does* gate job completion, stays inline.
    {
        let db = state.pool.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(RATING_SWEEP_INTERVAL);
            // A missed tick under load should not queue up a burst of refits.
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                match ratings::recompute_stale(&db).await {
                    Ok(0) => {}
                    Ok(n) => tracing::info!(pools = n, "refit rating pools"),
                    Err(err) => {
                        tracing::error!(error = %err.message, "rating sweep failed")
                    }
                }
            }
        });
    }

    // Keyed rate limiters hold one entry per key seen, and the keys are
    // outside input; without this the map only ever grows.
    {
        let limits = state.limits.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(RATE_LIMIT_SWEEP_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                limits.retain_recent();
            }
        });
    }

    let app = birdtest::app(state);

    let addr: SocketAddr = cfg.bind_addr.parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "birdtest listening");

    // `ConnectInfo` is the peer address `clientip` falls back to.
    //
    // Shut down gracefully on the signals a container runtime actually sends.
    // Without this, a deployment or a `docker stop` drops every in-flight
    // request: a worker that has just uploaded a completed batch loses it and
    // its retry is answered `accepted: false`, because the claim it was for is
    // still `claimed` and stays that way until the heartbeat timeout. Letting
    // open requests finish costs a few seconds of a rollout.
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let interrupt = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            // ECS stops a task with SIGTERM and only escalates to SIGKILL after
            // the stop timeout, so this is the signal that matters in
            // production.
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(err) => {
                tracing::error!(%err, "could not listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = interrupt => {}
        _ = terminate => {}
    }
    tracing::info!("shutting down; letting in-flight requests finish");
}
