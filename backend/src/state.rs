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

/// How many submissions a job takes between finish-condition checks.
///
/// The check reads the job's results to compute the SPRT statistic — one row
/// per task, summed — so it grows with the job's whole history and sits on the
/// path a worker waits on before it can ask for its next task. Running it on
/// every submission is what PLAN.md specified and measured; this spreads it
/// over `SPRT_CHECK_EVERY` of them instead.
///
/// **A debounced check is late, never wrong.** It still reads the rows, so
/// nothing here trades correctness for cost — which is the whole reason this is
/// acceptable where a counter-based stopping rule would not be.
///
/// Overshoot is bounded at `SPRT_CHECK_EVERY - 1` tasks, and the first several
/// of those are free. When the LLR crosses, the job flips to `completed`, but
/// every task already claimed across the fleet is still played and still
/// accepted — the submit path validates the claim, not the job's status. So a
/// value at or below the number of tasks typically in flight wastes nothing
/// that was not already going to be wasted. Eight is well under any fleet worth
/// having, and cuts the read rate by the same factor.
pub const SPRT_CHECK_EVERY: u64 = 8;

/// Submissions seen per job since that job's last finish-condition check.
///
/// In memory rather than in the database: it is a source of truth for nothing,
/// and losing it on a restart costs one extra check. Entries are dropped when a
/// job completes, so this does not grow with the number of jobs ever created.
#[derive(Clone, Default)]
pub struct FinishCheckCounters(Arc<std::sync::Mutex<std::collections::HashMap<uuid::Uuid, u64>>>);

impl FinishCheckCounters {
    /// Counts this submission, and says whether it is the one that checks.
    pub fn should_check(&self, job_id: uuid::Uuid) -> bool {
        let mut counters = self.0.lock().expect("finish-check counters poisoned");
        let count = counters.entry(job_id).or_insert(0);
        *count += 1;
        if *count >= SPRT_CHECK_EVERY {
            *count = 0;
            true
        } else {
            false
        }
    }

    /// Forget a job, once it can never need checking again.
    pub fn forget(&self, job_id: uuid::Uuid) {
        self.0.lock().expect("finish-check counters poisoned").remove(&job_id);
    }
}

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
    /// Debounces the per-submission finish-condition check; see
    /// [`SPRT_CHECK_EVERY`].
    pub finish_checks: FinishCheckCounters,
}
