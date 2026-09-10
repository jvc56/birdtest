use crate::audit;
use crate::auth::{csrf, AdminUser};
use crate::backups::{self, BackupStatus};
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
        .route("/input-data", get(list_input_data))
        .route("/input-data/:id", delete(delete_input_data))
        .route("/input-data/imports", post(start_import))
        .route("/input-data/imports/:id", get(get_import))
        .route("/input-data/imports/:id/confirm", post(confirm_import))
        .route("/jobs/:id/data-gaps", get(job_data_gaps))
        .route("/jobs/:id/rebuild-artifacts", post(rebuild_artifacts))
        .route("/backups", get(backups))
        .route("/fleet", get(fleet))
}

// ---------------------------------------------------------------------------
// Input data
// ---------------------------------------------------------------------------

#[derive(Serialize, sqlx::FromRow)]
struct InputDataRow {
    id: Uuid,
    path: String,
    role: String,
    name: String,
    sha256: String,
    bytes: i64,
    tarball_date: String,
    imported_at: chrono::DateTime<chrono::Utc>,
    /// How many jobs and player configs would block a delete.
    references: i64,
}

/// The vocabulary the job and player forms pick from, newest first.
async fn list_input_data(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> AppResult<Json<Vec<InputDataRow>>> {
    Ok(Json(
        sqlx::query_as::<_, InputDataRow>(
            "SELECT d.id, d.path, d.role, d.name, d.sha256, d.bytes, d.tarball_date,
                    d.imported_at,
                    (SELECT COUNT(*) FROM jobs j
                      WHERE j.letterdist_id = d.id OR j.layout_id = d.id)
                  + (SELECT COUNT(*) FROM player_configs pc
                      WHERE pc.kwg_id = d.id OR pc.klv_id = d.id OR pc.winpct_id = d.id)
                  + (SELECT COUNT(*) FROM job_leave_config lc WHERE lc.kwg_id = d.id)
                    AS references
             FROM input_data d
             ORDER BY d.tarball_date DESC, d.role, d.name",
        )
        .fetch_all(&state.pool)
        .await?,
    ))
}

/// Deleting relies on the foreign keys to refuse a referenced row: they carry
/// no `ON DELETE` clause, so Postgres defaults to `NO ACTION` and the
/// constraint *is* the safety mechanism. The raw violation is unreadable
/// though, so it is translated into what an admin needs to know.
async fn delete_input_data(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
    method: Method,
    headers: HeaderMap,
    jar: CookieJar,
) -> AppResult<StatusCode> {
    csrf::verify(&method, &headers, &jar)?;

    let uses: i64 = sqlx::query_scalar(
        "SELECT (SELECT COUNT(*) FROM jobs j
                  WHERE j.letterdist_id = $1 OR j.layout_id = $1)
              + (SELECT COUNT(*) FROM player_configs pc
                  WHERE pc.kwg_id = $1 OR pc.klv_id = $1 OR pc.winpct_id = $1)
              + (SELECT COUNT(*) FROM job_leave_config lc WHERE lc.kwg_id = $1)",
    )
    .bind(id)
    .fetch_one(&state.pool)
    .await?;
    if uses > 0 {
        return Err(AppError::conflict(format!(
            "this file is pinned by {uses} job{} or player config{}",
            if uses == 1 { "" } else { "s" },
            if uses == 1 { "" } else { "s" }
        )));
    }

    let deleted = sqlx::query("DELETE FROM input_data WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?;
    if deleted.rows_affected() == 0 {
        return Err(AppError::not_found("no such input data row"));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct StartImportBody {
    tarball_date: String,
    /// A tag, branch or commit sha, resolved to a commit at import time so the
    /// record names a commit and never a moving branch.
    #[serde(default = "default_ref")]
    git_ref: String,
}

fn default_ref() -> String {
    "main".to_string()
}

#[derive(Serialize)]
struct StartedImport {
    id: Uuid,
    state: &'static str,
}

/// Phase 1, in the background. The archive is ~94 MB, so the request returns an
/// id immediately and the admin UI polls `GET .../imports/<id>`; nothing waits
/// on the download and no transaction is held open across it.
async fn start_import(
    State(state): State<AppState>,
    admin: AdminUser,
    method: Method,
    headers: HeaderMap,
    jar: CookieJar,
    Json(body): Json<StartImportBody>,
) -> AppResult<(StatusCode, Json<StartedImport>)> {
    csrf::verify(&method, &headers, &jar)?;

    let tarball_date = body.tarball_date.trim().to_string();
    if tarball_date.len() != 8 || !tarball_date.chars().all(|c| c.is_ascii_digit()) {
        return Err(AppError::bad_request("tarball_date must be YYYYMMDD"));
    }

    // Resolving the ref is a single GitHub API call and its failure modes are
    // worth reporting synchronously -- a typo'd ref should not become a failed
    // background task.
    let commit_sha = crate::inputdata::resolve_ref(&state, &body.git_ref).await?;

    let mut tx = state.pool.begin().await?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO input_data_imports (tarball_date, commit_sha, requested_by)
         VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(&tarball_date)
    .bind(&commit_sha)
    .bind(admin.0.id)
    .fetch_one(&mut *tx)
    .await?;

    audit::log(
        &mut tx,
        "input_data.import_staged",
        Some(admin.0.id),
        None,
        Some("input_data_import"),
        Some(id.to_string()),
        None,
    )
    .await?;
    tx.commit().await?;

    tokio::spawn(crate::inputdata::run_import(
        state.clone(),
        id,
        tarball_date,
        commit_sha,
    ));

    Ok((StatusCode::ACCEPTED, Json(StartedImport { id, state: "running" })))
}

#[derive(Serialize, sqlx::FromRow)]
struct ImportRow {
    id: Uuid,
    tarball_date: String,
    commit_sha: String,
    tarball_sha256: Option<String>,
    state: String,
    progress_bytes: i64,
    progress_entries: i32,
    error: Option<String>,
    requested_at: chrono::DateTime<chrono::Utc>,
    confirmed_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Serialize, sqlx::FromRow)]
struct ImportFileRow {
    path: String,
    role: String,
    name: String,
    sha256: String,
    bytes: i64,
    /// `new`, `known`, or `collision` -- a path already known under a different
    /// digest, which is either a legitimate data update or a tarball re-cut
    /// under a name that was already used.
    disposition: String,
}

#[derive(Serialize)]
struct ImportDetail {
    #[serde(flatten)]
    import: ImportRow,
    files: Vec<ImportFileRow>,
}

/// Polled while the import runs, then read for the staged diff.
async fn get_import(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<ImportDetail>> {
    let import = sqlx::query_as::<_, ImportRow>(
        "SELECT id, tarball_date, commit_sha, tarball_sha256, state, progress_bytes,
                progress_entries, error, requested_at, confirmed_at
         FROM input_data_imports WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::not_found("no such import"))?;

    let files = sqlx::query_as::<_, ImportFileRow>(
        "SELECT path, role, name, sha256, bytes, disposition
         FROM input_data_import_rows WHERE import_id = $1
         ORDER BY disposition, path",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(ImportDetail { import, files }))
}

#[derive(Serialize)]
struct ConfirmedImport {
    inserted: i64,
}

/// Phase 2: insert the new rows, in one transaction, exactly as shown.
///
/// Only `new` rows are inserted -- `known` is already there, and a `collision`
/// is a different file at a known path, which is a new row by content anyway.
/// The bytes of the roles the server reads were kept at staging, so nothing is
/// downloaded again.
async fn confirm_import(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    method: Method,
    headers: HeaderMap,
    jar: CookieJar,
) -> AppResult<Json<ConfirmedImport>> {
    csrf::verify(&method, &headers, &jar)?;

    let mut tx = state.pool.begin().await?;
    let import = sqlx::query_as::<_, ImportRow>(
        "SELECT id, tarball_date, commit_sha, tarball_sha256, state, progress_bytes,
                progress_entries, error, requested_at, confirmed_at
         FROM input_data_imports WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::not_found("no such import"))?;

    if import.state != "staged" {
        return Err(AppError::conflict(format!(
            "this import is {}, not staged",
            import.state
        )));
    }

    let inserted = sqlx::query(
        "INSERT INTO input_data (path, role, name, sha256, bytes, tarball_date,
                                 content, imported_by)
         SELECT r.path, r.role, r.name, r.sha256, r.bytes, $2, r.content, $3
         FROM input_data_import_rows r
         WHERE r.import_id = $1 AND r.disposition <> 'known'
         ON CONFLICT (path, sha256) DO NOTHING",
    )
    .bind(id)
    .bind(&import.tarball_date)
    .bind(admin.0.id)
    .execute(&mut *tx)
    .await?
    .rows_affected() as i64;

    sqlx::query(
        "UPDATE input_data_imports SET state = 'confirmed', confirmed_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;

    audit::log(
        &mut tx,
        "input_data.import_confirmed",
        Some(admin.0.id),
        None,
        Some("input_data_import"),
        Some(id.to_string()),
        None,
    )
    .await?;
    tx.commit().await?;

    Ok(Json(ConfirmedImport { inserted }))
}

// ---------------------------------------------------------------------------
// What the fleet is missing, and what it is running
// ---------------------------------------------------------------------------

#[derive(Serialize, sqlx::FromRow)]
struct DataGap {
    role: String,
    name: String,
    expected: String,
    /// Distinct workers that reported this gap, which is what turns "this job
    /// is quiet" into "14 workers are all missing one file".
    workers: i64,
    declines: i64,
    last_reported_at: chrono::DateTime<chrono::Utc>,
}

async fn job_data_gaps(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Vec<DataGap>>> {
    Ok(Json(
        sqlx::query_as::<_, DataGap>(
            "SELECT g.role, g.name, g.expected,
                    COUNT(DISTINCT COALESCE(c.claimed_by_user_id, c.claimed_by_anon_uuid))
                        AS workers,
                    COUNT(*) AS declines,
                    MAX(g.reported_at) AS last_reported_at
             FROM worker_data_gaps g
             JOIN task_claims c ON c.id = g.claim_id
             WHERE g.job_id = $1
             GROUP BY g.role, g.name, g.expected
             ORDER BY workers DESC, g.role, g.name",
        )
        .bind(id)
        .fetch_all(&state.pool)
        .await?,
    ))
}

#[derive(Serialize, sqlx::FromRow)]
struct FleetVersion {
    magpie_version: Option<String>,
    workers: i64,
    claims: i64,
}

/// What the field is running, over the last week. This is the evidence for
/// raising a job's floor: the difference between doing it on evidence and doing
/// it on hope.
async fn fleet(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> AppResult<Json<Vec<FleetVersion>>> {
    Ok(Json(
        sqlx::query_as::<_, FleetVersion>(
            "SELECT magpie_version,
                    COUNT(DISTINCT COALESCE(claimed_by_user_id, claimed_by_anon_uuid))
                        AS workers,
                    COUNT(*) AS claims
             FROM task_claims
             WHERE claimed_at > now() - interval '7 days'
             GROUP BY magpie_version
             ORDER BY workers DESC",
        )
        .fetch_all(&state.pool)
        .await?,
    ))
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
    /// The files this player loads, as `input_data` rows rather than names.
    /// `winpct_id` is null for a static player, which never loads one.
    kwg_id: Uuid,
    klv_id: Uuid,
    #[serde(default)]
    winpct_id: Option<Uuid>,
    /// Set when this config is a clone onto newer data.
    #[serde(default)]
    cloned_from_id: Option<Uuid>,
    max_iterations: Option<i32>,
    num_plies: Option<i32>,
    num_plies_recorded: Option<i32>,
    num_plays: Option<i32>,
    num_plays_recorded: Option<i32>,
    stopping_pct: Option<f64>,
    use_inference: Option<bool>,
    time_limit_secs: Option<i32>,
    #[serde(default)]
    use_wordmap: Option<bool>,
    #[serde(default)]
    use_rit: Option<bool>,
    #[serde(default)]
    min_play_iterations: Option<i32>,
    #[serde(default)]
    threshold: Option<String>,
    #[serde(default)]
    sampling_rule: Option<String>,
    #[serde(default)]
    inference_margin: Option<f64>,
    #[serde(default)]
    utility_w_winpct: Option<f64>,
    #[serde(default)]
    utility_w_spread: Option<f64>,
    #[serde(default)]
    utility_spread_scale: Option<f64>,
    #[serde(default)]
    movegen_margin: Option<f64>,
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
    if let Some(threshold) = &body.threshold {
        if !matches!(threshold.as_str(), "none" | "gk16") {
            return Err(AppError::bad_request("threshold must be 'none', 'gk16' or null"));
        }
    }
    if let Some(rule) = &body.sampling_rule {
        if !matches!(rule.as_str(), "round_robin" | "top_two_ids") {
            return Err(AppError::bad_request(
                "sampling_rule must be 'round_robin', 'top_two_ids' or null",
            ));
        }
    }

    // Postgres cannot express this: the foreign keys point at `input_data`
    // without constraining which role each one lands on, so `kwg_id` could name
    // a letter distribution as far as the database is concerned.
    let kwg = require_role(&state.pool, body.kwg_id, "kwg").await?;
    let klv = require_role(&state.pool, body.klv_id, "klv").await?;
    let winpct = match body.winpct_id {
        Some(id) => Some(require_role(&state.pool, id, "winpct").await?),
        None => None,
    };

    // A player with simulation parameters loads a win% model; a static player
    // never opens one. Getting this wrong would either fail at the worker or
    // lock a contributor out of jobs that would never have read the file.
    let simming = body.max_iterations.is_some()
        || body.num_plies.is_some()
        || body.num_plays.is_some()
        || body.stopping_pct.is_some();
    match (simming, &winpct) {
        (true, None) => {
            return Err(AppError::bad_request(
                "a simming player config must name a win% model (winpct_id)",
            ))
        }
        (false, Some(_)) => {
            return Err(AppError::bad_request(
                "a static player config must not name a win% model: MAGPIE never loads one for it",
            ))
        }
        _ => {}
    }

    // Leaves are named after the lexicon they were built for, and MAGPIE
    // refuses a pairing from two different letter distributions.
    crate::compat::validate_lexicon_and_leaves(&kwg, &klv)?;

    let config = sqlx::query_as::<_, PlayerConfig>(
        "INSERT INTO player_configs
             (name, recorder_type, sort_strategy, kwg_id, klv_id, winpct_id,
              cloned_from_id, max_iterations, num_plies,
              num_plies_recorded, num_plays, num_plays_recorded,
              stopping_pct, use_inference, time_limit_secs,
              use_wordmap, use_rit, min_play_iterations, threshold,
              sampling_rule, inference_margin, utility_w_winpct, utility_w_spread,
              utility_spread_scale, movegen_margin, created_by)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,
                 $18,$19,$20,$21,$22,$23,$24,$25,$26)
         RETURNING *",
    )
    .bind(body.name.trim())
    .bind(&body.recorder_type)
    .bind(&body.sort_strategy)
    .bind(body.kwg_id)
    .bind(body.klv_id)
    .bind(body.winpct_id)
    .bind(body.cloned_from_id)
    .bind(body.max_iterations)
    .bind(body.num_plies)
    .bind(body.num_plies_recorded)
    .bind(body.num_plays)
    .bind(body.num_plays_recorded)
    .bind(body.stopping_pct)
    .bind(body.use_inference)
    .bind(body.time_limit_secs)
    .bind(body.use_wordmap)
    .bind(body.use_rit)
    .bind(body.min_play_iterations)
    .bind(&body.threshold)
    .bind(&body.sampling_rule)
    .bind(body.inference_margin)
    .bind(body.utility_w_winpct)
    .bind(body.utility_w_spread)
    .bind(body.utility_spread_scale)
    .bind(body.movegen_margin)
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

/// Reads an `input_data` row's name, insisting it is the role the caller
/// expects. The foreign keys cannot express this -- every one of them points at
/// the same table -- so it is validated wherever a role column is written.
async fn require_role(
    pool: &sqlx::PgPool,
    id: Uuid,
    role: &str,
) -> AppResult<String> {
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT role, name FROM input_data WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await?;
    let Some((actual, name)) = row else {
        return Err(AppError::bad_request(format!("no input data row {id}")));
    };
    if actual != role {
        return Err(AppError::bad_request(format!(
            "expected a {role} row, but {name} is a {actual} row"
        )));
    }
    Ok(name)
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
    /// Rules setting shared by every job type.
    variant: String,
    /// One letter distribution and one board per job: MAGPIE takes a single
    /// `-ld` for the whole game, and two players cannot draw from different
    /// bags.
    letterdist_id: Uuid,
    layout_id: Uuid,
    /// Defaults to the server-wide floor when the form leaves it out. The
    /// effective value is shown on the creation form so the default is visible
    /// rather than hidden.
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
        player_config_id: Uuid,
        #[serde(default = "default_racks_per_batch")]
        racks_per_batch: i32,
        #[serde(default = "default_rack_size")]
        rack_size: i32,
    },
    Game {
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
    },
    GamePair {
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
    },
    Leave {
        /// The one place a lexicon still sits on a job: leave generation has a
        /// single bot and no player config to hold it.
        kwg_id: Uuid,
        num_iterations: i32,
        #[serde(default = "one")]
        generation_count: i32,
        target_rack_count: i32,
        racks_per_task: i32,
        #[serde(default = "default_max_leave_size")]
        max_leave_size: i32,
        /// Whether the leave-generating bot plays with a wordmap. Defaults on:
        /// leave generation is the most game-heavy job type there is, and a
        /// wordmap is a large speedup. Workers build one on demand.
        #[serde(default = "default_true")]
        use_wordmap: bool,
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
fn default_true() -> bool {
    true
}
fn default_racks_per_batch() -> i32 {
    500
}
fn default_rack_size() -> i32 {
    7
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

    let letterdist_name = require_role(&state.pool, body.letterdist_id, "letterdist").await?;
    require_role(&state.pool, body.layout_id, "layout").await?;

    // Defaulted from config rather than typed, so the form shows the effective
    // value; unparseable text is 0.0.0, which no job would accept.
    let floor = crate::version::Version::parse_or_zero(
        body.min_magpie_version
            .as_deref()
            .unwrap_or(&state.cfg.min_magpie_version),
    );

    let mut tx = state.pool.begin().await?;
    let job = sqlx::query_as::<_, Job>(
        "INSERT INTO jobs
             (job_type, priority, redundancy, variant, letterdist_id, layout_id,
              min_magpie_major, min_magpie_minor, min_magpie_patch, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING *",
    )
    .bind(body.job_type)
    .bind(body.priority)
    .bind(body.redundancy)
    .bind(&body.variant)
    .bind(body.letterdist_id)
    .bind(body.layout_id)
    .bind(floor.major)
    .bind(floor.minor)
    .bind(floor.patch)
    .bind(admin.0.id)
    .fetch_one(&mut *tx)
    .await?;

    insert_job_config(&mut tx, &job, &body.config, &letterdist_name).await?;

    let initialized = registry::initialize_job_state(&mut tx, &job).await?;

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

    // Generation 1's zeroed KLV: a multi-megabyte build and an object-store
    // write, so it happens after the transaction commits rather than inside it.
    registry::initialize_job_artifacts(&state.pool, &state.artifacts, &job).await?;

    Ok((StatusCode::CREATED, Json(CreatedJob { job, initialized })))
}

/// MAGPIE has one value for these for the whole run, not one per player, even
/// though they live on `player_configs` (so that table stays the exhaustive
/// source of what a job asked for -- see the migration comment on
/// `win_pct_model`/`movegen_margin`). A `games`/`game_pairs` job whose two
/// player configs disagree on one of these can't be honored, so job creation
/// rejects it here rather than leaving one worker's value to win silently.
async fn validate_shared_player_options(
    conn: &mut sqlx::PgConnection,
    player1_config_id: Uuid,
    player2_config_id: Uuid,
) -> AppResult<()> {
    if player1_config_id == player2_config_id {
        return Ok(());
    }
    let row = sqlx::query(
        "SELECT p1.win_pct_model AS p1_win_pct_model, p2.win_pct_model AS p2_win_pct_model,
                p1.movegen_margin AS p1_movegen_margin, p2.movegen_margin AS p2_movegen_margin
         FROM player_configs p1, player_configs p2
         WHERE p1.id = $1 AND p2.id = $2",
    )
    .bind(player1_config_id)
    .bind(player2_config_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| AppError::bad_request("player config not found"))?;

    use sqlx::Row;
    let p1_win_pct_model: Option<String> = row.get("p1_win_pct_model");
    let p2_win_pct_model: Option<String> = row.get("p2_win_pct_model");
    if p1_win_pct_model != p2_win_pct_model {
        return Err(AppError::bad_request(
            "player configs disagree on win_pct_model, which MAGPIE cannot vary per player",
        ));
    }
    let p1_movegen_margin: Option<f64> = row.get("p1_movegen_margin");
    let p2_movegen_margin: Option<f64> = row.get("p2_movegen_margin");
    if p1_movegen_margin != p2_movegen_margin {
        return Err(AppError::bad_request(
            "player configs disagree on movegen_margin, which MAGPIE cannot vary per player",
        ));
    }
    Ok(())
}

/// The names a player config's files carry, for the compatibility check.
async fn player_file_names(
    conn: &mut sqlx::PgConnection,
    player_config_id: Uuid,
) -> AppResult<(String, String)> {
    let row: (String, String) = sqlx::query_as(
        "SELECT kwg.name, klv.name
         FROM player_configs pc
         JOIN input_data kwg ON kwg.id = pc.kwg_id
         JOIN input_data klv ON klv.id = pc.klv_id
         WHERE pc.id = $1",
    )
    .bind(player_config_id)
    .fetch_optional(conn)
    .await?
    .ok_or_else(|| AppError::bad_request("no such player config"))?;
    Ok(row)
}

/// MAGPIE decides compatibility from names, and birdtest must not be able to
/// build a job MAGPIE would refuse to load.
async fn validate_player_compatibility(
    conn: &mut sqlx::PgConnection,
    players: &[(&str, Uuid)],
    letterdist_name: &str,
) -> AppResult<()> {
    let mut names = Vec::new();
    for (label, id) in players {
        let (lexicon, leaves) = player_file_names(&mut *conn, *id).await?;
        names.push((*label, lexicon, leaves));
    }
    let files: Vec<crate::compat::PlayerFiles<'_>> = names
        .iter()
        .map(|(label, lexicon, leaves)| crate::compat::PlayerFiles {
            label,
            lexicon,
            leaves,
        })
        .collect();
    crate::compat::validate_job_files(&files, letterdist_name)
}

async fn insert_job_config(
    conn: &mut sqlx::PgConnection,
    job: &Job,
    config: &JobTypeConfig,
    letterdist_name: &str,
) -> AppResult<()> {
    // The untagged config must actually match the declared job type, or the job
    // would exist with no config row and never dispatch anything.
    let mismatch = || AppError::bad_request("config fields do not match the requested job_type");

    match (job.job_type, config) {
        (
            JobType::OpeningRack,
            JobTypeConfig::OpeningRack { player_config_id, racks_per_batch, rack_size },
        ) => {
            validate_player_compatibility(
                &mut *conn,
                &[("player", *player_config_id)],
                letterdist_name,
            )
            .await?;
            // Counting the space is cheap -- a small dynamic-programming table
            // over the letter distribution -- and recording it here means the
            // scheduler can tell when the job is exhausted without re-deriving
            // it on every claim. It counts over the *pinned* bytes: a count
            // taken from a different copy of the distribution would size a
            // universe the workers never play in.
            let job_data = crate::jobs::load_job_data(&mut *conn, job.id).await?;
            let total_racks =
                crate::jobs::opening_rack::total_racks(&job_data.letterdist, *rack_size);
            sqlx::query(
                "INSERT INTO job_opening_rack_config
                     (job_id, player_config_id,
                      racks_per_batch, rack_size, total_racks)
                 VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(job.id)
            .bind(player_config_id)
            .bind(racks_per_batch)
            .bind(rack_size)
            .bind(total_racks)
            .execute(conn)
            .await?;
        }
        (
            JobType::Games,
            JobTypeConfig::Game {
                player1_config_id, player2_config_id, games_per_batch,
                min_games, max_games, sprt_alpha, sprt_beta, elo_low, elo_high,
                capture_positions,
            },
        ) => {
            validate_shared_player_options(&mut *conn, *player1_config_id, *player2_config_id)
                .await?;
            validate_player_compatibility(
                &mut *conn,
                &[("player1", *player1_config_id), ("player2", *player2_config_id)],
                letterdist_name,
            )
            .await?;
            sqlx::query(
                "INSERT INTO job_game_config
                     (job_id, player1_config_id,
                      player2_config_id, games_per_batch, min_games, max_games, sprt_alpha, sprt_beta,
                      elo_low, elo_high, capture_positions)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
            )
            .bind(job.id)
            .bind(player1_config_id).bind(player2_config_id)
            .bind(games_per_batch).bind(min_games).bind(max_games)
            .bind(sprt_alpha).bind(sprt_beta).bind(elo_low).bind(elo_high)
            .bind(capture_positions)
            .execute(conn)
            .await?;
        }
        (
            JobType::GamePairs,
            JobTypeConfig::GamePair {
                player1_config_id, player2_config_id, pairs_per_batch,
                min_pairs, max_pairs, sprt_alpha, sprt_beta, elo_low, elo_high,
                capture_positions,
            },
        ) => {
            validate_shared_player_options(&mut *conn, *player1_config_id, *player2_config_id)
                .await?;
            validate_player_compatibility(
                &mut *conn,
                &[("player1", *player1_config_id), ("player2", *player2_config_id)],
                letterdist_name,
            )
            .await?;
            sqlx::query(
                "INSERT INTO job_game_pair_config
                     (job_id, player1_config_id,
                      player2_config_id, pairs_per_batch, min_pairs, max_pairs, sprt_alpha, sprt_beta,
                      elo_low, elo_high, capture_positions)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
            )
            .bind(job.id)
            .bind(player1_config_id).bind(player2_config_id)
            .bind(pairs_per_batch).bind(min_pairs).bind(max_pairs)
            .bind(sprt_alpha).bind(sprt_beta).bind(elo_low).bind(elo_high)
            .bind(capture_positions)
            .execute(conn)
            .await?;
        }
        (
            JobType::LeaveGeneration,
            JobTypeConfig::Leave {
                kwg_id, num_iterations,
                generation_count, target_rack_count, racks_per_task, max_leave_size,
                use_wordmap,
            },
        ) => {
            let lexicon: (String, String) = sqlx::query_as(
                "SELECT role, name FROM input_data WHERE id = $1",
            )
            .bind(kwg_id)
            .fetch_optional(&mut *conn)
            .await?
            .ok_or_else(|| AppError::bad_request("no such input data row"))?;
            if lexicon.0 != "kwg" {
                return Err(AppError::bad_request(format!(
                    "expected a kwg row, but {} is a {} row",
                    lexicon.1, lexicon.0
                )));
            }
            // No leaves to check: every generation plays with a server-built
            // KLV, generation 1's being a zeroed one.
            if !crate::compat::lex_ld_compat(&lexicon.1, letterdist_name) {
                return Err(AppError::bad_request(format!(
                    "lexicon {:?} is not compatible with letter distribution {letterdist_name:?}",
                    lexicon.1
                )));
            }
            sqlx::query(
                "INSERT INTO job_leave_config
                     (job_id, kwg_id, num_iterations,
                      generation_count, target_rack_count, racks_per_task,
                      max_leave_size, use_wordmap)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
            )
            .bind(job.id).bind(kwg_id)
            .bind(num_iterations).bind(generation_count).bind(target_rack_count)
            .bind(racks_per_task).bind(max_leave_size).bind(use_wordmap)
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

/// What a job is about to lose, as a single line for `audit_log.reason`.
///
/// Counted inside the same transaction as the deletion that follows, so it
/// describes exactly what that statement removes. Cheap relative to the delete
/// itself, and the only record of the job's size that survives it.
async fn job_census(conn: &mut sqlx::PgConnection, job_id: Uuid) -> AppResult<String> {
    use sqlx::Row;
    let row = sqlx::query(
        "SELECT
             (SELECT count(*) FROM tasks WHERE job_id = $1)                        AS tasks,
             (SELECT count(*) FROM task_claims c JOIN tasks t ON t.id = c.task_id
               WHERE t.job_id = $1)                                                AS claims,
             (SELECT count(*) FROM game_results r JOIN tasks t ON t.id = r.task_id
               WHERE t.job_id = $1)                                                AS game_results,
             (SELECT count(*) FROM leave_records r JOIN tasks t ON t.id = r.task_id
               WHERE t.job_id = $1)                                                AS leave_records,
             (SELECT count(*) FROM position_analysis_records r JOIN tasks t ON t.id = r.task_id
               WHERE t.job_id = $1)                                                AS positions,
             (SELECT count(*) FROM leave_rack_progress WHERE job_id = $1)          AS rack_progress,
             (SELECT count(*) FROM leave_generation_artifacts WHERE job_id = $1)   AS artifacts,
             (SELECT count(*) FROM player_config_ratings WHERE job_id = $1)        AS ratings",
    )
    .bind(job_id)
    .fetch_one(conn)
    .await?;

    Ok(format!(
        "tasks={} claims={} game_results={} leave_records={} positions={} \
rack_progress={} artifacts={} ratings={}",
        row.get::<i64, _>("tasks"),
        row.get::<i64, _>("claims"),
        row.get::<i64, _>("game_results"),
        row.get::<i64, _>("leave_records"),
        row.get::<i64, _>("positions"),
        row.get::<i64, _>("rack_progress"),
        row.get::<i64, _>("artifacts"),
        row.get::<i64, _>("ratings"),
    ))
}

/// The same, for an account deletion: claims and results are removed with the
/// user, and nothing else records how much work that was.
async fn user_census(conn: &mut sqlx::PgConnection, user_id: Uuid) -> AppResult<String> {
    use sqlx::Row;
    let row = sqlx::query(
        "SELECT
             (SELECT count(*) FROM task_claims WHERE claimed_by_user_id = $1)      AS claims,
             (SELECT count(*) FROM task_claims c
               WHERE c.claimed_by_user_id = $1 AND c.state = 'completed')          AS accepted,
             (SELECT count(*) FROM api_keys WHERE user_id = $1)                    AS api_keys",
    )
    .bind(user_id)
    .fetch_one(conn)
    .await?;

    Ok(format!(
        "claims={} accepted={} api_keys={}",
        row.get::<i64, _>("claims"),
        row.get::<i64, _>("accepted"),
        row.get::<i64, _>("api_keys"),
    ))
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

    // Written before anything is deleted: after this transaction commits, this
    // row is the only surviving description of what the job held.
    let census = job_census(&mut tx, id).await?;
    audit::log_detail(
        &mut tx,
        "job.purged.census",
        admin.0.id,
        "job",
        id.to_string(),
        Some(id),
        census,
    )
    .await?;

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
    registry::initialize_job_state(&mut tx, &job).await?;

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

    // The generation-0 KLV was deleted with the artifacts above; rebuild it, or
    // generation 1 would have nothing to play with.
    registry::initialize_job_artifacts(&state.pool, &state.artifacts, &job).await?;

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
    // The audit rows are written first: `audit_log.job_id` references `jobs`,
    // so they have to exist while the job still does. The census is what a
    // restore is scoped against if this delete turns out to be a mistake.
    let census = job_census(&mut tx, id).await?;
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
    audit::log_detail(
        &mut tx,
        "job.deleted.census",
        admin.0.id,
        "job",
        id.to_string(),
        None,
        census,
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
// Backups and artifacts
// ---------------------------------------------------------------------------

/// Recent backup runs and how stale the newest good one is.
///
/// Read-only, and read from this server's own database rather than from the
/// backup bucket: the backend deliberately holds no credentials for it, so a
/// compromised backend cannot read or replace backups (PLAN.md, "Making backups visible").
async fn backups(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> AppResult<Json<BackupStatus>> {
    Ok(Json(backups::status(&state.pool).await?))
}

#[derive(Deserialize)]
struct RebuildQuery {
    /// Rewrite an object whose bytes no longer hash to what was recorded.
    /// Off by default; see `leave_gen::rebuild_artifacts` for why a mismatch
    /// is not on its own a reason to overwrite.
    #[serde(default)]
    force: bool,
}

/// Recompute a leave-generation job's KLVs from `leave_rack_progress` and
/// report, per generation, whether the object is still present and still
/// hashes to what was recorded when the generation closed.
///
/// This is the repair path for an artifact store that has lost an object — the
/// bytes are derivable, so losing them is recoverable without a restore.
async fn rebuild_artifacts(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    Query(query): Query<RebuildQuery>,
    method: Method,
    headers: HeaderMap,
    jar: CookieJar,
) -> AppResult<Json<Vec<crate::jobs::leave_gen::ArtifactRebuild>>> {
    csrf::verify(&method, &headers, &jar)?;

    let mut conn = state.pool.acquire().await?;
    let job = sqlx::query_as::<_, Job>("SELECT * FROM jobs WHERE id = $1")
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or_else(|| AppError::not_found("no such job"))?;
    if job.job_type != JobType::LeaveGeneration {
        return Err(AppError::bad_request(
            "only leave generation jobs have artifacts to rebuild",
        ));
    }
    let job_data = crate::jobs::load_job_data(&mut conn, job.id).await?;
    drop(conn);

    let report = crate::jobs::leave_gen::rebuild_artifacts(
        &state.pool,
        &state.artifacts,
        job.id,
        &job_data.letterdist,
        query.force,
    )
    .await?;

    let rewritten = report.iter().filter(|r| r.rewritten).count();
    let mismatched = report.iter().filter(|r| !r.matches).count();
    let mut conn = state.pool.acquire().await?;
    audit::log_detail(
        &mut conn,
        "job.artifacts_rebuilt",
        admin.0.id,
        "job",
        job.id.to_string(),
        Some(job.id),
        format!(
            "generations={} rewritten={rewritten} mismatched={mismatched} force={}",
            report.len(),
            query.force
        ),
    )
    .await?;

    Ok(Json(report))
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

    let census = user_census(&mut tx, id).await?;
    audit::log_detail(
        &mut tx,
        "user.deleted.census",
        admin.0.id,
        "user",
        id.to_string(),
        None,
        census,
    )
    .await?;

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
