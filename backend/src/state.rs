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

/// Says when the process has been asked to stop, to the responses that would
/// otherwise never end.
///
/// `axum::serve`'s graceful shutdown stops accepting connections and then waits
/// for every open one to finish. A dashboard's SSE stream never finishes -- it
/// is a keep-alive every fifteen seconds for as long as the tab stays open --
/// so with one page open anywhere, `SIGTERM` was followed by nothing until the
/// container runtime gave up and sent `SIGKILL` (ECS: thirty seconds). The
/// service is a single instance whose old task must be gone before the new one
/// starts, so that was thirty seconds added to every deployment's gap, for a
/// wait that could only ever time out. A stream ends when this is triggered;
/// the page's `EventSource` reconnects by itself, to the new process.
#[derive(Clone)]
pub struct Shutdown(Arc<tokio::sync::watch::Sender<bool>>);

impl Default for Shutdown {
    fn default() -> Self {
        Self(Arc::new(tokio::sync::watch::channel(false).0))
    }
}

impl Shutdown {
    pub fn trigger(&self) {
        self.0.send_replace(true);
    }

    /// Resolves once [`Shutdown::trigger`] has been called, at once if it
    /// already has. The sender lives in `self`, so the channel cannot close
    /// under a waiter.
    pub async fn triggered(&self) {
        let mut stop = self.0.subscribe();
        let _ = stop.wait_for(|stop| *stop).await;
    }
}

#[derive(Clone)]
pub struct AppState {
    /// The pool claims, submissions, heartbeats, the finish check, admin
    /// actions and the background sweeps run on.
    pub pool: PgPool,
    /// The pool the public pages and the live dashboard read from
    /// (`db::connect_read`): smaller, with a statement timeout, and separate so
    /// that nothing display-only can hold a connection a worker is waiting
    /// for. Nothing that decides anything reads through it.
    pub read_pool: PgPool,
    pub cfg: Arc<Config>,
    /// The pinned MAGPIE binary, and what it says about its own builders.
    ///
    /// Resolved once at startup rather than per use: the builder versions are
    /// a constant of the image, and asking the binary on every request would
    /// spawn a process to read three integers. Reading them at startup also
    /// means an image with a broken or missing MAGPIE fails before it binds,
    /// instead of on the first job an admin creates.
    pub magpie: crate::magpie::Magpie,
    pub builders: Arc<crate::magpie::Builders>,
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
    /// The built wordmap and rack-info-table hashes of every job this process
    /// has found dispatchable, so the claim path asks the database once per
    /// job rather than once per claim; see [`crate::derived::DerivedCache`].
    pub derived_ready: crate::derived::DerivedCache,
    /// The immutable configuration of every job this process has dispatched
    /// from or accepted a result for -- its config row, its players, its
    /// letter distribution, its `expected_data` -- read once per job rather
    /// than once per claim inside the dispatch lock; see
    /// [`crate::jobs::dispatch::JobTemplates`].
    pub templates: crate::jobs::dispatch::JobTemplates,
    /// When each leave job's last claim-requested merge started; see
    /// [`crate::jobs::leave_gen::TailMerges`].
    pub leave_merges: crate::jobs::leave_gen::TailMerges,
    /// Ends the responses that never end on their own when the process is told
    /// to stop; see [`Shutdown`].
    pub shutdown: Shutdown,
    /// No claim is reclaimed for a missed heartbeat before this instant: the
    /// process's start plus the heartbeat timeout. See
    /// [`crate::scheduler::reclaim_lapsed`] for what that protects.
    pub reclaim_from: std::time::Instant,
}

impl AppState {
    /// The state the server runs with, built from what `main` establishes
    /// first: the configuration, both pools (after migrating), and the pinned
    /// MAGPIE with its builder versions. Everything else starts empty.
    ///
    /// Here rather than inline in `main` so that a freshly started process --
    /// its restart grace above all -- can be tested as the binary builds it.
    pub async fn new(
        cfg: Arc<Config>,
        pool: PgPool,
        read_pool: PgPool,
        magpie: crate::magpie::Magpie,
        builders: crate::magpie::Builders,
    ) -> Self {
        AppState {
            pool,
            read_pool,
            cfg: cfg.clone(),
            magpie,
            builders: Arc::new(builders),
            sse: SseBroadcaster::new(),
            finish_checks: Default::default(),
            derived_ready: Default::default(),
            templates: Default::default(),
            leave_merges: Default::default(),
            shutdown: Default::default(),
            // A claim's only evidence of life is a heartbeat this process
            // received, and it has received none yet: see
            // `scheduler::reclaim_lapsed`.
            reclaim_from: std::time::Instant::now() + cfg.heartbeat_timeout,
            limits: RateLimiters::new(),
            mailer: Mailer::new(cfg.clone()).await,
            artifacts: ArtifactStore::new(cfg.clone()).await,
            result_streams: Arc::new(Semaphore::new(MAX_CONCURRENT_RESULT_STREAMS)),
            http: reqwest::Client::builder()
                .user_agent("birdtest")
                .connect_timeout(std::time::Duration::from_secs(30))
                .read_timeout(std::time::Duration::from_secs(120))
                .build()
                .expect("HTTP client"),
        }
    }
}
