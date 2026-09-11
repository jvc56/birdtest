use anyhow::Result;
use birdtest::state::AppState;
use birdtest::{artifacts, config, db, email, inputdata, ratelimit, ratings, sse};
use std::net::SocketAddr;
use std::sync::Arc;

/// How often to look for rating pools whose evidence has grown. Ratings are a
/// summary, not a control signal, so minutes of staleness cost nothing while
/// per-submission refits would be pure waste.
const RATING_SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(120);

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

    let app = birdtest::app(state);

    let addr: SocketAddr = cfg.bind_addr.parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "birdtest listening");

    // `ConnectInfo` is the peer address `clientip` falls back to.
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await?;
    Ok(())
}
