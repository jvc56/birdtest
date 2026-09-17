use anyhow::{Context, Result};
use sqlx::postgres::{PgPool, PgPoolOptions};

/// The pool everything that *does* something runs on: claims, submissions,
/// heartbeats, the finish check, admin mutations, the background sweeps.
pub async fn connect(database_url: &str) -> Result<PgPool> {
    PgPoolOptions::new()
        .max_connections(20)
        .connect(database_url)
        .await
        .context("failed to connect to Postgres")
}

/// Connections in the display pool. See [`connect_read`].
pub const READ_POOL_CONNECTIONS: u32 = 8;

/// How long one display read may run before Postgres cancels it.
///
/// Every read on this pool is a page or a dashboard payload, and the slowest
/// measured is a few hundred milliseconds at a full job's volume (PLAN.md,
/// "What these reads cost"). A read that is still going after this long is one
/// whose plan went wrong or whose caller chose its parameters to make it so,
/// and either way nobody is waiting for the page any more.
const READ_STATEMENT_TIMEOUT: &str = "15s";

/// The pool the public pages and the live dashboard read from, separate from
/// [`connect`]'s so that display can never take a connection a worker's claim
/// or submission is waiting for.
///
/// Every route on it is unauthenticated and unmetered, and several of its
/// reads grow with a job's history. On the shared pool that made page views a
/// way to stall the fleet: twenty slow reads at once -- a dashboard left open
/// on a busy job by enough people, or one caller in a loop -- held all twenty
/// connections, and every claim and submission queued behind them until
/// sqlx's acquire timeout failed it. Nothing here decides anything (no
/// statistic is read while dispatching or accepting), so the reads can queue
/// among themselves, behind a bound, and leave the path workers wait on alone.
///
/// Three bounds, each doing a different job: the size caps how many
/// connections display can hold at all; the statement timeout caps how long
/// any one read holds one; and the short acquire timeout turns a saturated
/// pool into a quick `503` with `Retry-After` rather than a request parked for
/// sqlx's default thirty seconds.
pub async fn connect_read(database_url: &str) -> Result<PgPool> {
    PgPoolOptions::new()
        .max_connections(READ_POOL_CONNECTIONS)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .after_connect(|conn, _meta| {
            Box::pin(async move {
                sqlx::query(&format!("SET statement_timeout = '{READ_STATEMENT_TIMEOUT}'"))
                    .execute(conn)
                    .await?;
                Ok(())
            })
        })
        .connect(database_url)
        .await
        .context("failed to connect the display pool to Postgres")
}

/// Applied at startup, before the server binds, so a container never serves
/// traffic against an out-of-date schema.
pub async fn migrate(pool: &PgPool) -> Result<()> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .context("failed to run migrations")
}
