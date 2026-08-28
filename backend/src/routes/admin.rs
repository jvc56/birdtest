use crate::audit;
use crate::auth::{csrf, AdminUser};
use crate::error::{AppError, AppResult};
use crate::jobs::registry;
use crate::models::job::{Job, JobStatus, JobType, PlayerConfig};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use axum_extra::extract::CookieJar;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/player-configs", get(list_player_configs).post(create_player_config))
        .route("/player-configs/:id", get(get_player_config).delete(delete_player_config))
        .route("/jobs", post(create_job))
        .route("/jobs/:id/activate", post(activate_job))
        .route("/jobs/:id/deactivate", post(deactivate_job))
        .route("/jobs/:id/complete", post(complete_job))
        .route("/jobs/:id/purge", post(purge_job))
        .route("/jobs/:id", delete(delete_job))
        .route("/users/:id", delete(delete_user))
        .route("/workers/ban", post(ban_worker))
        .route("/workers/ban/:id", delete(unban_worker))
        .route("/audit-log", get(audit_log))
}

// ---------------------------------------------------------------------------
// Player configs
// ---------------------------------------------------------------------------

async fn list_player_configs(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> AppResult<Json<Vec<PlayerConfig>>> {
    Ok(Json(
        sqlx::query_as::<_, PlayerConfig>("SELECT * FROM player_configs ORDER BY created_at DESC")
            .fetch_all(&state.pool)
            .await?,
    ))
}

async fn get_player_config(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<PlayerConfig>> {
    Ok(Json(
        sqlx::query_as::<_, PlayerConfig>("SELECT * FROM player_configs WHERE id = $1")
            .bind(id)
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(|| AppError::not_found("no such player config"))?,
    ))
}

#[derive(Deserialize)]
struct CreatePlayerConfigBody {
    name: String,
    recorder_type: String,
    sort_strategy: Option<String>,
    leaves: Option<String>,
    max_iterations: Option<i32>,
    plies: Option<i32>,
    top_plays: Option<i32>,
    stopping_pct: Option<f64>,
    use_inference: Option<bool>,
    time_limit_secs: Option<f64>,
}

async fn create_player_config(
    State(state): State<AppState>,
    admin: AdminUser,
    method: Method,
    headers: HeaderMap,
    jar: CookieJar,
    Json(body): Json<CreatePlayerConfigBody>,
) -> AppResult<(StatusCode, Json<PlayerConfig>)> {
    csrf::verify(&method, &headers, &jar)?;

    if !matches!(body.recorder_type.as_str(), "best" | "equity" | "all") {
        return Err(AppError::bad_request("recorder_type must be 'best', 'equity' or 'all'"));
    }
    if let Some(sort) = &body.sort_strategy {
        if !matches!(sort.as_str(), "equity" | "score") {
            return Err(AppError::bad_request("sort_strategy must be 'equity', 'score' or null"));
        }
    }

    let config = sqlx::query_as::<_, PlayerConfig>(
        "INSERT INTO player_configs
             (name, recorder_type, sort_strategy, leaves, max_iterations, plies, top_plays,
              stopping_pct, use_inference, time_limit_secs, created_by)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
         RETURNING *",
    )
    .bind(body.name.trim())
    .bind(&body.recorder_type)
    .bind(&body.sort_strategy)
    .bind(&body.leaves)
    .bind(body.max_iterations)
    .bind(body.plies)
    .bind(body.top_plays)
    .bind(body.stopping_pct)
    .bind(body.use_inference)
    .bind(body.time_limit_secs)
    .bind(admin.0.id)
    .fetch_one(&state.pool)
    .await?;

    Ok((StatusCode::CREATED, Json(config)))
}

/// Player configs are immutable, so there is no update endpoint; deletion is
/// only allowed while nothing references the config.
async fn delete_player_config(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
    method: Method,
    headers: HeaderMap,
    jar: CookieJar,
) -> AppResult<StatusCode> {
    csrf::verify(&method, &headers, &jar)?;

    let referenced = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (
             SELECT 1 FROM job_opening_rack_config WHERE player_config_id = $1
             UNION ALL SELECT 1 FROM job_game_config
                 WHERE player1_config_id = $1 OR player2_config_id = $1
             UNION ALL SELECT 1 FROM job_game_pair_config
                 WHERE player1_config_id = $1 OR player2_config_id = $1
         )",
    )
    .bind(id)
    .fetch_one(&state.pool)
    .await?;

    if referenced {
        return Err(AppError::conflict("a job references this player config"));
    }

    let deleted = sqlx::query("DELETE FROM player_configs WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?;
    if deleted.rows_affected() == 0 {
        return Err(AppError::not_found("no such player config"));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Jobs
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct CreateJobBody {
    job_type: JobType,
    #[serde(default)]
    priority: i32,
    #[serde(default = "one")]
    redundancy: i32,
    min_magpie_version: Option<String>,
    #[serde(flatten)]
    config: JobTypeConfig,
}

fn one() -> i32 {
    1
}

/// Per-job-type configuration, expanded into typed columns rather than stored
/// as JSON.
#[derive(Deserialize)]
#[serde(untagged)]
enum JobTypeConfig {
    OpeningRack {
        lexicon: String,
        variant: String,
        player_config_id: Uuid,
        #[serde(default = "default_racks_per_batch")]
        racks_per_batch: i32,
        #[serde(default = "default_rack_size")]
        rack_size: i32,
        #[serde(default = "default_top_moves_stored")]
        top_moves_stored: i32,
    },
    Game {
        lexicon: String,
        variant: String,
        player1_config_id: Uuid,
        player2_config_id: Uuid,
        #[serde(default = "one")]
        games_per_batch: i32,
        min_games: i32,
        max_games: i32,
        #[serde(default = "default_alpha")]
        sprt_alpha: f64,
        #[serde(default = "default_alpha")]
        sprt_beta: f64,
        #[serde(default = "default_elo_low")]
        elo_low: f64,
        #[serde(default = "default_elo_high")]
        elo_high: f64,
        #[serde(default)]
        capture_positions: bool,
        #[serde(default = "default_capture_top_moves")]
        capture_top_moves: i32,
    },
    GamePair {
        lexicon: String,
        variant: String,
        player1_config_id: Uuid,
        player2_config_id: Uuid,
        #[serde(default = "one")]
        pairs_per_batch: i32,
        min_pairs: i32,
        max_pairs: i32,
        #[serde(default = "default_alpha")]
        sprt_alpha: f64,
        #[serde(default = "default_alpha")]
        sprt_beta: f64,
        #[serde(default = "default_elo_low")]
        elo_low: f64,
        #[serde(default = "default_elo_high")]
        elo_high: f64,
        #[serde(default)]
        capture_positions: bool,
        #[serde(default = "default_capture_top_moves")]
        capture_top_moves: i32,
    },
    Leave {
        lexicon: String,
        variant: String,
        num_iterations: i32,
        #[serde(default = "one")]
        generation_count: i32,
        target_rack_count: i32,
        racks_per_task: i32,
        #[serde(default = "default_max_leave_size")]
        max_leave_size: i32,
    },
}

fn default_alpha() -> f64 {
    0.05
}
fn default_elo_low() -> f64 {
    -10.0
}
fn default_elo_high() -> f64 {
    10.0
}
fn default_max_leave_size() -> i32 {
    6
}
/// Ranked moves kept per captured in-game position.
fn default_capture_top_moves() -> i32 {
    10
}
fn default_racks_per_batch() -> i32 {
    500
}
fn default_rack_size() -> i32 {
    7
}
/// Ranked moves kept per rack. Independent of how many the worker generates or
/// simulates, which is the player config's `top_plays`.
fn default_top_moves_stored() -> i32 {
    20
}

#[derive(Serialize)]
struct CreatedJob {
    job: Job,
    /// Rows written up front. Only leave generation has any: the rack universe
    /// that generation 1 is measured against. No job type pre-populates tasks.
    initialized: i64,
}

/// Jobs are always created inactive. Allocation is supplied later, at
/// activation, so the admin sets it while looking at the whole active set.
async fn create_job(
    State(state): State<AppState>,
    admin: AdminUser,
    method: Method,
    headers: HeaderMap,
    jar: CookieJar,
    Json(body): Json<CreateJobBody>,
) -> AppResult<(StatusCode, Json<CreatedJob>)> {
    csrf::verify(&method, &headers, &jar)?;

    let mut tx = state.pool.begin().await?;
    let job = sqlx::query_as::<_, Job>(
        "INSERT INTO jobs (job_type, priority, redundancy, min_magpie_version, created_by)
         VALUES ($1, $2, $3, $4, $5) RETURNING *",
    )
    .bind(body.job_type)
    .bind(body.priority)
    .bind(body.redundancy)
    .bind(&body.min_magpie_version)
    .bind(admin.0.id)
    .fetch_one(&mut *tx)
    .await?;

    insert_job_config(&mut tx, &job, &body.config, &state.cfg.data_path).await?;

    let initialized = registry::initialize_job_state(&mut tx, &job, &state.cfg.data_path).await?;

    audit::log(
        &mut tx,
        "job.created",
        Some(admin.0.id),
        None,
        Some("job"),
        Some(job.id.to_string()),
        Some(job.id),
    )
    .await?;
    tx.commit().await?;

    Ok((StatusCode::CREATED, Json(CreatedJob { job, initialized })))
}

async fn insert_job_config(
    conn: &mut sqlx::PgConnection,
    job: &Job,
    config: &JobTypeConfig,
    data_path: &std::path::Path,
) -> AppResult<()> {
    // The untagged config must actually match the declared job type, or the job
    // would exist with no config row and never dispatch anything.
    let mismatch = || AppError::bad_request("config fields do not match the requested job_type");

    match (job.job_type, config) {
        (
            JobType::OpeningRack,
            JobTypeConfig::OpeningRack {
                lexicon, variant, player_config_id, racks_per_batch, rack_size,
                top_moves_stored,
            },
        ) => {
            // Counting the space is cheap -- a small dynamic-programming table
            // over the letter distribution -- and recording it here means the
            // scheduler can tell when the job is exhausted without re-deriving
            // it on every claim.
            let total_racks =
                crate::jobs::opening_rack::total_racks(data_path, lexicon, *rack_size)?;
            sqlx::query(
                "INSERT INTO job_opening_rack_config
                     (job_id, lexicon, variant, player_config_id, racks_per_batch,
                      rack_size, top_moves_stored, total_racks)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            )
            .bind(job.id)
            .bind(lexicon)
            .bind(variant)
            .bind(player_config_id)
            .bind(racks_per_batch)
            .bind(rack_size)
            .bind(top_moves_stored)
            .bind(total_racks)
            .execute(conn)
            .await?;
        }
        (
            JobType::Games,
            JobTypeConfig::Game {
                lexicon, variant, player1_config_id, player2_config_id, games_per_batch,
                min_games, max_games, sprt_alpha, sprt_beta, elo_low, elo_high,
                capture_positions, capture_top_moves,
            },
        ) => {
            sqlx::query(
                "INSERT INTO job_game_config
                     (job_id, lexicon, variant, player1_config_id, player2_config_id,
                      games_per_batch, min_games, max_games, sprt_alpha, sprt_beta,
                      elo_low, elo_high, capture_positions, capture_top_moves)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)",
            )
            .bind(job.id).bind(lexicon).bind(variant)
            .bind(player1_config_id).bind(player2_config_id)
            .bind(games_per_batch).bind(min_games).bind(max_games)
            .bind(sprt_alpha).bind(sprt_beta).bind(elo_low).bind(elo_high)
            .bind(capture_positions).bind(capture_top_moves)
            .execute(conn)
            .await?;
        }
        (
            JobType::GamePairs,
            JobTypeConfig::GamePair {
                lexicon, variant, player1_config_id, player2_config_id, pairs_per_batch,
                min_pairs, max_pairs, sprt_alpha, sprt_beta, elo_low, elo_high,
                capture_positions, capture_top_moves,
            },
        ) => {
            sqlx::query(
                "INSERT INTO job_game_pair_config
                     (job_id, lexicon, variant, player1_config_id, player2_config_id,
                      pairs_per_batch, min_pairs, max_pairs, sprt_alpha, sprt_beta,
                      elo_low, elo_high, capture_positions, capture_top_moves)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)",
            )
            .bind(job.id).bind(lexicon).bind(variant)
            .bind(player1_config_id).bind(player2_config_id)
            .bind(pairs_per_batch).bind(min_pairs).bind(max_pairs)
            .bind(sprt_alpha).bind(sprt_beta).bind(elo_low).bind(elo_high)
            .bind(capture_positions).bind(capture_top_moves)
            .execute(conn)
            .await?;
        }
        (
            JobType::LeaveGeneration,
            JobTypeConfig::Leave {
                lexicon, variant, num_iterations, generation_count, target_rack_count,
                racks_per_task, max_leave_size,
            },
        ) => {
            sqlx::query(
                "INSERT INTO job_leave_config
                     (job_id, lexicon, variant, num_iterations, generation_count,
                      target_rack_count, racks_per_task, max_leave_size)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
            )
            .bind(job.id).bind(lexicon).bind(variant)
            .bind(num_iterations).bind(generation_count).bind(target_rack_count)
            .bind(racks_per_task).bind(max_leave_size)
            .execute(conn)
            .await?;
        }
        _ => return Err(mismatch()),
    }
    Ok(())
}

#[derive(Deserialize)]
struct ActivateBody {
    allocation: i32,
}

/// Activation sets the allocation. Active jobs in a priority tier must sum to
/// 100%, which is checked here rather than in the schema — the intermediate
/// states an admin passes through while rebalancing would violate a DB
/// constraint even when the end state is fine.
async fn activate_job(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    method: Method,
    headers: HeaderMap,
    jar: CookieJar,
    Json(body): Json<ActivateBody>,
) -> AppResult<Json<Job>> {
    csrf::verify(&method, &headers, &jar)?;

    if !(0..=100).contains(&body.allocation) {
        return Err(AppError::bad_request("allocation must be between 0 and 100"));
    }

    let mut tx = state.pool.begin().await?;
    let job = load_job_for_update(&mut tx, id).await?;
    if job.status == JobStatus::Completed {
        return Err(AppError::conflict("a completed job cannot be reactivated"));
    }

    let tier_total = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT SUM(allocation) FROM jobs
         WHERE status = 'active' AND priority = $1 AND id <> $2",
    )
    .bind(job.priority)
    .bind(id)
    .fetch_one(&mut *tx)
    .await?
    .unwrap_or(0);

    if tier_total + body.allocation as i64 > 100 {
        return Err(AppError::conflict(format!(
            "priority tier {} already allocates {tier_total}% — {}% is the most this job can take",
            job.priority,
            100 - tier_total
        )));
    }

    let updated = sqlx::query_as::<_, Job>(
        "UPDATE jobs SET status = 'active', allocation = $1, activated_at = now()
         WHERE id = $2 RETURNING *",
    )
    .bind(body.allocation)
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;

    audit::log_status_change(&mut tx, "job.activated", admin.0.id, id, "inactive", "active").await?;
    tx.commit().await?;
    Ok(Json(updated))
}

async fn deactivate_job(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    method: Method,
    headers: HeaderMap,
    jar: CookieJar,
) -> AppResult<Json<Job>> {
    csrf::verify(&method, &headers, &jar)?;

    let mut tx = state.pool.begin().await?;
    let job = sqlx::query_as::<_, Job>(
        "UPDATE jobs SET status = 'inactive', deactivated_at = now() WHERE id = $1 RETURNING *",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::not_found("no such job"))?;

    audit::log_status_change(&mut tx, "job.deactivated", admin.0.id, id, "active", "inactive")
        .await?;
    tx.commit().await?;
    Ok(Json(job))
}

async fn complete_job(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    method: Method,
    headers: HeaderMap,
    jar: CookieJar,
) -> AppResult<Json<Job>> {
    csrf::verify(&method, &headers, &jar)?;

    let mut tx = state.pool.begin().await?;
    let job =
        sqlx::query_as::<_, Job>("UPDATE jobs SET status = 'completed' WHERE id = $1 RETURNING *")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| AppError::not_found("no such job"))?;

    audit::log_status_change(&mut tx, "job.completed", admin.0.id, id, "active", "completed")
        .await?;
    tx.commit().await?;
    Ok(Json(job))
}

#[derive(Serialize)]
struct PurgeResult {
    tasks_reset: u64,
}

/// Clear every result and return the job's tasks to `available`. On-demand tasks
/// are deleted outright — they are regenerated at claim time, and keeping them
/// would leave the seed cursor advanced past work that was never done.
async fn purge_job(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    method: Method,
    headers: HeaderMap,
    jar: CookieJar,
) -> AppResult<Json<PurgeResult>> {
    csrf::verify(&method, &headers, &jar)?;

    let mut tx = state.pool.begin().await?;
    let job = load_job_for_update(&mut tx, id).await?;

    // Records and claims cascade from tasks; leave-gen progress and ratings are
    // keyed on the job directly.
    sqlx::query("DELETE FROM task_claims c USING tasks t WHERE c.task_id = t.id AND t.job_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM player_config_ratings WHERE job_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM leave_rack_progress WHERE job_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM leave_generation_artifacts WHERE job_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;

    // Every job type generates its tasks on demand, so purging deletes them
    // outright: they are regenerated from the start of the space at the next
    // claim. Leaving them would advance the seed cursor past work never done.
    let tasks_reset = sqlx::query("DELETE FROM tasks WHERE job_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?
        .rows_affected();

    // Leave-gen needs its generation-1 rack universe back to have anything to
    // measure progress against.
    registry::initialize_job_state(&mut tx, &job, &state.cfg.data_path).await?;

    audit::log(
        &mut tx,
        "job.purged",
        Some(admin.0.id),
        None,
        Some("job"),
        Some(id.to_string()),
        Some(id),
    )
    .await?;
    tx.commit().await?;

    Ok(Json(PurgeResult { tasks_reset }))
}

async fn delete_job(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    method: Method,
    headers: HeaderMap,
    jar: CookieJar,
) -> AppResult<StatusCode> {
    csrf::verify(&method, &headers, &jar)?;

    let mut tx = state.pool.begin().await?;
    // The audit row is written first: `audit_log.job_id` references `jobs`, so
    // it has to exist while the job still does.
    audit::log(
        &mut tx,
        "job.deleted",
        Some(admin.0.id),
        None,
        Some("job"),
        Some(id.to_string()),
        None,
    )
    .await?;

    let deleted = sqlx::query("DELETE FROM jobs WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    if deleted.rows_affected() == 0 {
        return Err(AppError::not_found("no such job"));
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Users and bans
// ---------------------------------------------------------------------------

/// Account deletion is done at the application layer, not by `ON DELETE
/// CASCADE`: the denormalized task counters have to be decremented and tasks may
/// revert from completed to available, which a DB-level cascade cannot do.
async fn delete_user(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    method: Method,
    headers: HeaderMap,
    jar: CookieJar,
) -> AppResult<StatusCode> {
    csrf::verify(&method, &headers, &jar)?;

    if id == admin.0.id {
        return Err(AppError::bad_request("you cannot delete your own account"));
    }

    let mut tx = state.pool.begin().await?;

    // 1. Roll back the counters every one of this user's claims contributed.
    sqlx::query(
        "WITH mine AS (
             SELECT task_id,
                    COUNT(*) FILTER (WHERE state = 'completed')::int AS accepted,
                    COUNT(*) FILTER (WHERE state = 'claimed')::int   AS active
             FROM task_claims WHERE claimed_by_user_id = $1
             GROUP BY task_id
         )
         UPDATE tasks t
         SET accepted_count = GREATEST(t.accepted_count - mine.accepted, 0),
             active_claim_count = GREATEST(t.active_claim_count - mine.active, 0),
             state = CASE
                 WHEN GREATEST(t.accepted_count - mine.accepted, 0) >= j.redundancy
                     THEN 'completed'::task_state
                 WHEN GREATEST(t.accepted_count - mine.accepted, 0)
                      + GREATEST(t.active_claim_count - mine.active, 0) >= j.redundancy
                     THEN 'claimed'::task_state
                 ELSE 'available'::task_state
             END,
             completed_at = CASE
                 WHEN GREATEST(t.accepted_count - mine.accepted, 0) >= j.redundancy
                     THEN t.completed_at ELSE NULL
             END
         FROM mine, jobs j
         WHERE t.id = mine.task_id AND j.id = t.job_id",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;

    // 2-3. Task records cascade from the claim rows.
    sqlx::query("DELETE FROM task_claims WHERE claimed_by_user_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;

    // 4. The user row, cascading to api_keys, confirmations and reset tokens.
    let deleted = sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    if deleted.rows_affected() == 0 {
        return Err(AppError::not_found("no such user"));
    }

    audit::log(
        &mut tx,
        "user.deleted",
        Some(admin.0.id),
        None,
        Some("user"),
        Some(id.to_string()),
        None,
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct BanBody {
    user_id: Option<Uuid>,
    anon_uuid: Option<Uuid>,
    reason: Option<String>,
}

async fn ban_worker(
    State(state): State<AppState>,
    admin: AdminUser,
    method: Method,
    headers: HeaderMap,
    jar: CookieJar,
    Json(body): Json<BanBody>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    csrf::verify(&method, &headers, &jar)?;

    if body.user_id.is_some() == body.anon_uuid.is_some() {
        return Err(AppError::bad_request("supply exactly one of user_id or anon_uuid"));
    }

    let mut tx = state.pool.begin().await?;
    let ban_id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO worker_bans (user_id, anon_uuid, reason, banned_by)
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(body.user_id)
    .bind(body.anon_uuid)
    .bind(&body.reason)
    .bind(admin.0.id)
    .fetch_one(&mut *tx)
    .await?;

    let target = body
        .user_id
        .map(|id| id.to_string())
        .or_else(|| body.anon_uuid.map(|id| id.to_string()))
        .unwrap_or_default();
    audit::log_ban(&mut tx, admin.0.id, target, body.reason).await?;
    tx.commit().await?;

    Ok((StatusCode::CREATED, Json(serde_json::json!({ "id": ban_id }))))
}

async fn unban_worker(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    method: Method,
    headers: HeaderMap,
    jar: CookieJar,
) -> AppResult<StatusCode> {
    csrf::verify(&method, &headers, &jar)?;

    let deleted = sqlx::query("DELETE FROM worker_bans WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?;
    if deleted.rows_affected() == 0 {
        return Err(AppError::not_found("no such ban"));
    }
    let _ = admin;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Audit log
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct AuditQuery {
    action: Option<String>,
    actor_user_id: Option<Uuid>,
    target_type: Option<String>,
    job_id: Option<Uuid>,
    #[serde(default)]
    page: i64,
    per_page: Option<i64>,
}

#[derive(Serialize, sqlx::FromRow)]
struct AuditRow {
    id: i64,
    action: String,
    actor_user_id: Option<Uuid>,
    actor_anon_uuid: Option<Uuid>,
    target_type: Option<String>,
    target_id: Option<String>,
    job_id: Option<Uuid>,
    reason: Option<String>,
    old_status: Option<String>,
    new_status: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
}

async fn audit_log(
    State(state): State<AppState>,
    _admin: AdminUser,
    Query(query): Query<AuditQuery>,
) -> AppResult<Json<super::Page<AuditRow>>> {
    let (limit, offset) = super::paginate(query.page, query.per_page);

    let rows = sqlx::query_as::<_, AuditRow>(
        "SELECT * FROM audit_log
         WHERE ($1::text IS NULL OR action = $1)
           AND ($2::uuid IS NULL OR actor_user_id = $2)
           AND ($3::text IS NULL OR target_type = $3)
           AND ($4::uuid IS NULL OR job_id = $4)
         ORDER BY created_at DESC, id DESC
         LIMIT $5 OFFSET $6",
    )
    .bind(&query.action)
    .bind(query.actor_user_id)
    .bind(&query.target_type)
    .bind(query.job_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.pool)
    .await?;

    let total = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM audit_log
         WHERE ($1::text IS NULL OR action = $1)
           AND ($2::uuid IS NULL OR actor_user_id = $2)
           AND ($3::text IS NULL OR target_type = $3)
           AND ($4::uuid IS NULL OR job_id = $4)",
    )
    .bind(&query.action)
    .bind(query.actor_user_id)
    .bind(&query.target_type)
    .bind(query.job_id)
    .fetch_one(&state.pool)
    .await?;

    Ok(Json(super::Page { items: rows, total, page: query.page.max(0), per_page: limit }))
}

async fn load_job_for_update(conn: &mut sqlx::PgConnection, id: Uuid) -> AppResult<Job> {
    sqlx::query_as::<_, Job>("SELECT * FROM jobs WHERE id = $1 FOR UPDATE")
        .bind(id)
        .fetch_optional(conn)
        .await?
        .ok_or_else(|| AppError::not_found("no such job"))
}
