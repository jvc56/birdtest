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
pub const FIRST_GAME_RESULT_PER_TASK: &str = "
    SELECT DISTINCT ON (r.task_id) r.*
    FROM game_results r
    JOIN tasks t ON t.id = r.task_id
    WHERE t.job_id = $1
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
    pub priority: i32,
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
    pub sprt: SprtResult,
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

#[derive(Debug, Serialize)]
pub struct LeaveGenStats {
    pub current_generation: i32,
    pub generation_count: i32,
    pub target_rack_count: i32,
    pub racks_at_target: i64,
    pub racks_total: i64,
    /// The rack furthest from target in the in-progress generation, live on
    /// every accepted result — sourced from `leave_rack_progress`, not from any
    /// single worker's heartbeat.
    pub min_rack: Option<String>,
    pub min_rack_count: Option<i64>,
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
            priority: job.priority,
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
    Ok(match job.job_type {
        JobType::Games => Some(plain_game_stats(pool, job).await?),
        JobType::GamePairs => Some(game_pair_stats(pool, job).await?),
        JobType::OpeningRack | JobType::LeaveGeneration => None,
    })
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
    let current_generation = (completed as i32 + 1).min(config.generation_count);

    let row = sqlx::query(
        "SELECT COUNT(*)::bigint AS total,
                COUNT(*) FILTER (WHERE occurrence_count >= $3)::bigint AS at_target
         FROM leave_rack_progress WHERE job_id = $1 AND generation = $2",
    )
    .bind(job_id)
    .bind(current_generation)
    .bind(config.target_rack_count as i64)
    .fetch_one(pool)
    .await?;

    let min = sqlx::query(
        "SELECT rack, occurrence_count FROM leave_rack_progress
         WHERE job_id = $1 AND generation = $2
         ORDER BY occurrence_count ASC, rack ASC LIMIT 1",
    )
    .bind(job_id)
    .bind(current_generation)
    .fetch_optional(pool)
    .await?;

    Ok(LeaveGenStats {
        current_generation,
        generation_count: config.generation_count,
        target_rack_count: config.target_rack_count,
        racks_at_target: row.get("at_target"),
        racks_total: row.get("total"),
        min_rack: min.as_ref().map(|r| r.get("rack")),
        min_rack_count: min.as_ref().map(|r| r.get("occurrence_count")),
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
    // One more than the cap, so "are there others" needs no second query when
    // the job has few contributors -- which is the common case.
    let rows = sqlx::query(
        "SELECT c.claimed_by_user_id AS user_id,
                left(encode(sha256(convert_to(c.claimed_by_anon_uuid::text, 'UTF8')), 'hex'), 16) AS anon_id,
                u.username,
                COUNT(*)::bigint AS tasks_completed
         FROM task_claims c
         JOIN tasks t ON t.id = c.task_id
         LEFT JOIN users u ON u.id = c.claimed_by_user_id
         WHERE t.job_id = $1 AND c.state = 'completed'
         GROUP BY 1, 2, 3
         ORDER BY 4 DESC
         LIMIT $2",
    )
    .bind(job_id)
    .bind(MAX_WORKER_CONTRIBUTIONS + 1)
    .fetch_all(pool)
    .await?;

    // Only when the cap was actually reached is a full count worth paying for.
    let other_workers = if rows.len() as i64 > MAX_WORKER_CONTRIBUTIONS {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM (
                 SELECT 1 FROM task_claims c
                 JOIN tasks t ON t.id = c.task_id
                 WHERE t.job_id = $1 AND c.state = 'completed'
                 GROUP BY c.claimed_by_user_id, c.claimed_by_anon_uuid
             ) w",
        )
        .bind(job_id)
        .fetch_one(pool)
        .await?
            - MAX_WORKER_CONTRIBUTIONS
    } else {
        0
    };

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

    let remaining = match games {
        Some(stats) => {
            let done = stats.units_completed as f64;
            let target = stats.max_units as f64;
            if done >= target {
                return Ok(Some(0.0));
            }
            // Convert remaining units into remaining tasks using the observed
            // units-per-completed-task ratio.
            let completed_tasks = tasks_completed.max(1) as f64;
            let units_per_task = (done / completed_tasks).max(1.0);
            (target - done) / units_per_task
        }
        None => (tasks_total - tasks_completed).max(0) as f64,
    };

    Ok(Some(remaining / per_second))
}
