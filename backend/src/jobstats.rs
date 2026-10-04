//! Aggregate statistics for a job. One place computes them, and both the public
//! REST endpoint and the SSE push use it, so a dashboard update over the live
//! stream is byte-for-byte what a page reload would produce.

use crate::error::AppResult;
use crate::models::job::{GameConfig, GamePairConfig, Job, JobType, LeaveConfig, SprtParams};
use crate::stats::sprt::{self, Pentanomial, Sample, SprtResult, Tally};
use serde::Serialize;
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

/// Every `game_results` row of job `$1`: one per task, since a task has one
/// slot.
///
/// Reads the job's rows through `game_results.job_id` rather than by joining
/// `tasks` to find out which rows belong to it. This is the query the finish
/// check runs on the submission path, so it is the one place where dropping a
/// join is worth the most.
pub const GAME_RESULTS: &str = "
    SELECT r.*
    FROM game_results r
    WHERE r.job_id = $1";

#[derive(Debug, Serialize)]
pub struct JobStats {
    pub job: JobSummary,
    pub tasks_total: i64,
    pub tasks_completed: i64,
    pub tasks_available: i64,
    pub tasks_claimed: i64,
    pub results_accepted: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub games: Option<GameStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opening_racks: Option<OpeningRackStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leave_generation: Option<LeaveGenStats>,
    /// The most productive workers on this job, at most
    /// [`MAX_WORKER_CONTRIBUTIONS`] of them. Always serialized, so the client
    /// can read `.length` without a presence check.
    pub workers: Vec<WorkerContribution>,
    /// Contributors beyond the ones listed. A job can have thousands, and the
    /// page renders a table; the list is capped so the payload does not grow
    /// without bound, and this is what the page says instead.
    pub other_workers: i64,
    /// Estimated seconds to completion from recent throughput, or `None` when
    /// there is not enough recent activity to extrapolate.
    pub eta_seconds: Option<f64>,
    /// How a completed job came to be completed; absent while it is not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion: Option<Completion>,
}

/// How a job was completed, from its `job.completed` audit row: a job page
/// said only "completed", and a pairs job stopped by its test and one stopped
/// at its cap read the same.
#[derive(Debug, Serialize)]
pub struct Completion {
    pub at: chrono::DateTime<chrono::Utc>,
    /// Completed by an admin (force-complete) rather than by its own rule.
    pub forced: bool,
    /// The server's reason, when it completed the job: the SPRT verdict
    /// (`passed`, `failed`, `terminated_at_max`), `reached_target` for a
    /// games or pairs job that runs no SPRT and played its games, or `last
    /// generation built`. None for an opening-rack job whose racks were all
    /// analysed.
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct JobSummary {
    pub id: Uuid,
    /// What the admin called it; empty for a job created without one.
    pub name: String,
    pub job_type: JobType,
    pub status: String,
    pub allocation: Option<i32>,
    pub min_magpie_version: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub created_by: Option<String>,
    pub lexicon: Option<String>,
    pub variant: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct GameStats {
    /// "game" for `games` jobs, "pair" for `game_pairs` — the SPRT unit.
    pub unit: &'static str,
    pub wins: u64,
    pub losses: u64,
    pub draws: u64,
    /// Games for a `games` job, pairs for a `game_pairs` job — the unit the
    /// job's min/max thresholds are stated in.
    pub units_completed: u64,
    /// Each player's score per game, and player 1's spread (the difference),
    /// over every game played: the batches' means weighted by their games.
    /// `None` before any game is.
    pub p1_score_mean: Option<f64>,
    pub p2_score_mean: Option<f64>,
    pub spread_mean: Option<f64>,
    /// Game pairs only: the five pair-outcome counts the LLR is computed from,
    /// indexed by player 1's half-point score across the pair.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pentanomial: Option<[u64; 5]>,
    /// Game pairs only: how many pairs diverged. A diagnostic — how often the
    /// two configs actually differ — and not part of the test.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub divergent_pairs: Option<u64>,
    /// Game pairs only: the match score over the games of the pairs that
    /// diverged -- where the two configs actually played differently. A
    /// diagnostic beside the full score, like `divergent_pairs`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub divergent: Option<MatchTally>,
    pub min_units: i32,
    pub max_units: i32,
    pub win_pct: f64,
    pub loss_pct: f64,
    pub draw_pct: f64,
    /// The test over every accepted result, recomputed on each read. `None`
    /// for a job that runs no SPRT: it plays `max_units` and stops, and an LLR
    /// nobody acts on would read as a verdict.
    pub sprt: Option<SprtResult>,
    /// What the job was completed on, when the finish check completed it
    /// (`jobs.sprt_decided_*`). The claims in flight at that moment are still
    /// played and accepted, so `sprt` can move after it -- even back inside
    /// the bounds -- and this is the decision that stands.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decided: Option<SprtDecided>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SprtDecided {
    pub status: String,
    pub llr: f64,
    pub units: i64,
}

/// Progress, and nothing else.
///
/// This used to also carry the average best equity and a breakdown of what the
/// best opening play was (placement, exchange or pass). Both aggregated over
/// every stored move row of the job, which was the most expensive read in the
/// whole payload and grew without bound.
///
/// Nothing is lost from storage: every ranked move is still there, the results
/// listing still returns the best move, score and equity per rack, `?rack=`
/// still returns a rack's full ranked list, and an admin export is the path for
/// analysing the corpus properly. Summarising millions of racks in two numbers
/// on a progress panel was not where that analysis belonged.
#[derive(Debug, Serialize)]
pub struct OpeningRackStats {
    /// Distinct racks with at least one accepted analysis.
    pub racks_analyzed: i64,
    /// Racks that need no more analysis -- the job is done once every rack
    /// is -- and those of them settled at their most analyses without a
    /// consensus. A job wanting one analysis per rack settles each at its
    /// first, so for it `racks_settled` is `racks_analyzed`.
    pub racks_settled: i64,
    pub racks_without_consensus: i64,
    /// Size of the rack space; the denominator for progress.
    pub racks_total: i64,
}

/// Two kinds of figure, and the page says which is which.
///
/// `tasks_completed` and `games_played` are **live**: counters for the
/// in-progress generation, bumped in the submit transaction. Everything about
/// racks is **as of `progress_as_of`**, the generation's last merge: accepted
/// results are staged and folded into the per-rack totals in batches
/// (`leave_gen::merge_staged`), so those figures lag by up to the merge
/// interval mid-generation and by about a minute near its end. They used to be
/// counted from `leave_rack_progress` on every view and every live push -- a
/// pass over 3.2 million rows -- and are now one row read.
#[derive(Debug, Serialize)]
pub struct LeaveGenStats {
    pub current_generation: i32,
    pub generation_count: i32,
    /// Generations whose KLV is built. `current_generation` stops at the last
    /// generation, so it cannot say a finished job's last one closed.
    pub generations_closed: i32,
    /// The in-progress generation's occurrence target, from
    /// `target_rack_counts`, which is every generation's in order.
    pub target_rack_count: i64,
    pub target_rack_counts: Vec<i32>,
    /// Accepted tasks of the in-progress generation, and the games they played.
    pub tasks_completed: i64,
    pub games_played: i64,
    pub racks_at_target: i64,
    pub racks_total: i64,
    /// The rack furthest from target in the in-progress generation.
    pub min_rack: Option<String>,
    pub min_rack_count: Option<i64>,
    /// When the rack figures were computed. `None` before the generation's
    /// universe is seeded.
    pub progress_as_of: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Serialize)]
pub struct WorkerContribution {
    pub user_id: Option<Uuid>,
    /// An anonymous worker's public name, never its UUID (the UUID is its
    /// credential); see `auth::public_anon_id`.
    pub anon_id: Option<String>,
    pub username: Option<String>,
    pub tasks_completed: i64,
    /// Those claims held from claim to submission, as the contributor list
    /// counts compute (`routes::worker::CLAIM_COMPUTE_MS`).
    pub compute_seconds: f64,
}

pub async fn load_job(pool: &PgPool, job_id: Uuid) -> AppResult<Job> {
    load_job_on(&mut *pool.acquire().await?, job_id).await
}

async fn load_job_on(conn: &mut PgConnection, job_id: Uuid) -> AppResult<Job> {
    Ok(sqlx::query_as::<_, Job>("SELECT * FROM jobs WHERE id = $1")
        .bind(job_id)
        .fetch_one(&mut *conn)
        .await?)
}

/// How long `compute` may take before it is worth saying so.
///
/// Every read here is display-only -- nothing in the claim path reads a
/// statistic -- so the cost is bounded by a job's history rather than by
/// anything urgent. The reads that still grow do so slowly (contributions with
/// claims, leave progress with generations, task counts with tasks), and the
/// decision on whether to move them to a background refresh is meant to be
/// made on evidence rather than guessed. This log line is that evidence: when
/// it starts appearing for real jobs, the refresh is worth building.
const SLOW_STATS_THRESHOLD: std::time::Duration = std::time::Duration::from_secs(1);

/// The job's stats as JSON, no older than `max_age`: from the last build
/// when it is recent enough, built (and kept) otherwise. See
/// `Config::stats_cache`.
pub async fn payload(
    pool: &PgPool,
    job: &Job,
    max_age: std::time::Duration,
) -> AppResult<std::sync::Arc<str>> {
    if max_age.is_zero() {
        return Ok(build_payload(pool, job.id, max_age).await?.0);
    }
    if let Some(cached) = cached_payload(job.id, max_age, None) {
        return Ok(cached);
    }
    // One build per job at a time: when a popular job's payload expires,
    // every viewer asking at once would otherwise build it at once. Per job,
    // not one lock for all: a slow build of one job (a second, for a large
    // one) held up every other job's page behind it.
    let asked = std::time::Instant::now();
    let lock = build_lock(job.id);
    let turn = lock.lock().await;
    // A build no older than `max_age` when this request asked answers it,
    // however long it took: judged by age now, a build slower than `max_age`
    // was never shared and every waiter built again in turn; and one that had
    // merely started before the request asked -- the one it waited on --
    // was refused, so on a slow database each burst of viewers built twice
    // (the audit's pass 10).
    // And a build that *failed* after this request asked answers it too, as
    // busy: tried again by each waiter in turn, on a saturated pool each
    // waited out the acquire timeout after the one before, and the tenth
    // viewer had its `503` after fifty seconds (the audit's pass 11).
    let result = match cached_payload(job.id, max_age, Some(asked)) {
        Some(cached) => Ok(cached),
        None if failed_since(job.id, asked) => Err(busy()),
        None => {
            let built = build_payload(pool, job.id, max_age).await.map(|(json, _)| json);
            if built.is_err() {
                FAILED.lock().expect("stats cache poisoned").insert(job.id, std::time::Instant::now());
            }
            built
        }
    };
    drop(turn);
    release_build_lock(job.id, lock);
    result
}

/// When each job's last build failed, for the requests waiting on it.
static FAILED: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<Uuid, std::time::Instant>>> =
    std::sync::LazyLock::new(Default::default);

/// Whether the job's last build failed after `since`; old entries go.
fn failed_since(job_id: Uuid, since: std::time::Instant) -> bool {
    let mut failed = FAILED.lock().expect("stats cache poisoned");
    failed.retain(|_, at| at.elapsed() < std::time::Duration::from_secs(60));
    failed.get(&job_id).is_some_and(|at| *at >= since)
}

/// What a request waiting on a failed build is told: what a pool timeout
/// tells it, since that is what failed builds on a busy server are.
fn busy() -> crate::error::AppError {
    crate::error::AppError {
        retry_after: Some(5),
        ..crate::error::AppError::new(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "unavailable",
            "the server is busy; try again shortly",
        )
    }
}

/// Builds the job's stats as JSON and keeps them for [`payload`] -- `None`
/// if what it built was superseded before it finished (a newer build, or an
/// admin action that [`forget`] was called for), which a live push must not
/// send: it would put a status from before the action back on every page.
pub async fn refresh_payload(
    pool: &PgPool,
    job_id: Uuid,
    max_age: std::time::Duration,
) -> AppResult<Option<std::sync::Arc<str>>> {
    let (json, kept) = build_payload(pool, job_id, max_age).await?;
    Ok((kept || max_age.is_zero()).then_some(json))
}

/// Drops what is kept for the job, for a change a submission does not push:
/// an admin's activate, deactivate, complete, purge or merge, a job
/// completing. Otherwise the admin page, reloading right after the action,
/// read the payload from before it -- and put the old allocation back in
/// the form. A build that started before this is not kept either.
pub fn forget(job_id: Uuid) {
    let now = std::time::Instant::now();
    // Both under the payloads' lock, which a build also holds while it checks
    // and keeps: taken one at a time, a build finishing between them was kept
    // as current, from before the action.
    let mut payloads = PAYLOADS.lock().expect("stats cache poisoned");
    payloads.remove(&job_id);
    // Kept apart from the payloads, and for longer than any build takes:
    // pruned with them after `max_age`, a marker was gone by the time a build
    // slower than that finished, and the build -- begun before the action --
    // was kept and pushed.
    let mut forgotten = FORGOTTEN.lock().expect("stats cache poisoned");
    forgotten.retain(|_, at| now.duration_since(*at) < FORGOTTEN_KEPT);
    forgotten.insert(job_id, now);
    drop(forgotten);
    drop(payloads);
}

/// Longer than any stats build can run: its statements are bounded by the
/// display pool's statement timeout.
const FORGOTTEN_KEPT: std::time::Duration = std::time::Duration::from_secs(3600);

static FORGOTTEN: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<Uuid, std::time::Instant>>> =
    std::sync::LazyLock::new(Default::default);

/// Builds from the job's row as it is when the build starts: one passed in,
/// read before a wait for the build lock, could predate an admin action the
/// build then outlived, and was kept as newer than it.
async fn build_payload(
    pool: &PgPool,
    job_id: Uuid,
    max_age: std::time::Duration,
) -> AppResult<(std::sync::Arc<str>, bool)> {
    // Freshness runs from when the build *started* -- what it read -- and a
    // build replaces only an entry that started before it: two builds
    // finishing out of order left the older one kept.
    let started = std::time::Instant::now();
    let mut conn = pool.acquire().await?;
    let job = load_job_on(&mut conn, job_id).await?;
    let stats = compute_on(&mut conn, &job).await?;
    drop(conn);
    let json: std::sync::Arc<str> = serde_json::to_string(&stats)
        .map_err(|e| crate::error::AppError::internal(format!("serializing job stats failed: {e}")))?
        .into();
    if max_age.is_zero() {
        return Ok((json, true));
    }
    // The payloads' lock first, as `forget` takes them.
    let mut payloads = PAYLOADS.lock().expect("stats cache poisoned");
    let forgotten_after_start = FORGOTTEN
        .lock()
        .expect("stats cache poisoned")
        .get(&job_id)
        .is_some_and(|at| *at > started);
    let superseded = forgotten_after_start
        || payloads.get(&job_id).is_some_and(|entry| entry.started > started);
    payloads.retain(|_, entry| entry.started.elapsed() < max_age);
    if !superseded {
        payloads.insert(job_id, CachedPayload { started, json: json.clone() });
    }
    Ok((json, !superseded))
}

struct CachedPayload {
    started: std::time::Instant,
    json: std::sync::Arc<str>,
}

static PAYLOADS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<Uuid, CachedPayload>>> =
    std::sync::LazyLock::new(Default::default);

type BuildLocks = std::collections::HashMap<Uuid, std::sync::Arc<tokio::sync::Mutex<()>>>;
static BUILD_LOCKS: std::sync::LazyLock<std::sync::Mutex<BuildLocks>> =
    std::sync::LazyLock::new(Default::default);

fn build_lock(job_id: Uuid) -> std::sync::Arc<tokio::sync::Mutex<()>> {
    BUILD_LOCKS.lock().expect("stats build locks poisoned").entry(job_id).or_default().clone()
}

/// Drops the job's lock once nobody else holds or waits on it.
fn release_build_lock(job_id: Uuid, lock: std::sync::Arc<tokio::sync::Mutex<()>>) {
    let mut locks = BUILD_LOCKS.lock().expect("stats build locks poisoned");
    drop(lock);
    if locks.get(&job_id).is_some_and(|l| std::sync::Arc::strong_count(l) == 1) {
        locks.remove(&job_id);
    }
}

/// The kept payload if it is younger than `max_age`, or if it was younger than
/// `max_age` at `since`, whatever its age now.
fn cached_payload(
    job_id: Uuid,
    max_age: std::time::Duration,
    since: Option<std::time::Instant>,
) -> Option<std::sync::Arc<str>> {
    PAYLOADS
        .lock()
        .expect("stats cache poisoned")
        .get(&job_id)
        .filter(|entry| {
            entry.started.elapsed() < max_age
                || since.is_some_and(|since| {
                    // Checked: a configured age past `Instant`'s range would
                    // panic here, under the payloads' lock, and poison it.
                    entry.started.checked_add(max_age).is_none_or(|fresh_until| fresh_until >= since)
                })
        })
        .map(|entry| entry.json.clone())
}

pub async fn compute(pool: &PgPool, job: &Job) -> AppResult<JobStats> {
    compute_on(&mut *pool.acquire().await?, job).await
}

/// [`compute`] on one connection, for a build: taken from the pool for each
/// of its statements, a build on a saturated pool waited out the acquire
/// timeout once per statement and answered in tens of seconds, where one wait
/// makes it a quick `503` (the audit's pass 10).
async fn compute_on(conn: &mut PgConnection, job: &Job) -> AppResult<JobStats> {
    let started = std::time::Instant::now();
    let stats = compute_inner(conn, job).await;
    let elapsed = started.elapsed();
    if elapsed >= SLOW_STATS_THRESHOLD {
        tracing::warn!(
            job_id = %job.id, job_type = ?job.job_type, elapsed_ms = elapsed.as_millis(),
            "job stats took over a second to compute"
        );
    }
    stats
}

async fn compute_inner(conn: &mut PgConnection, job: &Job) -> AppResult<JobStats> {
    let counts = sqlx::query(
        "SELECT
             COUNT(*)                                            AS total,
             COUNT(*) FILTER (WHERE state = 'completed')         AS completed,
             COUNT(*) FILTER (WHERE state = 'available')         AS available,
             COUNT(*) FILTER (WHERE state = 'claimed')           AS claimed,
             COALESCE(SUM(accepted_count), 0)::bigint            AS accepted
         FROM tasks WHERE job_id = $1",
    )
    .bind(job.id)
    .fetch_one(&mut *conn)
    .await?;

    let created_by = match job.created_by {
        Some(id) => {
            sqlx::query_scalar::<_, String>("SELECT username FROM users WHERE id = $1")
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?
        }
        None => None,
    };

    let (lexicon, variant) = lexicon_and_variant(&mut *conn, job).await?;

    let games = game_stats_on(&mut *conn, job).await?;

    let opening_racks = match job.job_type {
        JobType::OpeningRack => Some(opening_rack_stats(&mut *conn, job.id).await?),
        _ => None,
    };

    let leave_generation = match job.job_type {
        JobType::LeaveGeneration => Some(leave_gen_stats(&mut *conn, job.id).await?),
        _ => None,
    };

    let tasks_total: i64 = counts.get("total");
    let tasks_completed: i64 = counts.get("completed");
    let results_accepted: i64 = counts.get("accepted");

    let eta_seconds = estimate_eta(&mut *conn, job, &games, tasks_total, tasks_completed).await?;
    let (workers, other_workers) = worker_contributions_on(&mut *conn, job.id).await?;

    let completion = if job.status == crate::models::job::JobStatus::Completed {
        sqlx::query_as::<_, (chrono::DateTime<chrono::Utc>, bool, Option<String>)>(
            "SELECT created_at, actor_user_id IS NOT NULL, reason FROM audit_log
             WHERE job_id = $1 AND action = 'job.completed' ORDER BY id DESC LIMIT 1",
        )
        .bind(job.id)
        .fetch_optional(&mut *conn)
        .await?
        .map(|(at, forced, reason)| Completion { at, forced, reason })
    } else {
        None
    };

    Ok(JobStats {
        job: JobSummary {
            id: job.id,
            name: job.name.clone(),
            job_type: job.job_type,
            status: status_label(job).to_string(),
            allocation: job.allocation,
            min_magpie_version: job.min_magpie_version().to_string(),
            created_at: job.created_at,
            created_by,
            lexicon,
            variant,
        },
        tasks_total,
        tasks_completed,
        tasks_available: counts.get("available"),
        tasks_claimed: counts.get("claimed"),
        results_accepted,
        games,
        opening_racks,
        leave_generation,
        workers,
        other_workers,
        eta_seconds,
        completion,
    })
}

fn status_label(job: &Job) -> &'static str {
    match job.status {
        crate::models::job::JobStatus::Active => "active",
        crate::models::job::JobStatus::Inactive => "inactive",
        crate::models::job::JobStatus::Completed => "completed",
    }
}

/// The lexicon a job is "on", for display.
///
/// It is no longer a single job-level setting: it lives on the player configs,
/// and a games job may legitimately compare two different lexicons. Distinct
/// names are joined so the dashboard says what is actually being played rather
/// than picking one arbitrarily. Leave generation has one player, so one.
async fn lexicon_and_variant(
    conn: &mut PgConnection,
    job: &Job,
) -> AppResult<(Option<String>, Option<String>)> {
    let query = match job.job_type {
        JobType::OpeningRack => {
            "SELECT DISTINCT d.name
             FROM job_opening_rack_config c
             JOIN player_configs pc ON pc.id = c.player_config_id
             JOIN input_data d ON d.id = pc.kwg_id
             WHERE c.job_id = $1"
        }
        JobType::Games => {
            "SELECT DISTINCT d.name
             FROM job_game_config c
             JOIN player_configs pc
               ON pc.id IN (c.player1_config_id, c.player2_config_id)
             JOIN input_data d ON d.id = pc.kwg_id
             WHERE c.job_id = $1"
        }
        JobType::GamePairs => {
            "SELECT DISTINCT d.name
             FROM job_game_pair_config c
             JOIN player_configs pc
               ON pc.id IN (c.player1_config_id, c.player2_config_id)
             JOIN input_data d ON d.id = pc.kwg_id
             WHERE c.job_id = $1"
        }
        JobType::LeaveGeneration => {
            "SELECT DISTINCT d.name
             FROM job_leave_config c
             JOIN player_configs pc ON pc.id = c.player_config_id
             JOIN input_data d ON d.id = pc.kwg_id
             WHERE c.job_id = $1"
        }
    };
    let mut names = sqlx::query_scalar::<_, String>(query)
        .bind(job.id)
        .fetch_all(&mut *conn)
        .await?;
    names.sort();
    let lexicon = (!names.is_empty()).then(|| names.join(" vs "));
    Ok((lexicon, Some(job.variant.clone())))
}

/// The SPRT-relevant statistics for a games or game-pairs job, and `None` for
/// every other job type.
///
/// This is also what the submission path evaluates the finish conditions on,
/// which is why it is separate from [`compute`]: deciding whether a job is done
/// needs these aggregates and nothing else.
pub async fn game_stats(pool: &PgPool, job: &Job) -> AppResult<Option<GameStats>> {
    game_stats_on(&mut *pool.acquire().await?, job).await
}

async fn game_stats_on(conn: &mut PgConnection, job: &Job) -> AppResult<Option<GameStats>> {
    let mut stats = match job.job_type {
        JobType::Games => plain_game_stats(&mut *conn, job).await?,
        JobType::GamePairs => game_pair_stats(&mut *conn, job).await?,
        JobType::OpeningRack | JobType::LeaveGeneration => return Ok(None),
    };
    if let (Some(status), Some(llr), Some(units)) =
        (&job.sprt_decided_status, job.sprt_decided_llr, job.sprt_decided_units)
    {
        stats.decided = Some(SprtDecided { status: status.clone(), llr, units });
    }
    Ok(Some(stats))
}

/// Sum the per-task aggregates for a plain `games` job. The SPRT unit is a
/// game, so the tally and the unit count are the same number.
async fn plain_game_stats(conn: &mut PgConnection, job: &Job) -> AppResult<GameStats> {
    let config = sqlx::query_as::<_, GameConfig>("SELECT * FROM job_game_config WHERE job_id = $1")
        .bind(job.id)
        .fetch_one(&mut *conn)
        .await?;

    let row = sqlx::query(&format!(
        "SELECT COALESCE(SUM(r.games), 0)::bigint  AS games,
                COALESCE(SUM(r.wins), 0)::bigint   AS wins,
                COALESCE(SUM(r.losses), 0)::bigint AS losses,
                COALESCE(SUM(r.ties), 0)::bigint   AS ties,
                {SCORE_MEANS}
         FROM ({GAME_RESULTS}) r"
    ))
    .bind(job.id)
    .fetch_one(&mut *conn)
    .await?;

    let tally = Tally {
        wins: row.get::<i64, _>("wins") as u64,
        losses: row.get::<i64, _>("losses") as u64,
        draws: row.get::<i64, _>("ties") as u64,
    };
    let games = row.get::<i64, _>("games") as u64;
    let sample = Sample::from_games(&tally);
    Ok(build_game_stats(
        "game",
        tally,
        sample,
        games,
        ScoreMeans::from_row(&row),
        None,
        None,
        &SprtParams::from(&config),
    ))
}

/// A game-pairs job is evaluated on the **pentanomial**: every completed pair,
/// bucketed by player 1's half-point score across it.
///
/// The pair is the independent observation — its two games share a seed, so
/// they are not independent of each other — and the unit `min_pairs` and
/// `max_pairs` bound, so the sample size and the progress count are the same
/// number. Pairs that played identically are 1-1 ties in bucket 2: they stay in
/// the sample, where they pull the variance down. That is where paired play's
/// variance reduction comes from, and it is lost entirely if the sample is
/// filtered to the pairs that diverged, which conditions on the outcome and
/// makes a tiny difference look decisive.
///
/// The per-game win/loss/tie tally is still reported for display, and the
/// divergent counts alongside it as a diagnostic. Neither drives the test.
/// Player 1's record over some games, and each player's mean score and player
/// 1's spread over them (`None` before any).
#[derive(Debug, Serialize)]
pub struct MatchTally {
    pub wins: u64,
    pub losses: u64,
    pub draws: u64,
    pub p1_score_mean: Option<f64>,
    pub p2_score_mean: Option<f64>,
    pub spread_mean: Option<f64>,
}

async fn game_pair_stats(conn: &mut PgConnection, job: &Job) -> AppResult<GameStats> {
    let config =
        sqlx::query_as::<_, GamePairConfig>("SELECT * FROM job_game_pair_config WHERE job_id = $1")
            .bind(job.id)
            .fetch_one(&mut *conn)
            .await?;

    let row = sqlx::query(&format!(
        "SELECT COALESCE(SUM(r.games), 0)::bigint            AS games,
                COALESCE(SUM(r.wins), 0)::bigint             AS wins,
                COALESCE(SUM(r.losses), 0)::bigint           AS losses,
                COALESCE(SUM(r.ties), 0)::bigint             AS ties,
                COALESCE(SUM(r.pent_0), 0)::bigint           AS pent_0,
                COALESCE(SUM(r.pent_1), 0)::bigint           AS pent_1,
                COALESCE(SUM(r.pent_2), 0)::bigint           AS pent_2,
                COALESCE(SUM(r.pent_3), 0)::bigint           AS pent_3,
                COALESCE(SUM(r.pent_4), 0)::bigint           AS pent_4,
                COALESCE(SUM(r.divergent_games), 0)::bigint  AS divergent_games,
                COALESCE(SUM(r.divergent_wins), 0)::bigint   AS divergent_wins,
                COALESCE(SUM(r.divergent_losses), 0)::bigint AS divergent_losses,
                COALESCE(SUM(r.divergent_ties), 0)::bigint   AS divergent_ties,
                SUM(r.divergent_games * r.divergent_p1_score_mean)
                    / NULLIF(SUM(r.divergent_games), 0)      AS divergent_p1_mean,
                SUM(r.divergent_games * r.divergent_p2_score_mean)
                    / NULLIF(SUM(r.divergent_games), 0)      AS divergent_p2_mean,
                {SCORE_MEANS}
         FROM ({GAME_RESULTS}) r"
    ))
    .bind(job.id)
    .fetch_one(&mut *conn)
    .await?;

    let tally = Tally {
        wins: row.get::<i64, _>("wins") as u64,
        losses: row.get::<i64, _>("losses") as u64,
        draws: row.get::<i64, _>("ties") as u64,
    };
    let mut counts = [0u64; 5];
    for (i, slot) in counts.iter_mut().enumerate() {
        *slot = row.get::<i64, _>(format!("pent_{i}").as_str()) as u64;
    }
    let pentanomial = Pentanomial { counts };
    // Pairs played is the sample size and the progress count at once, so these
    // cannot drift apart the way a filtered sample and a full progress count
    // could.
    let pairs_played = pentanomial.pairs();
    let sample = Sample::from_pentanomial(&pentanomial);
    let (p1, p2): (Option<f64>, Option<f64>) =
        (row.get("divergent_p1_mean"), row.get("divergent_p2_mean"));
    let divergent = MatchTally {
        wins: row.get::<i64, _>("divergent_wins") as u64,
        losses: row.get::<i64, _>("divergent_losses") as u64,
        draws: row.get::<i64, _>("divergent_ties") as u64,
        p1_score_mean: p1,
        p2_score_mean: p2,
        spread_mean: p1.zip(p2).map(|(p1, p2)| p1 - p2),
    };
    let mut stats = build_game_stats(
        "pair",
        tally,
        sample,
        pairs_played,
        ScoreMeans::from_row(&row),
        Some(counts),
        Some(row.get::<i64, _>("divergent_games") as u64 / 2),
        &SprtParams::from(&config),
    );
    stats.divergent = Some(divergent);
    Ok(stats)
}

/// Each player's mean score per game over the rows of
/// [`GAME_RESULTS`] aliased `r`, as columns `p1_mean` and
/// `p2_mean`. A batch reports its own means, so they are weighted by its
/// games: an average of the batches' means would count a batch of 2 games as
/// much as one of 200. NULL before any game is played.
const SCORE_MEANS: &str = "
    SUM(r.games * r.p1_score_mean) / NULLIF(SUM(r.games), 0) AS p1_mean,
    SUM(r.games * r.p2_score_mean) / NULLIF(SUM(r.games), 0) AS p2_mean";

/// The two columns [`SCORE_MEANS`] reads.
struct ScoreMeans {
    p1: Option<f64>,
    p2: Option<f64>,
}

impl ScoreMeans {
    fn from_row(row: &sqlx::postgres::PgRow) -> Self {
        Self { p1: row.get("p1_mean"), p2: row.get("p2_mean") }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_game_stats(
    unit: &'static str,
    tally: Tally,
    sample: Sample,
    units_completed: u64,
    scores: ScoreMeans,
    pentanomial: Option<[u64; 5]>,
    divergent_pairs: Option<u64>,
    params: &SprtParams,
) -> GameStats {
    // Percentages describe every game played, for both job types: the sample
    // the LLR runs on is now the same games, viewed as pairs.
    let total = tally.total();
    let pct = |n: u64| {
        if total == 0 {
            0.0
        } else {
            100.0 * n as f64 / total as f64
        }
    };
    let sprt = params.enabled.then(|| {
        sprt::evaluate(
            &sample,
            units_completed,
            params.min_units as u64,
            params.max_units as u64,
            params.alpha,
            params.beta,
            params.elo_low,
            params.elo_high,
        )
    });
    GameStats {
        unit,
        wins: tally.wins,
        losses: tally.losses,
        draws: tally.draws,
        units_completed,
        p1_score_mean: scores.p1,
        p2_score_mean: scores.p2,
        spread_mean: scores.p1.zip(scores.p2).map(|(p1, p2)| p1 - p2),
        pentanomial,
        divergent_pairs,
        divergent: None,
        min_units: params.min_units,
        max_units: params.max_units,
        win_pct: pct(tally.wins),
        loss_pct: pct(tally.losses),
        draw_pct: pct(tally.draws),
        sprt,
        decided: None,
    }
}

async fn opening_rack_stats(conn: &mut PgConnection, job_id: Uuid) -> AppResult<OpeningRackStats> {
    // Two single-row reads, constant time at any job size. The rack counts
    // are `jobs`' running totals, maintained one result at a time in the
    // submit transaction; the aggregates that used to sit beside them here
    // scanned the job's whole history on every detail view and every live
    // push.
    let (racks_analyzed, racks_settled, racks_without_consensus) =
        sqlx::query_as::<_, (i64, i64, i64)>(
            "SELECT racks_analyzed, racks_settled, racks_without_consensus FROM jobs WHERE id = $1",
        )
        .bind(job_id)
        .fetch_one(&mut *conn)
        .await?;

    let racks_total = sqlx::query_scalar::<_, i64>(
        "SELECT total_racks FROM job_opening_rack_config WHERE job_id = $1",
    )
    .bind(job_id)
    .fetch_optional(&mut *conn)
    .await?
    .unwrap_or(0);

    Ok(OpeningRackStats { racks_analyzed, racks_settled, racks_without_consensus, racks_total })
}

async fn leave_gen_stats(conn: &mut PgConnection, job_id: Uuid) -> AppResult<LeaveGenStats> {
    let config =
        sqlx::query_as::<_, LeaveConfig>("SELECT * FROM job_leave_config WHERE job_id = $1")
            .bind(job_id)
            .fetch_one(&mut *conn)
            .await?;

    // Generation 0 is the zeroed KLV generation 1 plays with, not a completed
    // generation; counting it would report generation 2 while generation 1 is
    // still running.
    let completed = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM leave_generation_artifacts WHERE job_id = $1 AND generation >= 1",
    )
    .bind(job_id)
    .fetch_one(&mut *conn)
    .await?;
    let generations_closed = completed as i32;
    let current_generation = (generations_closed + 1).min(config.generation_count());

    // One row, kept by the submit path (the live counters) and by each merge
    // (the rack summary). Absent until the generation's universe is seeded.
    let row = sqlx::query(
        "SELECT tasks_completed, games_played, racks_total, racks_at_target,
                min_rack, min_rack_count, merged_at
         FROM leave_generation_progress WHERE job_id = $1 AND generation = $2",
    )
    .bind(job_id)
    .bind(current_generation)
    .fetch_optional(&mut *conn)
    .await?;

    Ok(LeaveGenStats {
        current_generation,
        generation_count: config.generation_count(),
        target_rack_count: config.target_for(current_generation),
        generations_closed,
        tasks_completed: row.as_ref().map_or(0, |r| r.get("tasks_completed")),
        games_played: row.as_ref().map_or(0, |r| r.get("games_played")),
        racks_at_target: row.as_ref().map_or(0, |r| r.get("racks_at_target")),
        racks_total: row.as_ref().map_or(0, |r| r.get("racks_total")),
        min_rack: row.as_ref().and_then(|r| r.get("min_rack")),
        min_rack_count: row.as_ref().and_then(|r| r.get("min_rack_count")),
        progress_as_of: row.as_ref().and_then(|r| r.get("merged_at")),
        target_rack_counts: config.target_rack_counts,
    })
}

/// How many contributors the job stats name individually.
///
/// The list used to be every worker with an accepted result, unbounded: a
/// popular job has thousands, and all of them were serialized into every detail
/// view and every live push. Matches the API's default page size, which is the
/// size a table on a page is built for.
pub const MAX_WORKER_CONTRIBUTIONS: i64 = 50;

/// The top contributors, and how many more there are.
pub async fn worker_contributions(
    pool: &PgPool,
    job_id: Uuid,
) -> AppResult<(Vec<WorkerContribution>, i64)> {
    worker_contributions_on(&mut *pool.acquire().await?, job_id).await
}

async fn worker_contributions_on(
    conn: &mut PgConnection,
    job_id: Uuid,
) -> AppResult<(Vec<WorkerContribution>, i64)> {
    // One more than the cap, as the list was always read.
    //
    // Grouped by the raw identity first and hashed after the limit: grouped by
    // the pseudonym, the SHA-256 was computed for every completed claim of the
    // job -- a hundred thousand of them for a long job, on every detail view
    // and every live push -- to name at most fifty-one.
    let rows = sqlx::query(&format!(
        "SELECT w.user_id,
                left(encode(sha256(convert_to(w.anon_uuid::text, 'UTF8')), 'hex'), 16) AS anon_id,
                u.username,
                w.tasks_completed, w.compute_ms, w.contributors
         FROM (
             SELECT c.claimed_by_user_id AS user_id, c.claimed_by_anon_uuid AS anon_uuid,
                    COUNT(*)::bigint AS tasks_completed,
                    SUM({compute})::bigint AS compute_ms,
                    COUNT(*) OVER ()::bigint AS contributors
             FROM task_claims c
             JOIN tasks t ON t.id = c.task_id
             WHERE t.job_id = $1 AND c.state = 'completed'
             GROUP BY 1, 2
             ORDER BY 3 DESC, 1, 2
             LIMIT $2
         ) w
         LEFT JOIN users u ON u.id = w.user_id
         ORDER BY w.tasks_completed DESC, w.user_id, w.anon_uuid",
        compute = crate::routes::worker::CLAIM_COMPUTE_MS,
    ))
    .bind(job_id)
    .bind(MAX_WORKER_CONTRIBUTIONS + 1)
    .fetch_all(&mut *conn)
    .await?;

    // How many contributors there are in all, from the same scan: the window
    // count is taken before the LIMIT. (A second grouped scan of the job's
    // claims used to answer it, on every view of a popular job.)
    let other_workers = rows
        .first()
        .map(|r| r.get::<i64, _>("contributors") - MAX_WORKER_CONTRIBUTIONS)
        .unwrap_or(0)
        .max(0);

    Ok((
        rows.into_iter()
            .take(MAX_WORKER_CONTRIBUTIONS as usize)
            .map(|r| WorkerContribution {
                user_id: r.get("user_id"),
                anon_id: r.get("anon_id"),
                username: r.get("username"),
                tasks_completed: r.get("tasks_completed"),
                compute_seconds: r.get::<i64, _>("compute_ms") as f64 / 1000.0,
            })
            .collect(),
        other_workers,
    ))
}

/// Throughput over the last hour, extrapolated to whatever is left. For games
/// and pairs jobs "what's left" is the distance to `max_units`: the target of
/// a job without an SPRT, and a ceiling for one with it -- that job may well
/// stop earlier when the LLR crosses.
async fn estimate_eta(
    conn: &mut PgConnection,
    job: &Job,
    games: &Option<GameStats>,
    tasks_total: i64,
    tasks_completed: i64,
) -> AppResult<Option<f64>> {
    if job.status != crate::models::job::JobStatus::Active {
        return Ok(None);
    }

    // The last hour, or since the job was activated if that is more recent:
    // over a whole hour, a job ten minutes old read six times its real time
    // left, and thirty times at two minutes. At least a minute, so a job's
    // first seconds do not read as a burst.
    //
    // The hour stays a bound of its own, a constant the planner can read, so
    // the completed-claims index serves the scan; the activation is a second
    // condition on the rows it finds. Written as one bound through a CTE, it
    // reached the planner as a parameter it could not estimate, and every
    // live push seq-scanned both tables (~200 ms at two million claims, where
    // this is ~10).
    //
    // The window's length is read in the same statement, from the database's
    // own clock: measured in Rust before the query, it left out the wait for
    // a connection and any skew between the two clocks.
    let (recent, window_seconds) = sqlx::query_as::<_, (i64, f64)>(
        "SELECT (SELECT COUNT(*) FROM task_claims c
                   JOIN tasks t ON t.id = c.task_id
                  WHERE t.job_id = $1 AND c.state = 'completed'
                    AND c.completed_at > now() - interval '1 hour'
                    AND c.completed_at > least(coalesce($2, '-infinity'::timestamptz),
                                               now() - interval '1 minute')),
                EXTRACT(EPOCH FROM now() - greatest(
                    now() - interval '1 hour',
                    least(coalesce($2, '-infinity'::timestamptz), now() - interval '1 minute')
                ))::float8",
    )
    .bind(job.id)
    .bind(job.activated_at)
    .fetch_one(&mut *conn)
    .await?;

    if recent == 0 {
        return Ok(None);
    }
    let per_second = recent as f64 / window_seconds.max(60.0);

    // Tasks are made on demand, so `tasks_total - tasks_completed` is only what
    // is in flight: a 3.2-million-rack job 1% done read "three minutes left".
    // An opening-rack job counts the analyses it still wants instead, at the
    // rate racks have been analysed; a leave job's remaining work is
    // generations whose size depends on the draws, so it has no estimate.
    if job.job_type == crate::models::job::JobType::LeaveGeneration {
        return Ok(None);
    }
    if job.job_type == crate::models::job::JobType::OpeningRack {
        let (total_racks, racks_per_batch, min_results): (i64, i32, i32) = sqlx::query_as(
            "SELECT total_racks, racks_per_batch, min_results_per_rack
             FROM job_opening_rack_config WHERE job_id = $1",
        )
        .bind(job.id)
        .fetch_one(&mut *conn)
        .await?;
        // Racks not yet analysed want at least their fewest analyses each;
        // racks analysed and not yet settled at least one more. A consensus
        // that is slow to come makes this an underestimate.
        let unanalysed = (total_racks - job.racks_analyzed).max(0) as f64;
        let unsettled = (job.racks_analyzed - job.racks_settled).max(0) as f64;
        let remaining = unanalysed * f64::from(min_results.max(1)) + unsettled;
        let racks_per_second = per_second * f64::from(racks_per_batch.max(1));
        return Ok(Some(remaining / racks_per_second));
    }

    if let Some(stats) = games {
        let done = stats.units_completed as f64;
        let target = stats.max_units as f64;
        if done >= target {
            return Ok(Some(0.0));
        }
        // Units per claim from the job's batch size: a task has one slot, and
        // its units count once.
        let per_batch: i32 = if job.job_type == crate::models::job::JobType::GamePairs {
            sqlx::query_scalar("SELECT pairs_per_batch FROM job_game_pair_config WHERE job_id = $1")
        } else {
            sqlx::query_scalar("SELECT games_per_batch FROM job_game_config WHERE job_id = $1")
        }
        .bind(job.id)
        .fetch_one(&mut *conn)
        .await?;
        let units_per_second = per_second * f64::from(per_batch.max(1));
        return Ok(Some((target - done) / units_per_second));
    }

    let remaining = (tasks_total - tasks_completed).max(0) as f64;
    Ok(Some(remaining / per_second))
}
