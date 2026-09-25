//! Aggregate statistics for a job. One place computes them, and both the public
//! REST endpoint and the SSE push use it, so a dashboard update over the live
//! stream is byte-for-byte what a page reload would produce.

use crate::error::AppResult;
use crate::models::job::{GameConfig, GamePairConfig, Job, JobType, LeaveConfig, SprtParams};
use crate::stats::sprt::{self, Pentanomial, Sample, SprtResult, Tally};
use serde::Serialize;
use sqlx::{PgPool, Row};
use uuid::Uuid;

/// One `game_results` row per task of job `$1`: the first accepted.
///
/// With redundancy above 1 a task has one row per accepted claim, and since
/// games are seeded and deterministic those rows describe the *same* games.
/// Summing all of them would count every game `redundancy` times -- SPRT would
/// see `redundancy` times the evidence it has and stop early on noise, and
/// `min`/`max` gates would trip at a fraction of the games they name. Which
/// copy is used is arbitrary but fixed; reconciling copies that disagree is a
/// cross-check this read deliberately does not attempt (PLAN.md, "Worker
/// Integrity").
///
/// Reads the job's rows through `game_results.job_id` rather than by joining
/// `tasks` to find out which rows belong to it. This is the query the finish
/// check runs on the submission path, so it is the one place where dropping a
/// join is worth the most.
pub const FIRST_GAME_RESULT_PER_TASK: &str = "
    SELECT DISTINCT ON (r.task_id) r.*
    FROM game_results r
    WHERE r.job_id = $1
    ORDER BY r.task_id, r.submitted_at, r.task_claim_id";

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
}

#[derive(Debug, Serialize)]
pub struct JobSummary {
    pub id: Uuid,
    pub job_type: JobType,
    pub status: String,
    pub allocation: Option<i32>,
    pub redundancy: i32,
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
    /// Game pairs only: the five pair-outcome counts the LLR is computed from,
    /// indexed by player 1's half-point score across the pair.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pentanomial: Option<[u64; 5]>,
    /// Game pairs only: how many pairs diverged. A diagnostic — how often the
    /// two configs actually differ — and not part of the test.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub divergent_pairs: Option<u64>,
    pub min_units: i32,
    pub max_units: i32,
    pub win_pct: f64,
    pub loss_pct: f64,
    pub draw_pct: f64,
    /// The test over every accepted result, recomputed on each read.
    pub sprt: SprtResult,
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
/// whole payload and grew without bound; and both counted per *claim* while
/// `racks_analyzed` counts per task, so at `redundancy > 1` the page showed a
/// move-type total of twice the racks it was displayed beside.
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
    pub target_rack_count: i32,
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
}

pub async fn load_job(pool: &PgPool, job_id: Uuid) -> AppResult<Job> {
    Ok(sqlx::query_as::<_, Job>("SELECT * FROM jobs WHERE id = $1")
        .bind(job_id)
        .fetch_one(pool)
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
    // A build that started after this request asked answers it, however long
    // it took: judged by age alone, a build slower than `max_age` was never
    // shared, and every waiter built again in turn.
    let result = match cached_payload(job.id, max_age, Some(asked)) {
        Some(cached) => Ok(cached),
        None => build_payload(pool, job.id, max_age).await.map(|(json, _)| json),
    };
    drop(turn);
    release_build_lock(job.id, lock);
    result
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
    PAYLOADS.lock().expect("stats cache poisoned").remove(&job_id);
    // Kept apart from the payloads, and for longer than any build takes:
    // pruned with them after `max_age`, a marker was gone by the time a build
    // slower than that finished, and the build -- begun before the action --
    // was kept and pushed.
    let mut forgotten = FORGOTTEN.lock().expect("stats cache poisoned");
    forgotten.retain(|_, at| now.duration_since(*at) < FORGOTTEN_KEPT);
    forgotten.insert(job_id, now);
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
    let job = load_job(pool, job_id).await?;
    let stats = compute(pool, &job).await?;
    let json: std::sync::Arc<str> = serde_json::to_string(&stats)
        .map_err(|e| crate::error::AppError::internal(format!("serializing job stats failed: {e}")))?
        .into();
    if max_age.is_zero() {
        return Ok((json, true));
    }
    let forgotten_after_start = FORGOTTEN
        .lock()
        .expect("stats cache poisoned")
        .get(&job_id)
        .is_some_and(|at| *at > started);
    let mut payloads = PAYLOADS.lock().expect("stats cache poisoned");
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

/// The kept payload if it is younger than `max_age`, or if it started after
/// `since` whatever its age.
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
            entry.started.elapsed() < max_age || since.is_some_and(|since| entry.started >= since)
        })
        .map(|entry| entry.json.clone())
}

pub async fn compute(pool: &PgPool, job: &Job) -> AppResult<JobStats> {
    let started = std::time::Instant::now();
    let stats = compute_inner(pool, job).await;
    let elapsed = started.elapsed();
    if elapsed >= SLOW_STATS_THRESHOLD {
        tracing::warn!(
            job_id = %job.id, job_type = ?job.job_type, elapsed_ms = elapsed.as_millis(),
            "job stats took over a second to compute"
        );
    }
    stats
}

async fn compute_inner(pool: &PgPool, job: &Job) -> AppResult<JobStats> {
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
    .fetch_one(pool)
    .await?;

    let created_by = match job.created_by {
        Some(id) => {
            sqlx::query_scalar::<_, String>("SELECT username FROM users WHERE id = $1")
                .bind(id)
                .fetch_optional(pool)
                .await?
        }
        None => None,
    };

    let (lexicon, variant) = lexicon_and_variant(pool, job).await?;

    let games = game_stats(pool, job).await?;

    let opening_racks = match job.job_type {
        JobType::OpeningRack => Some(opening_rack_stats(pool, job.id).await?),
        _ => None,
    };

    let leave_generation = match job.job_type {
        JobType::LeaveGeneration => Some(leave_gen_stats(pool, job.id).await?),
        _ => None,
    };

    let tasks_total: i64 = counts.get("total");
    let tasks_completed: i64 = counts.get("completed");
    let results_accepted: i64 = counts.get("accepted");

    let eta_seconds = estimate_eta(pool, job, &games, tasks_total, tasks_completed).await?;
    let (workers, other_workers) = worker_contributions(pool, job.id).await?;

    Ok(JobStats {
        job: JobSummary {
            id: job.id,
            job_type: job.job_type,
            status: status_label(job).to_string(),
            allocation: job.allocation,
            redundancy: job.redundancy,
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
/// than picking one arbitrarily. Leave generation is the exception -- one bot,
/// one lexicon, on the job config.
async fn lexicon_and_variant(
    pool: &PgPool,
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
             JOIN input_data d ON d.id = c.kwg_id
             WHERE c.job_id = $1"
        }
    };
    let mut names = sqlx::query_scalar::<_, String>(query)
        .bind(job.id)
        .fetch_all(pool)
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
    let mut stats = match job.job_type {
        JobType::Games => plain_game_stats(pool, job).await?,
        JobType::GamePairs => game_pair_stats(pool, job).await?,
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
async fn plain_game_stats(pool: &PgPool, job: &Job) -> AppResult<GameStats> {
    let config = sqlx::query_as::<_, GameConfig>("SELECT * FROM job_game_config WHERE job_id = $1")
        .bind(job.id)
        .fetch_one(pool)
        .await?;

    let row = sqlx::query(&format!(
        "SELECT COALESCE(SUM(r.games), 0)::bigint  AS games,
                COALESCE(SUM(r.wins), 0)::bigint   AS wins,
                COALESCE(SUM(r.losses), 0)::bigint AS losses,
                COALESCE(SUM(r.ties), 0)::bigint   AS ties
         FROM ({FIRST_GAME_RESULT_PER_TASK}) r"
    ))
    .bind(job.id)
    .fetch_one(pool)
    .await?;

    let tally = Tally {
        wins: row.get::<i64, _>("wins") as u64,
        losses: row.get::<i64, _>("losses") as u64,
        draws: row.get::<i64, _>("ties") as u64,
    };
    let games = row.get::<i64, _>("games") as u64;
    let sample = Sample::from_games(&tally);
    Ok(build_game_stats("game", tally, sample, games, None, None, &SprtParams::from(&config)))
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
async fn game_pair_stats(pool: &PgPool, job: &Job) -> AppResult<GameStats> {
    let config =
        sqlx::query_as::<_, GamePairConfig>("SELECT * FROM job_game_pair_config WHERE job_id = $1")
            .bind(job.id)
            .fetch_one(pool)
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
                COALESCE(SUM(r.divergent_games), 0)::bigint  AS divergent_games
         FROM ({FIRST_GAME_RESULT_PER_TASK}) r"
    ))
    .bind(job.id)
    .fetch_one(pool)
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
    Ok(build_game_stats(
        "pair",
        tally,
        sample,
        pairs_played,
        Some(counts),
        Some(row.get::<i64, _>("divergent_games") as u64 / 2),
        &SprtParams::from(&config),
    ))
}

#[allow(clippy::too_many_arguments)]
fn build_game_stats(
    unit: &'static str,
    tally: Tally,
    sample: Sample,
    units_completed: u64,
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
    let sprt = sprt::evaluate(
        &sample,
        units_completed,
        params.min_units as u64,
        params.max_units as u64,
        params.alpha,
        params.beta,
        params.elo_low,
        params.elo_high,
    );
    GameStats {
        unit,
        wins: tally.wins,
        losses: tally.losses,
        draws: tally.draws,
        units_completed,
        pentanomial,
        divergent_pairs,
        min_units: params.min_units,
        max_units: params.max_units,
        win_pct: pct(tally.wins),
        loss_pct: pct(tally.losses),
        draw_pct: pct(tally.draws),
        sprt,
        decided: None,
    }
}

async fn opening_rack_stats(pool: &PgPool, job_id: Uuid) -> AppResult<OpeningRackStats> {
    // Two single-row reads, constant time at any job size. `racks_analyzed` is
    // `jobs.racks_analyzed`, maintained one task at a time in the submit
    // transaction; the aggregates that used to sit beside it here scanned the
    // job's whole history on every detail view and every live push.
    let racks_analyzed =
        sqlx::query_scalar::<_, i64>("SELECT racks_analyzed FROM jobs WHERE id = $1")
            .bind(job_id)
            .fetch_one(pool)
            .await?;

    let racks_total = sqlx::query_scalar::<_, i64>(
        "SELECT total_racks FROM job_opening_rack_config WHERE job_id = $1",
    )
    .bind(job_id)
    .fetch_optional(pool)
    .await?
    .unwrap_or(0);

    Ok(OpeningRackStats { racks_analyzed, racks_total })
}

async fn leave_gen_stats(pool: &PgPool, job_id: Uuid) -> AppResult<LeaveGenStats> {
    let config =
        sqlx::query_as::<_, LeaveConfig>("SELECT * FROM job_leave_config WHERE job_id = $1")
            .bind(job_id)
            .fetch_one(pool)
            .await?;

    // Generation 0 is the zeroed KLV generation 1 plays with, not a completed
    // generation; counting it would report generation 2 while generation 1 is
    // still running.
    let completed = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM leave_generation_artifacts WHERE job_id = $1 AND generation >= 1",
    )
    .bind(job_id)
    .fetch_one(pool)
    .await?;
    let generations_closed = completed as i32;
    let current_generation = (generations_closed + 1).min(config.generation_count);

    // One row, kept by the submit path (the live counters) and by each merge
    // (the rack summary). Absent until the generation's universe is seeded.
    let row = sqlx::query(
        "SELECT tasks_completed, games_played, racks_total, racks_at_target,
                min_rack, min_rack_count, merged_at
         FROM leave_generation_progress WHERE job_id = $1 AND generation = $2",
    )
    .bind(job_id)
    .bind(current_generation)
    .fetch_optional(pool)
    .await?;

    Ok(LeaveGenStats {
        current_generation,
        generation_count: config.generation_count,
        target_rack_count: config.target_rack_count,
        generations_closed,
        tasks_completed: row.as_ref().map_or(0, |r| r.get("tasks_completed")),
        games_played: row.as_ref().map_or(0, |r| r.get("games_played")),
        racks_at_target: row.as_ref().map_or(0, |r| r.get("racks_at_target")),
        racks_total: row.as_ref().map_or(0, |r| r.get("racks_total")),
        min_rack: row.as_ref().and_then(|r| r.get("min_rack")),
        min_rack_count: row.as_ref().and_then(|r| r.get("min_rack_count")),
        progress_as_of: row.as_ref().and_then(|r| r.get("merged_at")),
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
    // One more than the cap, as the list was always read.
    //
    // Grouped by the raw identity first and hashed after the limit: grouped by
    // the pseudonym, the SHA-256 was computed for every completed claim of the
    // job -- a hundred thousand of them for a long job, on every detail view
    // and every live push -- to name at most fifty-one.
    let rows = sqlx::query(
        "SELECT w.user_id,
                left(encode(sha256(convert_to(w.anon_uuid::text, 'UTF8')), 'hex'), 16) AS anon_id,
                u.username,
                w.tasks_completed, w.contributors
         FROM (
             SELECT c.claimed_by_user_id AS user_id, c.claimed_by_anon_uuid AS anon_uuid,
                    COUNT(*)::bigint AS tasks_completed,
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
    )
    .bind(job_id)
    .bind(MAX_WORKER_CONTRIBUTIONS + 1)
    .fetch_all(pool)
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
            })
            .collect(),
        other_workers,
    ))
}

/// Throughput over the last hour, extrapolated to whatever is left. For SPRT
/// jobs "what's left" is the distance to `max_units`, which is a ceiling — the
/// job may well stop earlier when the LLR crosses.
async fn estimate_eta(
    pool: &PgPool,
    job: &Job,
    games: &Option<GameStats>,
    tasks_total: i64,
    tasks_completed: i64,
) -> AppResult<Option<f64>> {
    if job.status != crate::models::job::JobStatus::Active {
        return Ok(None);
    }

    let recent = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM task_claims c
         JOIN tasks t ON t.id = c.task_id
         WHERE t.job_id = $1 AND c.state = 'completed'
           AND c.completed_at > now() - interval '1 hour'",
    )
    .bind(job.id)
    .fetch_one(pool)
    .await?;

    if recent == 0 {
        return Ok(None);
    }
    let per_second = recent as f64 / 3600.0;

    // Tasks are made on demand, so `tasks_total - tasks_completed` is only what
    // is in flight: a 3.2-million-rack job 1% done read "three minutes left".
    // An opening-rack job counts what is left of its rack space instead, at
    // the rate racks have been finishing; a leave job's remaining work is
    // generations whose size depends on the draws, so it has no estimate.
    if job.job_type == crate::models::job::JobType::LeaveGeneration {
        return Ok(None);
    }
    if job.job_type == crate::models::job::JobType::OpeningRack {
        let (total_racks, racks_per_batch): (i64, i32) = sqlx::query_as(
            "SELECT total_racks, racks_per_batch FROM job_opening_rack_config WHERE job_id = $1",
        )
        .bind(job.id)
        .fetch_one(pool)
        .await?;
        let remaining_racks = (total_racks - job.racks_analyzed).max(0) as f64;
        // A claim is one copy of a task, and a task's racks count once.
        let racks_per_second = per_second * f64::from(racks_per_batch.max(1))
            / f64::from(job.redundancy.max(1));
        return Ok(Some(remaining_racks / racks_per_second));
    }

    if let Some(stats) = games {
        let done = stats.units_completed as f64;
        let target = stats.max_units as f64;
        if done >= target {
            return Ok(Some(0.0));
        }
        // Units per claim from the job's batch size, over the redundancy: a
        // claim is one copy of a task, and a task's units count once. (From
        // the observed units per completed task it was neither -- `done`
        // counts a task's first result, `tasks_completed` only tasks with all
        // of theirs -- and at redundancy 2 the page read half the time left.)
        let per_batch: i32 = if job.job_type == crate::models::job::JobType::GamePairs {
            sqlx::query_scalar("SELECT pairs_per_batch FROM job_game_pair_config WHERE job_id = $1")
        } else {
            sqlx::query_scalar("SELECT games_per_batch FROM job_game_config WHERE job_id = $1")
        }
        .bind(job.id)
        .fetch_one(pool)
        .await?;
        let units_per_second =
            per_second * f64::from(per_batch.max(1)) / f64::from(job.redundancy.max(1));
        return Ok(Some((target - done) / units_per_second));
    }

    let remaining = (tasks_total - tasks_completed).max(0) as f64;
    Ok(Some(remaining / per_second))
}
