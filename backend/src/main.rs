use anyhow::Result;
use birdtest::state::AppState;
use birdtest::{config, db, inputdata, ratings};
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

/// How often to expire staged input-data imports nobody confirmed. They
/// expire after a day (`inputdata::UNCONFIRMED_IMPORT_TTL`), so an hourly
/// look is plenty.
const IMPORT_EXPIRY_SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3600);

/// How often to thin each pool's rating runs older than
/// `ratings::RUN_FULL_RESOLUTION` down to one a day. The window is a month, so
/// an hourly look is plenty; past the first pass each one deletes an hour's
/// worth of runs.
const RATING_RUN_THIN_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3600);

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

    // Before anything else: the server builds every wordmap, rack info table
    // and leave-generation KLV with this binary, and records the builder
    // version beside each hash. An image without a working MAGPIE can dispatch
    // nothing that needs one, so it fails here rather than on the first job an
    // admin creates.
    let magpie = birdtest::magpie::Magpie::new(&cfg.magpie_bin, cfg.magpie_threads);
    let builders = magpie.builders().await.map_err(|e| {
        anyhow::anyhow!(
            "could not read builder versions from {}: {}. Set MAGPIE_BIN to a MAGPIE \
             of at least {}.",
            cfg.magpie_bin,
            e.message,
            cfg.min_magpie_version
        )
    })?;
    if birdtest::version::Version::parse_or_zero(&builders.magpie_version)
        < birdtest::version::Version::parse_or_zero(&cfg.min_magpie_version)
    {
        anyhow::bail!(
            "the pinned MAGPIE at {} is {}, below this server's floor of {}. It would hand \
             workers hashes built by a builder they are not allowed to run.",
            cfg.magpie_bin,
            builders.magpie_version,
            cfg.min_magpie_version
        );
    }
    tracing::info!(
        binary = %cfg.magpie_bin,
        version = %builders.magpie_version,
        target = %builders.build_target,
        wmp = %builders.wmp(),
        rit = %builders.rit(),
        klv = %builders.klv(),
        "pinned MAGPIE"
    );

    let pool = db::connect(&cfg.database_url).await?;

    // Migrations run before the server binds, so a container never serves
    // traffic against an out-of-date schema.
    db::migrate(&pool).await?;

    // After the migration, so its connections never see the old schema.
    let read_pool = db::connect_read(&cfg.database_url).await?;

    let state = AppState::new(cfg.clone(), pool, read_pool, magpie, builders).await;

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

    // Likewise a leave-generation transition: it runs on a spawned task, so one
    // left open belongs to a process that is gone, and the next claim should
    // take it over now rather than after the half-hour takeover timeout.
    match birdtest::jobs::leave_gen::release_orphaned_transitions(&state.pool).await {
        Ok(0) => {}
        Ok(n) => tracing::warn!(count = n, "released generation transitions left open by a restart"),
        Err(err) => tracing::error!(error = %err.message, "could not release orphaned transitions"),
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

    // A staged import is a proposal an admin was shown and did not act on.
    // Left forever it holds its staged rows and their bytes, and shows a diff
    // against a vocabulary that has since moved on.
    {
        let db = state.pool.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(IMPORT_EXPIRY_SWEEP_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                match inputdata::expire_unconfirmed_imports(&db).await {
                    Ok(0) => {}
                    Ok(n) => tracing::info!(count = n, "expired unconfirmed input data imports"),
                    Err(err) => {
                        tracing::error!(error = %err.message, "expiring unconfirmed imports failed")
                    }
                }
            }
        });
    }

    // Rating runs are snapshots, one per fit, and a pool with an active job
    // takes one every two minutes for as long as the job runs. Past a month the
    // last run of each day is all the history chart can show anyway.
    {
        let db = state.pool.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(RATING_RUN_THIN_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                match ratings::thin_old_runs(&db).await {
                    Ok(0) => {}
                    Ok(n) => tracing::info!(runs = n, "thinned old rating runs"),
                    Err(err) => {
                        tracing::error!(error = %err.message, "thinning old rating runs failed")
                    }
                }
            }
        });
    }

    // Accepted leave results are staged by the submit path and folded into the
    // per-rack totals here, in one pass per job, rather than by every
    // submission; see `leave_gen::stage_fold` for what that saves. Claims ask
    // for a merge themselves near a generation's end, and a transition drains
    // before it reads, so this interval sets write volume and dashboard lag
    // and nothing else.
    {
        let db = state.pool.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(birdtest::jobs::leave_gen::MERGE_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                match birdtest::jobs::leave_gen::merge_all_staged(&db).await {
                    Ok(_) => {}
                    Err(err) => {
                        tracing::error!(error = %err.message, "merging staged leave results failed")
                    }
                }
            }
        });
    }

    let shutdown = state.shutdown.clone();
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
    //
    // The dashboards' SSE streams are told as well (`state::Shutdown`): they
    // are requests that never finish, and waiting for one is waiting for the
    // runtime's SIGKILL.
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            shutdown.trigger();
        })
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
