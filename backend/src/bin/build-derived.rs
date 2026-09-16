//! The derived-file builder: drains `derived_data`, then exits.
//!
//! Run as a scheduled ECS task, not inside the web process. A rack info table
//! build peaks at about 2.4 GB of memory and writes a 1.9 GB file, and the web
//! task has 1 vCPU and 2 GB -- it does not fit, and a wordmap's 710 MB peak is
//! a third of that task's memory alongside everything else it is doing. This
//! runs in its own task with its own memory and ephemeral storage, the way the
//! nightly backup does.
//!
//! **Why a schedule rather than an immediate trigger.** The web task could
//! call `ecs:RunTask` the moment an admin creates a job, which would start a
//! build seconds earlier. It would also put an AWS control-plane call on the
//! job-creation path, give the web task permission to run tasks and pass
//! roles, and need its own retry story for a call that can be throttled. A
//! build takes minutes; a poll every few minutes is a small fraction of that,
//! and the queue is the thing that makes the work reliable either way. The
//! admin UI shows the queue (`GET /api/admin/derived-data`), so the wait is
//! visible rather than mysterious.
//!
//! Exits zero when the queue is empty, which is the ordinary outcome of most
//! runs and must not look like a failure.

use anyhow::Result;
use birdtest::{artifacts, config, db, derived, magpie};
use std::sync::Arc;

/// How many files one run builds before it stops.
///
/// A bound rather than "drain everything": each build can take minutes, and a
/// run that keeps going indefinitely overlaps the next scheduled one and pays
/// for two tasks doing the same queue. The lease means the overlap is safe
/// rather than duplicated work, but paying for it is still waste. Whatever is
/// left is picked up by the next run.
const MAX_BUILDS_PER_RUN: usize = 8;

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "birdtest=info".into()),
        )
        .init();

    let cfg = Arc::new(config::Config::from_env()?);
    let magpie = magpie::Magpie::new(&cfg.magpie_bin, cfg.magpie_threads);
    let builders = magpie.builders().await.map_err(|e| {
        anyhow::anyhow!("could not read builder versions from {}: {}", cfg.magpie_bin, e.message)
    })?;
    tracing::info!(
        binary = %cfg.magpie_bin,
        version = %builders.magpie_version,
        target = %builders.build_target,
        threads = cfg.magpie_threads,
        "derived file builder starting"
    );

    // No migrations here. This task and the web task run the same image, and
    // two processes racing to migrate is a worse failure than a builder that
    // waits a deployment for a new column: the web task migrates before it
    // binds, and that is the one place it happens.
    let pool = db::connect(&cfg.database_url).await?;
    let store = artifacts::ArtifactStore::new(cfg.clone()).await;

    let mut built = 0;
    while built < MAX_BUILDS_PER_RUN {
        match derived::build_next(&pool, &store, &magpie, &builders).await {
            // The queue is empty: the ordinary end of most runs.
            Ok(false) => break,
            Ok(true) => built += 1,
            // `build_next` records a build's own failure on its row and
            // returns Ok. Reaching here means the queue itself could not be
            // read, which the next run will hit too, so it stops rather than
            // spinning.
            Err(err) => {
                tracing::error!(error = %err.message, "could not take work from the build queue");
                anyhow::bail!("{}", err.message);
            }
        }
    }

    tracing::info!(built, "derived file builder finished");
    Ok(())
}
