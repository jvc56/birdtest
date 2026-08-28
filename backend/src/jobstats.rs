//! Aggregate statistics for a job. One place computes them, and both the public
//! REST endpoint and the SSE push use it, so a dashboard update over the live
//! stream is byte-for-byte what a page reload would produce.

use crate::error::AppResult;
use crate::models::job::{GameConfig, GamePairConfig, Job, JobType, LeaveConfig, SprtParams};
use crate::stats::sprt::{self, SprtResult, Tally};
use serde::Serialize;
use sqlx::{PgPool, Row};
use uuid::Uuid;

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
    /// Empty for every job type but game pairs. Always serialized, so the
    /// client can read `.length` without a presence check.
    pub ratings: Vec<RatingSnapshot>,
    pub workers: Vec<WorkerContribution>,
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
    pub min_magpie_version: Option<String>,
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
    /// Game pairs only: how many of those pairs diverged and so contributed to
    /// the tally above.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub divergent_pairs: Option<u64>,
    pub min_units: i32,
    pub max_units: i32,
    pub win_pct: f64,
    pub loss_pct: f64,
    pub draw_pct: f64,
    pub sprt: SprtResult,
}

#[derive(Debug, Serialize)]
pub struct OpeningRackStats {
    /// Distinct racks with at least one accepted analysis.
    pub racks_analyzed: i64,
    /// Size of the rack space; the denominator for progress.
    pub racks_total: i64,
    pub average_best_equity: Option<f64>,
    pub best_move_types: Vec<MoveTypeCount>,
}

#[derive(Debug, Serialize)]
pub struct MoveTypeCount {
    pub move_type: String,
    pub count: i64,
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
pub struct RatingSnapshot {
    pub player_config_id: Uuid,
    pub name: String,
    pub rating: f64,
    pub rating_deviation: f64,
    pub games_played: i32,
}

#[derive(Debug, Serialize)]
pub struct WorkerContribution {
    pub user_id: Option<Uuid>,
    pub anon_uuid: Option<Uuid>,
    pub username: Option<String>,
    pub tasks_completed: i64,
}

pub async fn load_job(pool: &PgPool, job_id: Uuid) -> AppResult<Job> {
    Ok(sqlx::query_as::<_, Job>("SELECT * FROM jobs WHERE id = $1")
        .bind(job_id)
        .fetch_one(pool)
        .await?)
}

pub async fn compute(pool: &PgPool, job: &Job) -> AppResult<JobStats> {
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

    let games = match job.job_type {
        JobType::Games => Some(game_stats(pool, job).await?),
        JobType::GamePairs => Some(game_pair_stats(pool, job).await?),
        _ => None,
    };

    let opening_racks = match job.job_type {
        JobType::OpeningRackAnalysis => Some(opening_rack_stats(pool, job.id).await?),
        _ => None,
    };

    let leave_generation = match job.job_type {
        JobType::LeaveGeneration => Some(leave_gen_stats(pool, job.id).await?),
        _ => None,
    };

    let ratings = match job.job_type {
        JobType::GamePairs => rating_snapshots(pool, job.id).await?,
        _ => Vec::new(),
    };

    let tasks_total: i64 = counts.get("total");
    let tasks_completed: i64 = counts.get("completed");
    let results_accepted: i64 = counts.get("accepted");

    let eta_seconds = estimate_eta(pool, job, &games, tasks_total, tasks_completed).await?;

    Ok(JobStats {
        job: JobSummary {
            id: job.id,
            job_type: job.job_type,
            status: status_label(job).to_string(),
            priority: job.priority,
            allocation: job.allocation,
            redundancy: job.redundancy,
            min_magpie_version: job.min_magpie_version.clone(),
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
        ratings,
        workers: worker_contributions(pool, job.id).await?,
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

async fn lexicon_and_variant(
    pool: &PgPool,
    job: &Job,
) -> AppResult<(Option<String>, Option<String>)> {
    let table = match job.job_type {
        JobType::OpeningRackAnalysis => "job_opening_rack_config",
        JobType::Games => "job_game_config",
        JobType::GamePairs => "job_game_pair_config",
        JobType::LeaveGeneration => "job_leave_config",
    };
    let row = sqlx::query(&format!(
        "SELECT lexicon, variant FROM {table} WHERE job_id = $1"
    ))
    .bind(job.id)
    .fetch_optional(pool)
    .await?;
    Ok(match row {
        Some(row) => (Some(row.get("lexicon")), Some(row.get("variant"))),
        None => (None, None),
    })
}

/// Sum the per-task aggregates for a plain `games` job. The SPRT unit is a
/// game, so the tally and the unit count are the same number.
async fn game_stats(pool: &PgPool, job: &Job) -> AppResult<GameStats> {
    let config = sqlx::query_as::<_, GameConfig>("SELECT * FROM job_game_config WHERE job_id = $1")
        .bind(job.id)
        .fetch_one(pool)
        .await?;

    let row = sqlx::query(
        "SELECT COALESCE(SUM(r.games), 0)::bigint  AS games,
                COALESCE(SUM(r.wins), 0)::bigint   AS wins,
                COALESCE(SUM(r.losses), 0)::bigint AS losses,
                COALESCE(SUM(r.ties), 0)::bigint   AS ties
         FROM game_results r JOIN tasks t ON t.id = r.task_id
         WHERE t.job_id = $1",
    )
    .bind(job.id)
    .fetch_one(pool)
    .await?;

    let tally = Tally {
        wins: row.get::<i64, _>("wins") as u64,
        losses: row.get::<i64, _>("losses") as u64,
        draws: row.get::<i64, _>("ties") as u64,
    };
    let games = row.get::<i64, _>("games") as u64;
    Ok(build_game_stats("game", tally, games, &SprtParams::from(&config)))
}

/// A game-pairs job reports two numbers that mean different things.
///
/// Progress is measured in **pairs played** — two games each — because that is
/// what `min_pairs` and `max_pairs` bound. The LLR, though, is computed over the
/// **divergent** games only: a pair whose two games played identically is a
/// guaranteed tie carrying no information, and excluding those is the variance
/// reduction that pairing exists to provide.
///
/// This does treat the two games of a divergent pair as independent
/// observations. They are not quite — they share a seed — so the LLR is
/// slightly optimistic. Correcting it would need per-pair outcomes, which
/// MAGPIE does not report.
async fn game_pair_stats(pool: &PgPool, job: &Job) -> AppResult<GameStats> {
    let config =
        sqlx::query_as::<_, GamePairConfig>("SELECT * FROM job_game_pair_config WHERE job_id = $1")
            .bind(job.id)
            .fetch_one(pool)
            .await?;

    let row = sqlx::query(
        "SELECT COALESCE(SUM(r.games), 0)::bigint            AS games,
                COALESCE(SUM(r.divergent_games), 0)::bigint  AS divergent_games,
                COALESCE(SUM(r.divergent_wins), 0)::bigint   AS wins,
                COALESCE(SUM(r.divergent_losses), 0)::bigint AS losses,
                COALESCE(SUM(r.divergent_ties), 0)::bigint   AS ties
         FROM game_results r JOIN tasks t ON t.id = r.task_id
         WHERE t.job_id = $1",
    )
    .bind(job.id)
    .fetch_one(pool)
    .await?;

    let tally = Tally {
        wins: row.get::<i64, _>("wins") as u64,
        losses: row.get::<i64, _>("losses") as u64,
        draws: row.get::<i64, _>("ties") as u64,
    };
    let pairs_played = row.get::<i64, _>("games") as u64 / 2;
    let mut stats = build_game_stats("pair", tally, pairs_played, &SprtParams::from(&config));
    stats.divergent_pairs = Some(row.get::<i64, _>("divergent_games") as u64 / 2);
    Ok(stats)
}

fn build_game_stats(
    unit: &'static str,
    tally: Tally,
    units_completed: u64,
    params: &SprtParams,
) -> GameStats {
    // Percentages describe the tally the LLR is computed from, which for pairs
    // is the divergent subset rather than every game played.
    let total = tally.total();
    let pct = |n: u64| if total == 0 { 0.0 } else { 100.0 * n as f64 / total as f64 };
    let sprt = sprt::evaluate(
        &tally,
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
        divergent_pairs: None,
        min_units: params.min_units,
        max_units: params.max_units,
        win_pct: pct(tally.wins),
        loss_pct: pct(tally.losses),
        draw_pct: pct(tally.draws),
        sprt,
    }
}

async fn opening_rack_stats(pool: &PgPool, job_id: Uuid) -> AppResult<OpeningRackStats> {
    let row = sqlx::query(
        "SELECT COUNT(DISTINCT r.rack)::bigint AS analyzed, AVG(r.best_equity) AS avg_equity
         FROM position_analysis_records r JOIN tasks t ON t.id = r.task_id
         WHERE t.job_id = $1",
    )
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

    // A play containing a '.' is a placement; anything else is an exchange or a
    // pass. That is the only distinction the dashboard draws.
    let types = sqlx::query(
        "SELECT CASE
                    WHEN r.best_move ILIKE 'ex%' THEN 'exchange'
                    WHEN r.best_move ILIKE 'pass%' THEN 'pass'
                    ELSE 'placement'
                END AS move_type,
                COUNT(*)::bigint AS count
         FROM position_analysis_records r JOIN tasks t ON t.id = r.task_id
         WHERE t.job_id = $1
         GROUP BY 1 ORDER BY 2 DESC",
    )
    .bind(job_id)
    .fetch_all(pool)
    .await?;

    Ok(OpeningRackStats {
        racks_analyzed: row.get("analyzed"),
        racks_total,
        average_best_equity: row.get("avg_equity"),
        best_move_types: types
            .into_iter()
            .map(|r| MoveTypeCount { move_type: r.get("move_type"), count: r.get("count") })
            .collect(),
    })
}

async fn leave_gen_stats(pool: &PgPool, job_id: Uuid) -> AppResult<LeaveGenStats> {
    let config = sqlx::query_as::<_, LeaveConfig>("SELECT * FROM job_leave_config WHERE job_id = $1")
        .bind(job_id)
        .fetch_one(pool)
        .await?;

    let completed = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM leave_generation_artifacts WHERE job_id = $1",
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

async fn rating_snapshots(pool: &PgPool, job_id: Uuid) -> AppResult<Vec<RatingSnapshot>> {
    let rows = sqlx::query(
        "SELECT r.player_config_id, p.name, r.rating, r.rating_deviation, r.games_played
         FROM player_config_ratings r
         JOIN player_configs p ON p.id = r.player_config_id
         WHERE r.job_id = $1
         ORDER BY r.rating DESC",
    )
    .bind(job_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| RatingSnapshot {
            player_config_id: r.get("player_config_id"),
            name: r.get("name"),
            rating: r.get("rating"),
            rating_deviation: r.get("rating_deviation"),
            games_played: r.get("games_played"),
        })
        .collect())
}

pub async fn worker_contributions(
    pool: &PgPool,
    job_id: Uuid,
) -> AppResult<Vec<WorkerContribution>> {
    let rows = sqlx::query(
        "SELECT c.claimed_by_user_id AS user_id,
                c.claimed_by_anon_uuid AS anon_uuid,
                u.username,
                COUNT(*)::bigint AS tasks_completed
         FROM task_claims c
         JOIN tasks t ON t.id = c.task_id
         LEFT JOIN users u ON u.id = c.claimed_by_user_id
         WHERE t.job_id = $1 AND c.state = 'completed'
         GROUP BY 1, 2, 3
         ORDER BY 4 DESC",
    )
    .bind(job_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| WorkerContribution {
            user_id: r.get("user_id"),
            anon_uuid: r.get("anon_uuid"),
            username: r.get("username"),
            tasks_completed: r.get("tasks_completed"),
        })
        .collect())
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
