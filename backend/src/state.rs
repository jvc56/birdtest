use crate::artifacts::ArtifactStore;
use crate::config::Config;
use crate::email::Mailer;
use crate::ratelimit::RateLimiters;
use crate::sse::SseBroadcaster;
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::Semaphore;

/// How many result streams may run at once.
///
/// A stream holds a pool connection for as long as its caller keeps reading,
/// and the pool is 20 (`db::connect`). Without a cap a handful of concurrent
/// streams starves dispatch and submission, which is how a read turns into an
/// outage; a rate limit does not bound this, because it limits how often a
/// stream *starts*, not how many are running. These two numbers are one
/// decision: raising the cap without raising the pool takes connections from
/// everything else.
pub const MAX_CONCURRENT_RESULT_STREAMS: usize = 2;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub cfg: Arc<Config>,
    pub sse: SseBroadcaster,
    pub limits: RateLimiters,
    pub mailer: Mailer,
    pub artifacts: ArtifactStore,
    /// Shared HTTP client, used only by the admin import path. Cloning it is
    /// cheap and shares the connection pool.
    pub http: reqwest::Client,
    /// Permits for [`MAX_CONCURRENT_RESULT_STREAMS`]. A stream holds its permit
    /// until the response body is dropped, so a caller that disconnects
    /// mid-scan releases it just as one that reads to the end does.
    pub result_streams: Arc<Semaphore>,
}
