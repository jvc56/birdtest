use crate::audit;
use crate::auth::{csrf, AdminUser};
use crate::backups::{self, BackupStatus};
use crate::extract::ApiJson;
use crate::error::{AppError, AppResult};
use crate::jobs::registry;
use crate::models::job::{Job, JobStatus, JobType, PlayerConfig};
use crate::state::AppState;
use crate::extract::{ApiPath as Path, ApiQuery as Query};
use axum::extract::State;
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
        .route("/workers", get(super::public::list_workers_admin))
        .route("/workers/bans", get(list_bans))
        .route("/workers/ban", post(ban_worker))
        .route("/workers/ban/:id", delete(unban_worker))
        .route("/audit-log", get(audit_log))
        .route("/input-data", get(list_input_data))
        .route("/input-data/:id", delete(delete_input_data))
        .route("/input-data/imports", post(start_import))
        .route("/input-data/imports/:id", get(get_import))
        .route("/input-data/imports/:id/confirm", post(confirm_import))
        .route("/jobs/:id/data-gaps", get(job_data_gaps))
        // Bulk reads of a job's results are admin operations: the public gets
        // the paginated `/api/jobs/:id/results`. The stream scans from a cursor
        // and holds a pool connection while it does; the export is that scan
        // done once, for a completed job, into a downloadable artifact.
        .route("/jobs/:id/results/stream", get(super::public::job_results_stream))
        .route("/jobs/:id/export", post(start_export).get(get_export))
        .route("/jobs/:id/rebuild-artifacts", post(rebuild_artifacts))
        .route("/jobs/:id/merge-progress", post(merge_leave_progress))
        .route("/derived-data", get(list_derived_data))
        .route("/derived-data/retry", post(retry_derived_data))
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
                  + (SELECT COUNT(*) FROM rating_pools rp
                      WHERE rp.letterdist_id = d.id OR rp.layout_id = d.id)
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
    admin: AdminUser,
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
              + (SELECT COUNT(*) FROM job_leave_config lc WHERE lc.kwg_id = $1)
              + (SELECT COUNT(*) FROM rating_pools rp
                  WHERE rp.letterdist_id = $1 OR rp.layout_id = $1)",
    )
    .bind(id)
    .fetch_one(&state.pool)
    .await?;
    if uses > 0 {
        return Err(AppError::conflict(format!(
            "this file is pinned by {uses} job{s}, player config{s} or rating pool{s}",
            s = if uses == 1 { "" } else { "s" },
        )));
    }

    // Logged in the same transaction as the delete, like every other
    // destructive admin action: the row it names is gone once this commits.
    let mut tx = state.pool.begin().await?;
    // What was built from it goes with it. Nothing pins the file any more, so
    // no job can need those wordmaps or tables, and nothing else ever reads
    // them -- but they hold foreign keys to it, and left in place they made
    // every file anything was ever built from undeletable. Should a job pin
    // the file between the count above and here, its foreign key fails the
    // delete below and this goes back with it.
    sqlx::query(
        "DELETE FROM derived_data WHERE kwg_id = $1 OR klv_id = $1 OR letterdist_id = $1",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    let deleted = sqlx::query("DELETE FROM input_data WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    if deleted.rows_affected() == 0 {
        return Err(AppError::not_found("no such input data row"));
    }
    audit::log(
        &mut tx,
        "input_data.deleted",
        Some(admin.0.id),
        None,
        Some("input_data"),
        Some(id.to_string()),
        None,
    )
    .await?;
    tx.commit().await?;
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
    ApiJson(body): ApiJson<StartImportBody>,
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
                                 content, object_key, imported_by)
         SELECT r.path, r.role, r.name, r.sha256, r.bytes, $2, r.content,
                r.object_key, $3
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
    /// Required: how many ranked moves per position are reported and kept.
    num_plays_recorded: i32,
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
    ApiJson(body): ApiJson<CreatePlayerConfigBody>,
) -> AppResult<(StatusCode, Json<PlayerConfig>)> {
    csrf::verify(&method, &headers, &jar)?;

    validate_player_config_body(&body)?;

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

    // MAGPIE decides per player whether to simulate on plies alone (autoplay
    // reads `sim_args->num_plies > 0`), so that is what a simmer is here too.
    // A config with simulation settings but no plies would be carried as a
    // simmer -- made to name a win% model, rated as one -- and play statically
    // on every worker, so it is refused. `num_plays` is not a simulation
    // setting: an opening-rack analysis sizes its move list from it, simulating
    // or not.
    let simming = body.num_plies.is_some_and(|plies| plies >= 1);
    let states_simulation = body.max_iterations.is_some()
        || body.stopping_pct.is_some()
        || body.use_inference.is_some()
        || body.time_limit_secs.is_some()
        || body.min_play_iterations.is_some()
        || body.threshold.is_some()
        || body.sampling_rule.is_some()
        || body.inference_margin.is_some()
        || body.utility_w_winpct.is_some()
        || body.utility_w_spread.is_some()
        || body.utility_spread_scale.is_some();
    if states_simulation && !simming {
        return Err(AppError::bad_request("player config is invalid")
            .with_field("num_plies", "a simming player must simulate at least 1 ply"));
    }
    // A simmer's candidates are the top plays by equity: autoplay's simulating
    // player generates them that way whatever the config says, so a `score`
    // simmer would mean equity in a games job and score in an opening-rack one
    // (whose executor sorts by the player's strategy) -- one config, two
    // players. Refused rather than given two meanings.
    if simming && body.sort_strategy.as_deref() == Some("score") {
        return Err(AppError::bad_request("player config is invalid").with_field(
            "sort_strategy",
            "a simming player's candidates are the best plays by equity, in games jobs \
             whatever the config says; use 'equity' (or leave it out) for a simmer",
        ));
    }
    // A simulation stops at whichever comes first, its iteration budget or its
    // time limit, and a time limit makes how far it gets depend on the
    // contributor's hardware: two honest workers would rank the same position
    // differently. MAGPIE applies a limit only above 0, and a null here means
    // its 60-second default, so a simmer states 0 and an iteration budget --
    // without one, nothing but the stopping condition would end a simulation.
    if simming {
        let mut err = AppError::bad_request("player config is invalid");
        if body.max_iterations.is_none() {
            err = err.with_field("max_iterations", "a simming player must set an iteration budget");
        }
        if body.time_limit_secs != Some(0) {
            err = err.with_field(
                "time_limit_secs",
                "must be 0 for a simming player: a time limit makes results depend on the \
                 contributor's hardware, so the iteration budget bounds a simulation instead",
            );
        }
        if !err.fields.is_empty() {
            return Err(err);
        }
    }
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

    // Every setting a request states is written into the row now, from
    // MAGPIE's defaults where the body leaves it out, so a task built from this
    // config means the same thing on every MAGPIE release (see
    // `magpie_defaults`). A static player's simulation settings stay NULL:
    // nothing reads them, and the schema's CHECK holds the two sets apart.
    use crate::magpie_defaults as defaults;
    let sort_strategy =
        body.sort_strategy.clone().unwrap_or_else(|| defaults::SORT_STRATEGY.to_string());
    let num_plies = body.num_plies.unwrap_or(0);
    let num_plays = body.num_plays.unwrap_or(defaults::NUM_PLAYS);
    let num_plies_recorded = body.num_plies_recorded.unwrap_or(defaults::NUM_PLIES_RECORDED);
    let movegen_margin = body.movegen_margin.unwrap_or(defaults::MOVEGEN_MARGIN);
    let stopping_pct = simming.then(|| body.stopping_pct.unwrap_or(defaults::STOPPING_PCT));
    let use_inference = simming.then(|| body.use_inference.unwrap_or(defaults::USE_INFERENCE));
    let min_play_iterations =
        simming.then(|| body.min_play_iterations.unwrap_or(defaults::MIN_PLAY_ITERATIONS));
    let threshold = simming
        .then(|| body.threshold.clone().unwrap_or_else(|| defaults::THRESHOLD.to_string()));
    let sampling_rule = simming.then(|| {
        body.sampling_rule.clone().unwrap_or_else(|| defaults::SAMPLING_RULE.to_string())
    });
    let inference_margin =
        simming.then(|| body.inference_margin.unwrap_or(defaults::INFERENCE_MARGIN));
    let utility_w_winpct =
        simming.then(|| body.utility_w_winpct.unwrap_or(defaults::UTILITY_W_WINPCT));
    let utility_w_spread =
        simming.then(|| body.utility_w_spread.unwrap_or(defaults::UTILITY_W_SPREAD));
    let utility_spread_scale =
        simming.then(|| body.utility_spread_scale.unwrap_or(defaults::UTILITY_SPREAD_SCALE));

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
    .bind(&sort_strategy)
    .bind(body.kwg_id)
    .bind(body.klv_id)
    .bind(body.winpct_id)
    .bind(body.cloned_from_id)
    .bind(body.max_iterations)
    .bind(num_plies)
    .bind(num_plies_recorded)
    .bind(num_plays)
    .bind(body.num_plays_recorded)
    .bind(stopping_pct)
    .bind(use_inference)
    .bind(body.time_limit_secs)
    .bind(body.use_wordmap.unwrap_or(false))
    // Absent means no, for both: a rack info table is a large, slow thing to
    // provision, and a config that did not ask for one must not get one
    // because a default said so.
    .bind(body.use_rit.unwrap_or(false))
    .bind(min_play_iterations)
    .bind(&threshold)
    .bind(&sampling_rule)
    .bind(inference_margin)
    .bind(utility_w_winpct)
    .bind(utility_w_spread)
    .bind(utility_spread_scale)
    .bind(movegen_margin)
    .bind(admin.0.id)
    .fetch_one(&state.pool)
    .await
    .map_err(|e| match body.cloned_from_id {
        Some(id) => super::ratings::unknown_config(
            e.into(),
            "player_configs_cloned_from_id_fkey",
            "cloned_from_id",
            id,
        ),
        None => e.into(),
    })?;

    Ok((StatusCode::CREATED, Json(config)))
}

/// Numbers MAGPIE would refuse, or silently read as "use your own default".
///
/// MAGPIE validates these too, but only on a contributor's machine, after a
/// job has been built on the config and dispatched: every worker would fail
/// the task, and the job would sit there producing nothing. Refusing at
/// creation puts the error in front of the admin who can fix it.
const MAGPIE_MAX_PLIES: i32 = 25;
const MAGPIE_MAX_CAPTURED_PLIES: i32 = 10;

fn validate_player_config_body(body: &CreatePlayerConfigBody) -> AppResult<()> {
    let mut err = AppError::bad_request("player config is invalid");
    if body.name.trim().is_empty() {
        err = err.with_field("name", "must not be empty");
    }
    let positive = [
        ("max_iterations", body.max_iterations),
        ("num_plays", body.num_plays),
        ("num_plies_recorded", body.num_plies_recorded),
        ("min_play_iterations", body.min_play_iterations),
    ];
    for (field, value) in positive {
        if value.is_some_and(|v| v < 1) {
            err = err.with_field(field, "must be at least 1");
        }
    }
    if body.num_plays_recorded < 1 {
        err = err.with_field("num_plays_recorded", "must be at least 1");
    }
    if body.num_plies.is_some_and(|v| v < 0) {
        err = err.with_field("num_plies", "must not be negative");
    }
    // MAGPIE's own limits (`MAX_PLIES` in sim_defs.h; a captured position
    // keeps at most `CAPTURED_PLAY_MAX_PLIES` in autoplay_results.c). Past the
    // first every worker failed every task of the job; past the second the
    // plies were cut off without a word.
    if body.num_plies.is_some_and(|v| v > MAGPIE_MAX_PLIES) {
        err = err.with_field("num_plies", format!("must be at most {MAGPIE_MAX_PLIES}"));
    }
    if body.num_plies_recorded.is_some_and(|v| v > MAGPIE_MAX_CAPTURED_PLIES) {
        err = err.with_field(
            "num_plies_recorded",
            format!("must be at most {MAGPIE_MAX_CAPTURED_PLIES}"),
        );
    }
    if body.time_limit_secs.is_some_and(|v| v < 0) {
        err = err.with_field("time_limit_secs", "must not be negative");
    }
    if body.stopping_pct.is_some_and(|v| !(v > 0.0 && v < 100.0)) {
        err = err.with_field("stopping_pct", "must be strictly between 0 and 100");
    }
    let non_negative = [
        ("inference_margin", body.inference_margin),
        ("movegen_margin", body.movegen_margin),
        ("utility_w_winpct", body.utility_w_winpct),
        ("utility_w_spread", body.utility_w_spread),
    ];
    for (field, value) in non_negative {
        if value.is_some_and(|v| !v.is_finite() || v < 0.0) {
            err = err.with_field(field, "must be a finite, non-negative number");
        }
    }
    if body.utility_spread_scale.is_some_and(|v| !v.is_finite() || v <= 0.0) {
        err = err.with_field("utility_spread_scale", "must be a finite, positive number");
    }
    // `use_rit` was refused outright until the server could check a table. A
    // rack info table is not an exact accelerator the way a wordmap is: each
    // entry carries precomputed leave values, which move generation uses in
    // place of the loaded leaves. It was found by lexicon name alone, recorded
    // nothing about the KLV it was built from, and was covered by no digest,
    // so a player whose leaves are not that KLV -- or a contributor whose
    // table is older than their leaves -- ranked moves on the wrong values
    // with nothing to say so.
    //
    // Both halves of that are now closed. The server builds the table for this
    // exact (lexicon, leaves) pair with its own pinned MAGPIE and sends the
    // hash with every claim, and the file is named for the pair rather than
    // the lexicon, so two jobs on one lexicon with different leaves cannot
    // share one. A job that asks for a table waits until it is built; see
    // `derived` and MAGPIE_DEPENDENCY.md.
    if err.fields.is_empty() {
        Ok(())
    } else {
        Err(err)
    }
}

/// Player configs are immutable, so there is no update endpoint; deletion is
/// only allowed while nothing references the config.
async fn delete_player_config(
    State(state): State<AppState>,
    admin: AdminUser,
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
             UNION ALL SELECT 1 FROM rating_pools WHERE anchor_player_config_id = $1
             UNION ALL SELECT 1 FROM rating_pool_members WHERE player_config_id = $1
             UNION ALL SELECT 1 FROM player_config_ratings WHERE player_config_id = $1
             UNION ALL SELECT 1 FROM player_configs WHERE cloned_from_id = $1
         )",
    )
    .bind(id)
    .fetch_one(&state.pool)
    .await?;

    if referenced {
        return Err(AppError::conflict(
            "a job, a rating pool, a rating history or a clone references this player config",
        ));
    }

    let mut tx = state.pool.begin().await?;
    let deleted = sqlx::query("DELETE FROM player_configs WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    if deleted.rows_affected() == 0 {
        return Err(AppError::not_found("no such player config"));
    }
    audit::log(
        &mut tx,
        "player_config.deleted",
        Some(admin.0.id),
        None,
        Some("player_config"),
        Some(id.to_string()),
        None,
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Reads an `input_data` row's name, insisting it is the role the caller
/// expects. The foreign keys cannot express this -- every one of them points at
/// the same table -- so it is validated wherever a role column is written.
pub(crate) async fn require_role(
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
fn default_true() -> bool {
    true
}
fn default_racks_per_batch() -> i32 {
    500
}
fn default_rack_size() -> i32 {
    7
}

/// Creation writes no rows up front for any job type: no tasks, and no
/// leave-generation rack universe, which the first claim seeds on its own task
/// as it does every generation's. Seeding generation 1 here held the creating
/// request open for the tens of seconds 3.2 million rows take.
#[derive(Serialize)]
struct CreatedJob {
    job: Job,
}

/// Jobs are always created inactive. Allocation is supplied later, at
/// activation, so the admin sets it while looking at the whole active set.
async fn create_job(
    State(state): State<AppState>,
    admin: AdminUser,
    method: Method,
    headers: HeaderMap,
    jar: CookieJar,
    ApiJson(body): ApiJson<CreateJobBody>,
) -> AppResult<(StatusCode, Json<CreatedJob>)> {
    csrf::verify(&method, &headers, &jar)?;

    validate_job_body(&body)?;

    let letterdist_name = require_role(&state.pool, body.letterdist_id, "letterdist").await?;
    require_role(&state.pool, body.layout_id, "layout").await?;
    // Parsed now as every claim will parse it: a file the server or MAGPIE
    // cannot use -- more letters than MAGPIE holds, a malformed row -- was
    // found by the first claim, as a 500, on a job already created.
    let content: Vec<u8> = sqlx::query_scalar("SELECT content FROM input_data WHERE id = $1")
        .bind(body.letterdist_id)
        .fetch_one(&state.pool)
        .await?;
    crate::jobs::racks::LetterDistribution::parse(&content, &letterdist_name).map_err(|e| {
        AppError::bad_request("the letter distribution cannot be used")
            .with_field("letterdist_id", e.message)
    })?;

    // Defaulted from config rather than typed, so the form shows the effective
    // value; a typed one was checked above.
    let floor = crate::version::Version::parse_or_zero(
        body.min_magpie_version
            .as_deref()
            .unwrap_or(&state.cfg.min_magpie_version),
    );

    let mut tx = state.pool.begin().await?;
    let job = sqlx::query_as::<_, Job>(
        "INSERT INTO jobs
             (job_type, redundancy, variant, letterdist_id, layout_id,
              min_magpie_major, min_magpie_minor, min_magpie_patch, bingo_bonus,
              sim_cutoff, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) RETURNING *",
    )
    .bind(body.job_type)
    .bind(body.redundancy)
    .bind(&body.variant)
    .bind(body.letterdist_id)
    .bind(body.layout_id)
    .bind(floor.major)
    .bind(floor.minor)
    .bind(floor.patch)
    // Written from MAGPIE's defaults, like a player config's settings, so
    // every request states them rather than each worker's build supplying
    // its own.
    .bind(crate::magpie_defaults::BINGO_BONUS)
    .bind(crate::magpie_defaults::SIM_CUTOFF)
    .bind(admin.0.id)
    .fetch_one(&mut *tx)
    .await?;

    insert_job_config(&mut tx, &job, &body.config, &letterdist_name).await?;

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
    registry::initialize_job_artifacts(&state, &job).await?;

    // Queued at creation rather than at activation: a rack info table takes
    // minutes to build, and the admin who creates a job typically activates it
    // in the next breath. Requesting it now means the wait happens while they
    // are still deciding rather than after.
    request_derived_data(&state, job.id).await?;

    Ok((StatusCode::CREATED, Json(CreatedJob { job })))
}

/// The largest opening-rack batch accepted. A task's racks are expanded into
/// its request and every rack comes back analysed in one submission, so this
/// bounds both; 500 is the default.
const MAX_RACKS_PER_BATCH: i32 = 10_000;

/// Settings the schema cannot express and no worker or test could run with.
///
/// Each of these used to be accepted and fail later, far from the admin who
/// typed it: a `games_per_batch` of 0 makes every claim generate the seed the
/// previous claim already took, so the job retries a unique-index violation
/// forever and dispatches nothing; an `elo_low` above `elo_high` inverts the
/// LLR's sign, so SPRT confidently accepts the wrong hypothesis; an `alpha` of
/// 0 or 1 puts a logarithm of zero or infinity in the bounds. Every problem is
/// reported at once, like registration does.
fn validate_job_body(body: &CreateJobBody) -> AppResult<()> {
    let mut err = AppError::bad_request("job settings are invalid");
    if body.redundancy < 1 {
        err = err.with_field("redundancy", "must be at least 1");
    }
    // A leave task's redundant copies replay the same seed only when MAGPIE runs
    // single-threaded; multi-threaded they are different samples, and there is
    // no integrity use for them today. Refused rather than given a meaning.
    if body.job_type == JobType::LeaveGeneration && body.redundancy > 1 {
        err = err.with_field("redundancy", "leave generation runs at redundancy 1");
    }
    if !matches!(body.variant.as_str(), "classic" | "wordsmog") {
        err = err.with_field("variant", "must be 'classic' or 'wordsmog'");
    }
    // Read loosely, a typo ("v1.6.0", "1") was 0.0.0: the lowest floor there
    // is, so a raise meant to keep older builds off the job let them all on.
    if let Some(text) = body.min_magpie_version.as_deref() {
        if crate::version::Version::parse_strict(text).is_none() {
            err = err.with_field("min_magpie_version", "must be a version such as 0.1.1");
        }
    }

    let sprt = |mut err: AppError,
                unit: &str,
                batch: i32,
                min_units: i32,
                max_units: i32,
                alpha: f64,
                beta: f64,
                elo_low: f64,
                elo_high: f64| {
        if batch < 1 {
            err = err.with_field(format!("{unit}s_per_batch"), "must be at least 1");
        }
        if min_units < 0 {
            err = err.with_field(format!("min_{unit}s"), "must not be negative");
        }
        if max_units < 1 {
            err = err.with_field(format!("max_{unit}s"), "must be at least 1");
        }
        for (field, value) in [("sprt_alpha", alpha), ("sprt_beta", beta)] {
            if !(value > 0.0 && value < 1.0) {
                err = err.with_field(field, "must be strictly between 0 and 1");
            }
        }
        if alpha + beta >= 1.0 {
            err = err.with_field("sprt_beta", "sprt_alpha + sprt_beta must be below 1");
        }
        if !(elo_low.is_finite() && elo_high.is_finite() && elo_low < elo_high) {
            err = err.with_field("elo_high", "must be a finite number greater than elo_low");
        }
        err
    };

    err = match &body.config {
        JobTypeConfig::OpeningRack { racks_per_batch, rack_size, .. } => {
            if !(1..=MAX_RACKS_PER_BATCH).contains(racks_per_batch) {
                err = err.with_field(
                    "racks_per_batch",
                    format!("must be between 1 and {MAX_RACKS_PER_BATCH}"),
                );
            }
            if !(1..=7).contains(rack_size) {
                err = err.with_field("rack_size", "must be between 1 and 7");
            }
            err
        }
        JobTypeConfig::Game {
            games_per_batch, min_games, max_games, sprt_alpha, sprt_beta, elo_low, elo_high, ..
        } => sprt(
            err, "game", *games_per_batch, *min_games, *max_games, *sprt_alpha, *sprt_beta,
            *elo_low, *elo_high,
        ),
        JobTypeConfig::GamePair {
            pairs_per_batch, min_pairs, max_pairs, sprt_alpha, sprt_beta, elo_low, elo_high, ..
        } => sprt(
            err, "pair", *pairs_per_batch, *min_pairs, *max_pairs, *sprt_alpha, *sprt_beta,
            *elo_low, *elo_high,
        ),
        JobTypeConfig::Leave {
            num_iterations, generation_count, target_rack_count, racks_per_task, ..
        } => {
            for (field, value) in [
                ("num_iterations", *num_iterations),
                ("generation_count", *generation_count),
                ("target_rack_count", *target_rack_count),
                ("racks_per_task", *racks_per_task),
            ] {
                if value < 1 {
                    err = err.with_field(field, "must be at least 1");
                }
            }
            err
        }
    };

    if err.fields.is_empty() {
        Ok(())
    } else {
        Err(err)
    }
}

/// An opening-rack job asks for a *ranked list* per rack, and a static player
/// with a `best` recorder cannot produce one.
///
/// `-r best` is `MOVE_RECORD_BEST`: move generation keeps the single top play
/// and discards the rest, so a static player's batch comes back with exactly
/// one move per rack however many the config says to record. Verified against
/// MAGPIE: `generate` on an opening rack reports "1 of 1 plays" under
/// `-r1 best` and 100 under `-r1 all`.
///
/// Nothing downstream notices. The racks are analysed, the results are
/// accepted, `racks_analyzed` climbs, and the corpus quietly holds a
/// hundredth of the analysis it was configured for. So the contradiction is
/// refused where it is introduced rather than discovered in the data later.
///
/// A simulating player is not refused: its candidates are every play up to
/// `num_plays` whatever its recorder, in opening-rack jobs as in autoplay, so
/// a `best` simmer ranks as many moves as it records. (MAGPIE's opening-rack
/// executor once generated them with the recorder, so a `best` simmer reported
/// the static top play -- PLAN.md.) `best` with `num_plays_recorded = 1` is
/// coherent for any player: "the best opening play for every rack" is a real
/// job.
///
/// The same quiet shortfall comes from a `num_plays` below `num_plays_recorded`,
/// static or simulating: an opening-rack analysis sizes its move list from
/// `num_plays`, so no rack can come back with more, and the job would store
/// fewer moves than it asks for.
async fn validate_opening_rack_player(
    conn: &mut sqlx::PgConnection,
    player_config_id: Uuid,
) -> AppResult<()> {
    let (recorder, recorded, plies, plays) = sqlx::query_as::<_, (String, i32, i32, i32)>(
        "SELECT recorder_type, num_plays_recorded, num_plies, num_plays
         FROM player_configs WHERE id = $1",
    )
    .bind(player_config_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| AppError::bad_request("player config not found"))?;

    if recorder == "best" && recorded > 1 && plies == 0 {
        return Err(AppError::bad_request(
            "an opening-rack job cannot rank moves with a static 'best' recorder",
        )
        .with_field(
            "player_config_id",
            format!(
                "this static config records the single best move, so every rack would come \
                 back with one move rather than the {recorded} it asks for. Use a config with \
                 recorder_type 'all' or 'equity', a simulating one, or set \
                 num_plays_recorded to 1."
            ),
        ));
    }
    if plays < recorded {
        return Err(AppError::bad_request(
            "an opening-rack job cannot record more moves than its player generates",
        )
        .with_field(
            "player_config_id",
            format!(
                "this config generates {plays} plays per rack, so every rack would come back \
                 with at most {plays} moves rather than the {recorded} it asks for. Use a \
                 config with num_plays of at least {recorded}, or a lower num_plays_recorded."
            ),
        ));
    }
    Ok(())
}

/// MAGPIE has one value for these for the whole run, not one per player, even
/// though they live on `player_configs` (so that table stays the exhaustive
/// source of what a job asked for -- see the migration comment on
/// `winpct_id`/`movegen_margin`). A `games`/`game_pairs` job whose two
/// player configs disagree on one of these can't be honored, so job creation
/// rejects it here rather than leaving one worker's value to win silently.
///
/// The win% model is compared by `input_data` id rather than by name: the id
/// is what pins the bytes, and two rows can share a name across tarball
/// versions while holding different content.
async fn validate_shared_player_options(
    conn: &mut sqlx::PgConnection,
    player1_config_id: Uuid,
    player2_config_id: Uuid,
) -> AppResult<()> {
    if player1_config_id == player2_config_id {
        return Ok(());
    }
    let row = sqlx::query(
        "SELECT p1.winpct_id AS p1_winpct_id, p2.winpct_id AS p2_winpct_id,
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
    // Only two *simming* players can disagree. A static player has no win%
    // model at all (`winpct_id` is refused on one), so comparing with plain
    // equality made every static-versus-simmer job -- the mix PLAN.md promises
    // a games job supports -- fail here as a "disagreement". The worker loads
    // the model from whichever player states one.
    let p1_winpct_id: Option<Uuid> = row.get("p1_winpct_id");
    let p2_winpct_id: Option<Uuid> = row.get("p2_winpct_id");
    if matches!((p1_winpct_id, p2_winpct_id), (Some(p1), Some(p2)) if p1 != p2) {
        return Err(AppError::bad_request(
            "player configs disagree on the win% model, which MAGPIE cannot vary per player",
        ));
    }
    let p1_movegen_margin: f64 = row.get("p1_movegen_margin");
    let p2_movegen_margin: f64 = row.get("p2_movegen_margin");
    if p1_movegen_margin != p2_movegen_margin {
        return Err(AppError::bad_request(
            "player configs disagree on movegen_margin, which MAGPIE cannot vary per player",
        ));
    }
    Ok(())
}

/// A capture job's simmers must already consider at least as many plays as are
/// captured.
///
/// With `capture_positions` on, MAGPIE's autoplay raises each simming player's
/// candidate count to the capture cap -- player 1's `num_plays_recorded` -- so
/// there are enough ranked plays to record. A simmer configured for fewer would
/// then consider more candidates with capture on than off, and turning capture
/// on, which is meant only to decide what is kept, would change the games.
async fn validate_capture_play_cap(
    conn: &mut sqlx::PgConnection,
    player1_config_id: Uuid,
    player2_config_id: Uuid,
) -> AppResult<()> {
    let cap: i32 =
        sqlx::query_scalar("SELECT num_plays_recorded FROM player_configs WHERE id = $1")
            .bind(player1_config_id)
            .fetch_optional(&mut *conn)
            .await?
            .ok_or_else(|| AppError::bad_request("player config not found"))?;
    let players = sqlx::query_as::<_, (String, i32, i32)>(
        "SELECT name, num_plies, num_plays FROM player_configs WHERE id = ANY($1)",
    )
    .bind(vec![player1_config_id, player2_config_id])
    .fetch_all(&mut *conn)
    .await?;
    for (name, plies, plays) in players {
        if plies > 0 && plays < cap {
            return Err(AppError::bad_request("job settings are invalid").with_field(
                "capture_positions",
                format!(
                    "{name} simulates {plays} candidate plays, fewer than the {cap} captured \
                     per position; with capture on MAGPIE would raise it and play different \
                     games. Use a config with num_plays of at least {cap}, or lower player \
                     1's num_plays_recorded."
                ),
            ));
        }
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
            validate_opening_rack_player(&mut *conn, *player_config_id).await?;
            // Counting the space is cheap -- a small dynamic-programming table
            // over the letter distribution -- and recording it here means the
            // scheduler can tell when the job is exhausted without re-deriving
            // it on every claim. It counts over the *pinned* bytes: a count
            // taken from a different copy of the distribution would size a
            // universe the workers never play in.
            let job_data = crate::jobs::load_job_data(&mut *conn, job.id).await?;
            let total_racks =
                crate::jobs::opening_rack::total_racks(&job_data.letterdist, *rack_size)?;
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
            if *capture_positions {
                validate_capture_play_cap(&mut *conn, *player1_config_id, *player2_config_id)
                    .await?;
            }
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
            if *capture_positions {
                validate_capture_play_cap(&mut *conn, *player1_config_id, *player2_config_id)
                    .await?;
            }
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
                generation_count, target_rack_count, racks_per_task, use_wordmap,
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
            // Every generation seeds and hands out full racks over the pinned
            // distribution, so one whose racks cannot be spelt is refused now
            // rather than at the first claim.
            let job_data = crate::jobs::load_job_data(&mut *conn, job.id).await?;
            crate::jobs::racks::RackIndex::new(&job_data.letterdist, crate::jobs::leave_gen::RACK_SIZE)?;
            sqlx::query(
                "INSERT INTO job_leave_config
                     (job_id, kwg_id, num_iterations,
                      generation_count, target_rack_count, racks_per_task, use_wordmap)
                 VALUES ($1,$2,$3,$4,$5,$6,$7)",
            )
            .bind(job.id).bind(kwg_id)
            .bind(num_iterations).bind(generation_count).bind(target_rack_count)
            .bind(racks_per_task).bind(use_wordmap)
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

/// Activation sets the allocation. The active jobs must sum to at most 100%,
/// which is checked here rather than in the schema — the intermediate states
/// an admin passes through while rebalancing would violate a DB constraint
/// even when the end state is fine. An allocation of 0 is accepted and means
/// what `inactive` means: the job is offered to nobody until it is raised.
async fn activate_job(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    method: Method,
    headers: HeaderMap,
    jar: CookieJar,
    ApiJson(body): ApiJson<ActivateBody>,
) -> AppResult<Json<Job>> {
    csrf::verify(&method, &headers, &jar)?;
    let purges = refuse_while_purging(&state, id)?;

    if !(0..=100).contains(&body.allocation) {
        return Err(AppError::bad_request("allocation must be between 0 and 100"));
    }

    // A leave-generation job cannot dispatch without its generation-0 KLV.
    // Creation writes it after committing, so a failed object-store write
    // there leaves a job that exists without one; activating it as-is would
    // make every claim against it fail. Built here, outside the transaction,
    // for the same reason creation builds it outside its own.
    let unlocked = crate::jobstats::load_job(&state.pool, id).await?;
    if !registry::job_artifacts_ready(&state.pool, &unlocked).await? {
        registry::initialize_job_artifacts(&state, &unlocked).await?;
    }
    // Again at activation, because the builder may have moved since creation:
    // a deployment with a newer MAGPIE needs this job's files rebuilt under
    // the new builder before it can dispatch, and nothing else would ask.
    request_derived_data(&state, id).await?;

    let mut tx = state.pool.begin().await?;
    let job = load_job_for_update(&mut tx, id).await?;
    refuse_if_purged_since(&state, id, purges)?;
    if job.status == JobStatus::Completed {
        return Err(AppError::conflict("a completed job cannot be reactivated"));
    }

    // Serializes activations. The row lock above covers only this job, so two
    // jobs activated at once would each read the other's allocation as absent
    // and together exceed 100%.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('birdtest.activate'))")
        .execute(&mut *tx)
        .await?;

    let others = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT SUM(allocation) FROM jobs WHERE status = 'active' AND id <> $1",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await?
    .unwrap_or(0);

    if others + body.allocation as i64 > 100 {
        return Err(AppError::conflict(format!(
            "the other active jobs already allocate {others}% — {}% is the most this job can take",
            100 - others
        )));
    }

    sqlx::query("UPDATE jobs SET status = 'active', allocation = $1, activated_at = now() WHERE id = $2")
        .bind(body.allocation)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    // The job joins the others level with the one furthest behind, rather
    // than with a lifetime deficit to work off at their expense. Activation is
    // also how an allocation is changed, and a new allocation rescales the
    // ratio, so this runs every time. Under the activation lock, so two jobs
    // activated together each see the other or neither.
    crate::scheduler::join_at_parity(&mut tx, id, state.cfg.heartbeat_timeout).await?;
    let updated = sqlx::query_as::<_, Job>("SELECT * FROM jobs WHERE id = $1")
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;

    audit::log_status_change(
        &mut tx,
        "job.activated",
        admin.0.id,
        id,
        status_name(job.status),
        "active",
    )
    .await?;
    tx.commit().await?;
    super::worker::push_after_change(&state, id);
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
    let purges = refuse_while_purging(&state, id)?;

    let mut tx = state.pool.begin().await?;
    let before = load_job_for_update(&mut tx, id).await?;
    refuse_if_purged_since(&state, id, purges)?;
    // Completion is final. Flipping a completed job to inactive would be a
    // way around that rule: activation only refuses jobs that are *currently*
    // completed, so deactivate-then-activate would restart it.
    if before.status == JobStatus::Completed {
        return Err(AppError::conflict("a completed job cannot be deactivated"));
    }
    let job = sqlx::query_as::<_, Job>(
        "UPDATE jobs SET status = 'inactive', deactivated_at = now() WHERE id = $1 RETURNING *",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;

    audit::log_status_change(
        &mut tx,
        "job.deactivated",
        admin.0.id,
        id,
        status_name(before.status),
        "inactive",
    )
    .await?;
    tx.commit().await?;
    super::worker::push_after_change(&state, id);
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
    let purges = refuse_while_purging(&state, id)?;

    let mut tx = state.pool.begin().await?;
    let before = load_job_for_update(&mut tx, id).await?;
    refuse_if_purged_since(&state, id, purges)?;
    let job =
        sqlx::query_as::<_, Job>("UPDATE jobs SET status = 'completed' WHERE id = $1 RETURNING *")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;

    audit::log_status_change(
        &mut tx,
        "job.completed",
        admin.0.id,
        id,
        status_name(before.status),
        "completed",
    )
    .await?;
    tx.commit().await?;
    super::worker::push_after_change(&state, id);
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
             (SELECT count(*) FROM game_results WHERE job_id = $1)                  AS game_results,
             (SELECT count(*) FROM leave_records r JOIN tasks t ON t.id = r.task_id
               WHERE t.job_id = $1)                                                AS leave_records,
             (SELECT count(*) FROM position_analysis_records WHERE job_id = $1)     AS positions,
             (SELECT count(*) FROM leave_rack_progress WHERE job_id = $1)          AS rack_progress,
             (SELECT count(*) FROM leave_rack_staging WHERE job_id = $1)           AS staged_results,
             (SELECT count(*) FROM leave_generation_artifacts WHERE job_id = $1)   AS artifacts",
    )
    .bind(job_id)
    .fetch_one(conn)
    .await?;

    Ok(format!(
        "tasks={} claims={} game_results={} leave_records={} positions={} \
rack_progress={} staged_results={} artifacts={}",
        row.get::<i64, _>("tasks"),
        row.get::<i64, _>("claims"),
        row.get::<i64, _>("game_results"),
        row.get::<i64, _>("leave_records"),
        row.get::<i64, _>("positions"),
        row.get::<i64, _>("rack_progress"),
        row.get::<i64, _>("staged_results"),
        row.get::<i64, _>("artifacts"),
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
/// What each identity earned on this job, to be given back when its claims
/// are destroyed.
///
/// The counters on `jobs` belong to the job, so a purge simply zeroes them. The
/// ones on `users` and `anonymous_workers` do not: they span every job an
/// identity ever worked on, so a job whose claims are about to disappear has to
/// hand back exactly what it contributed, or the contributor lists read high
/// for good and nothing says why. Must be read *before* the claims go, since it
/// counts them; and it is exact up to the commit, because the caller holds the
/// job's dispatch lock and every open claim (`lock_open_claims`), so no claim
/// of the job can complete in between.
///
/// Read here and written by [`Contributions::give_back`] as the caller's last
/// statement. Written here, as it once was, the update held every
/// contributor's row for the whole of the deletes that follow -- minutes for a
/// large job -- and every request those identities made meanwhile waited on
/// it with a pool connection held: an anonymous worker's `last_seen_at` touch
/// runs on every worker request, and a submission for *any* job bumps its
/// contributor's counter. The fleet stalled exactly as it had on the claims.
///
/// `last_completed_at` is deliberately not rewound. Finding the new maximum
/// means the scan these counters exist to avoid, and it is a display figure
/// that only ever moves forward; a purge can leave it pointing at a time whose
/// task is gone.
struct Contributions {
    users: Vec<(Uuid, i64)>,
    anonymous: Vec<(Uuid, i64)>,
}

impl Contributions {
    async fn count(conn: &mut sqlx::PgConnection, job_id: Uuid) -> AppResult<Self> {
        let users = sqlx::query_as::<_, (Uuid, i64)>(
            "SELECT c.claimed_by_user_id, COUNT(*)::bigint
             FROM task_claims c JOIN tasks t ON t.id = c.task_id
             WHERE t.job_id = $1 AND c.state = 'completed'
               AND c.claimed_by_user_id IS NOT NULL
             GROUP BY 1 ORDER BY 1",
        )
        .bind(job_id)
        .fetch_all(&mut *conn)
        .await?;
        let anonymous = sqlx::query_as::<_, (Uuid, i64)>(
            "SELECT c.claimed_by_anon_uuid, COUNT(*)::bigint
             FROM task_claims c JOIN tasks t ON t.id = c.task_id
             WHERE t.job_id = $1 AND c.state = 'completed'
               AND c.claimed_by_anon_uuid IS NOT NULL
             GROUP BY 1 ORDER BY 1",
        )
        .bind(job_id)
        .fetch_all(&mut *conn)
        .await?;
        Ok(Contributions { users, anonymous })
    }

    /// The caller's last statement before it commits, so the rows are held
    /// for milliseconds. In id order, so two purges sharing contributors lock
    /// them in the same order rather than deadlocking at the end of both.
    async fn give_back(self, conn: &mut sqlx::PgConnection) -> AppResult<()> {
        let (ids, counts): (Vec<Uuid>, Vec<i64>) = self.users.into_iter().unzip();
        // Locked in id order first: the update below locks rows in whatever
        // order its plan visits them, which a sorted array does not decide.
        sqlx::query("SELECT 1 FROM users WHERE id = ANY($1) ORDER BY id FOR NO KEY UPDATE")
            .bind(&ids)
            .execute(&mut *conn)
            .await?;
        sqlx::query(
            "UPDATE users u
             SET tasks_completed = GREATEST(u.tasks_completed - d.n, 0)
             FROM UNNEST($1::uuid[], $2::bigint[]) AS d(id, n)
             WHERE u.id = d.id",
        )
        .bind(&ids)
        .bind(&counts)
        .execute(&mut *conn)
        .await?;
        let (uuids, counts): (Vec<Uuid>, Vec<i64>) = self.anonymous.into_iter().unzip();
        sqlx::query(
            "SELECT 1 FROM anonymous_workers WHERE uuid = ANY($1) ORDER BY uuid FOR NO KEY UPDATE",
        )
        .bind(&uuids)
        .execute(&mut *conn)
        .await?;
        sqlx::query(
            "UPDATE anonymous_workers w
             SET tasks_completed = GREATEST(w.tasks_completed - d.n, 0)
             FROM UNNEST($1::uuid[], $2::bigint[]) AS d(uuid, n)
             WHERE w.uuid = d.uuid",
        )
        .bind(&uuids)
        .bind(&counts)
        .execute(&mut *conn)
        .await?;
        Ok(())
    }
}

/// Wait out every submission, decline and heartbeat in flight on this job's
/// open claims, and hold further ones off until the caller commits.
///
/// A submission locks its claim, then its task, then the job's row. Purge and
/// delete took the job's row first and deleted the claims afterwards -- the
/// opposite order -- so a submission arriving mid-purge waited on the job's row
/// while the purge waited on that submission's claim: a deadlock, which
/// Postgres breaks by failing one of the two. And a submission that committed
/// after `Contributions::count` had counted, but before the delete, credited
/// its identity for a claim the purge then destroyed, so that contributor's
/// total read high for good.
///
/// Locking the open claims first, before the job's row, puts destruction in
/// the order every submission uses, and means `Contributions::count` sees
/// every submission that got in ahead of it. The caller takes the dispatch lock
/// before this, which is what stops new claims appearing meanwhile.
async fn lock_open_claims(conn: &mut sqlx::PgConnection, job_id: Uuid) -> AppResult<()> {
    sqlx::query(
        "SELECT c.id FROM task_claims c JOIN tasks t ON t.id = c.task_id
         WHERE t.job_id = $1 AND c.state = 'claimed'
         FOR UPDATE OF c",
    )
    .bind(job_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

async fn purge_job(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    method: Method,
    headers: HeaderMap,
    jar: CookieJar,
) -> AppResult<Json<PurgeResult>> {
    csrf::verify(&method, &headers, &jar)?;
    let hold = hold_for_purge_or_delete(&state, id)?;
    run_to_completion(purge_body(state, admin.0.id, id, hold)).await
}

const ALREADY_RUNNING: &str = "a purge or delete of this job is running; its result will show \
     on the job's page and in the audit log when it finishes";

/// The hold a purge or delete runs under, taken in the handler: claims skip
/// the job while it runs, and submissions for its claims are answered at once
/// (see `jobs::DispatchHolds`). A purge or delete of the job already running
/// -- one the load balancer stopped waiting for, which the admin page shows as
/// an error and invites a second click on -- is refused rather than started
/// again: the second parked a pool connection on the first's locks and then
/// did all of it over. Checked and taken in one step; checked and then taken
/// in the spawned task, a double click got two.
fn hold_for_purge_or_delete(state: &AppState, id: Uuid) -> AppResult<crate::jobs::DispatchHold> {
    state
        .dispatch_holds
        .try_hold_claims(id, state.cfg.heartbeat_timeout)
        .ok_or_else(|| AppError::conflict(ALREADY_RUNNING))
}

/// Activating, deactivating or completing a job being purged or deleted would
/// wait out the whole operation on its row with a pool connection held --
/// and completing it then finished a job the purge had just emptied.
/// Returns the job's purge count, for [`refuse_if_purged_since`].
fn refuse_while_purging(state: &AppState, id: Uuid) -> AppResult<u64> {
    let taken = state.dispatch_holds.claims_holds_taken(id);
    if state.dispatch_holds.claims_held(id) {
        return Err(AppError::conflict(ALREADY_RUNNING));
    }
    Ok(taken)
}

/// Again under the job's row lock: a purge that took the job between the
/// first check and the lock has committed by the time the lock is had, and
/// acting on the emptied job -- completing it, above all, for good -- is what
/// the check exists to prevent. Compared by count, not by whether a hold is
/// held: a purge of a job with nothing to do after its commit has released
/// its hold by the time the waiter wakes.
fn refuse_if_purged_since(state: &AppState, id: Uuid, taken: u64) -> AppResult<()> {
    if state.dispatch_holds.claims_holds_taken(id) != taken || state.dispatch_holds.claims_held(id) {
        return Err(AppError::conflict(
            "the job was purged while this waited for it; look at it again before acting",
        ));
    }
    Ok(())
}

/// Runs a purge or a delete on a task of its own, and waits for it.
///
/// Spawned so that a request dropped mid-way -- the load balancer's idle
/// timeout, the admin closing the tab -- does not drop the transaction with
/// it. Dropped with the request, the transaction's rollback waited on its
/// connection for the running statement (minutes, for a large job's
/// cascade), while its `DispatchHold`, dropped at once, told claims and
/// submissions the job was free and started the reclaim grace on a job whose
/// claims were still locked. On its own task the operation finishes whatever
/// happens to the request, and the hold lasts at least as long as its locks --
/// a little longer, to the end of the post-commit steps (a leave job's
/// generation-0 rebuild), so a claim cannot start a second build of it
/// alongside; claims skip the job and lifecycle actions answer 409 meanwhile.
async fn run_to_completion<T: Send + 'static>(
    operation: impl std::future::Future<Output = AppResult<T>> + Send + 'static,
) -> AppResult<T> {
    tokio::spawn(operation)
        .await
        .map_err(|e| AppError::internal(format!("the operation's task failed: {e}")))?
}

async fn purge_body(
    state: AppState,
    admin_id: Uuid,
    id: Uuid,
    mut hold: crate::jobs::DispatchHold,
) -> AppResult<Json<PurgeResult>> {
    let mut tx = state.pool.begin().await?;
    // The same lock every claim takes before deciding what to hand out, and
    // for the same reason. A claim in flight has already read the seed cursor
    // and is about to insert its task and its claim row; the deletes below
    // cannot see those uncommitted rows, so without this the purge finishes
    // and the claim then commits a task into the job it just emptied --
    // leaving the seed cursor past zero and `claims_issued` at 1 on a job that
    // was supposed to start over. Taken before the census, so the numbers
    // written to the audit log are the ones actually destroyed.
    //
    // The merge lock comes first, before anything else is held: a merge of a
    // leave job's staged results takes the staged rows and then the per-rack
    // rows, the deletes below take them the other way round, and the two
    // deadlocked -- see `leave_gen::lock_merges`. Every job type takes it; for
    // the ones that stage nothing it is an uncontended lock.
    crate::jobs::leave_gen::lock_merges(&mut tx, id).await?;
    crate::jobs::lock_job_dispatch(&mut tx, id).await?;
    // Before the job's row, for the lock order every submission uses: see
    // `lock_open_claims`.
    lock_open_claims(&mut tx, id).await?;
    let job = load_job_for_update(&mut tx, id).await?;

    // Written before anything is deleted: after this transaction commits, this
    // row is the only surviving description of what the job held.
    let census = job_census(&mut tx, id).await?;
    audit::log_detail(
        &mut tx,
        "job.purged.census",
        admin_id,
        "job",
        id.to_string(),
        Some(id),
        census,
    )
    .await?;

    // Before the claims go, and for the same reason the census is taken first:
    // it counts what is about to be destroyed. Given back last, below.
    let contributions = Contributions::count(&mut tx, id).await?;

    // Records and claims cascade from tasks; leave-gen progress is keyed on
    // the job directly. Ratings are not touched: they belong to rating pools,
    // not jobs, and are a pure function of the results that remain -- the
    // periodic sweep notices the pool's evidence shrank and refits it.
    sqlx::query("DELETE FROM task_claims c USING tasks t WHERE c.task_id = t.id AND t.job_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    // Every counter on the job describes rows this purge is deleting. Left
    // alone, a purged job would restart owing the scheduler every claim it ever
    // had, and reporting progress it no longer has any results for.
    //
    // A completed job goes back to inactive: it has nothing left to be complete
    // about, and a completed job cannot be activated, so one purged in place
    // was an empty job nothing could ever run again -- where the purge is
    // meant to start it over. Active and inactive jobs keep their state.
    sqlx::query(
        "UPDATE jobs SET claims_issued = 0, games_completed = 0, racks_analyzed = 0,
                         tasks_total = 0, tasks_completed = 0, last_completed_at = NULL,
                         sprt_decided_status = NULL, sprt_decided_llr = NULL,
                         sprt_decided_units = NULL,
                         status = CASE WHEN status = 'completed' THEN 'inactive'::job_status
                                       ELSE status END
         WHERE id = $1",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM leave_rack_progress WHERE job_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    // And the accepted results still waiting to be merged into it, with the
    // generations' summaries. Every open claim's submission has either
    // committed its staged row or is held off (`lock_open_claims`), so nothing
    // staged for the old run survives to be folded into the new one. No merge
    // is running: this transaction has held the job's merge lock since before
    // it touched anything (`leave_gen::lock_merges`).
    sqlx::query("DELETE FROM leave_rack_staging WHERE job_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM leave_generation_progress WHERE job_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    // The sweep's cursor with them: the purged job's first claim starts a lap
    // from the beginning of a universe it has yet to seed.
    sqlx::query("DELETE FROM leave_selection_cursors WHERE job_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM leave_generation_artifacts WHERE job_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    // Deleted with the artifacts they produced: a surviving completed row for
    // generation 1 would tell the next claim that its transition is someone
    // else's business, and the job would never close a generation again.
    sqlx::query("DELETE FROM leave_generation_transitions WHERE job_id = $1")
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

    // The generation-1 rack universe just deleted is not written back here. The
    // first claim finds it missing and seeds it on its own task, as it does
    // every generation's; seeding it inline held this transaction -- and with
    // it the job's row and every open claim -- for the tens of seconds 3.2
    // million rows take, while that job's submissions queued behind it.

    audit::log(
        &mut tx,
        "job.purged",
        Some(admin_id),
        None,
        Some("job"),
        Some(id.to_string()),
        Some(id),
    )
    .await?;
    // A job back at zero claims would otherwise be first in every candidate
    // list until it had re-issued as many as the jobs beside it. Last, not
    // beside the counters it follows: the other jobs go on issuing claims for
    // the minutes the cascade above takes, and parity taken before it left the
    // purged job that far behind, heading every candidate list until it had
    // caught up.
    crate::scheduler::join_at_parity(&mut tx, id, state.cfg.heartbeat_timeout).await?;
    // One matrix build per pool on the next sweep, which is what the sweep
    // did every time before it had its cheap check. Before the give-back:
    // this can wait out a running fit, and waiting with every contributor's
    // row locked stalled every submission of theirs, for any job.
    crate::ratings::mark_every_pool_for_refit(&mut tx).await?;
    // Exports describe results this purge deletes. A row left saying `ready`
    // would hand an admin -- and, once the job completed again, every
    // download of its results -- a stable-looking artifact of a job that no
    // longer holds any of it. Deleted with everything else; the objects go
    // once this commits.
    let export_objects = crate::exports::purge(&mut tx, id).await?;
    // Last, and so held for as long as the commit takes: see `Contributions`.
    contributions.give_back(&mut tx).await?;
    tx.commit().await?;
    hold.committed();
    super::worker::push_after_change(&state, id);
    crate::exports::remove_objects(&state, export_objects);

    // After the commit, so a failure here is not the purge failing: it
    // committed, and answering 500 invited a second one. Logged instead.
    //
    // The generation-0 KLV was deleted with the artifacts above; rebuilt here,
    // and if that fails, by the next claim (`Acquired::NeedsZeroGeneration`).
    if let Err(err) = registry::initialize_job_artifacts(&state, &job).await {
        tracing::error!(
            job_id = %id, error = %err.message,
            "rebuilding a purged job's generation-0 KLV failed; the next claim retries it"
        );
    }

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
    let hold = hold_for_purge_or_delete(&state, id)?;
    run_to_completion(delete_body(state, admin.0.id, id, hold)).await
}

/// See [`run_to_completion`] and [`hold_for_purge_or_delete`].
async fn delete_body(
    state: AppState,
    admin_id: Uuid,
    id: Uuid,
    mut hold: crate::jobs::DispatchHold,
) -> AppResult<StatusCode> {
    let mut tx = state.pool.begin().await?;
    // The same locks a purge takes, in the same order and for the same
    // reasons: no claim is issued meanwhile, and no submission is between its
    // claim and its commit when `Contributions::count` counts -- see
    // `lock_open_claims`. The cascade below deletes every claim and task, so
    // without them this deadlocked against a submission in flight just as
    // purge did. The merge lock first, as there: the cascade deletes a leave
    // job's staged and per-rack rows in an order of its own.
    crate::jobs::leave_gen::lock_merges(&mut tx, id).await?;
    crate::jobs::lock_job_dispatch(&mut tx, id).await?;
    lock_open_claims(&mut tx, id).await?;
    load_job_for_update(&mut tx, id).await?;

    // The census is what a restore is scoped against if this delete turns out
    // to be a mistake, so it is written before anything is removed.
    let census = job_census(&mut tx, id).await?;
    audit::log(
        &mut tx,
        "job.deleted",
        Some(admin_id),
        None,
        Some("job"),
        Some(id.to_string()),
        None,
    )
    .await?;
    audit::log_detail(
        &mut tx,
        "job.deleted.census",
        admin_id,
        "job",
        id.to_string(),
        None,
        census,
    )
    .await?;

    // Deleting the job cascades its tasks and their claims away, so what the
    // identities earned is counted first and given back last -- see
    // `Contributions`.
    let contributions = Contributions::count(&mut tx, id).await?;

    let deleted = sqlx::query("DELETE FROM jobs WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    if deleted.rows_affected() == 0 {
        return Err(AppError::not_found("no such job"));
    }
    // As a purge does, and before the give-back for the same reason: the
    // pools this job fed must refit, and marking them can wait out a fit.
    crate::ratings::mark_every_pool_for_refit(&mut tx).await?;
    contributions.give_back(&mut tx).await?;
    tx.commit().await?;
    hold.committed();
    // Nothing to push: the job is gone. Its open streams are ended; the pages
    // reconnect, are answered 404, and stop.
    crate::jobstats::forget(id);
    state.sse.close(id);
    // Tidiness only: a remembered answer for a job that no longer exists is
    // never asked for, but there is no reason to keep it.
    state.derived_ready.forget(id);
    state.templates.forget(id);
    state.finish_checks.forget(id);
    state.leave_merges.forget(id);
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Exports
// ---------------------------------------------------------------------------

#[derive(Serialize, sqlx::FromRow)]
struct ExportRow {
    id: Uuid,
    state: String,
    bytes: Option<i64>,
    sha256: Option<String>,
    row_count: Option<i64>,
    /// Set for a games or game-pairs job that captured positions, whose
    /// export has a second object holding them.
    positions_bytes: Option<i64>,
    positions_sha256: Option<String>,
    positions_row_count: Option<i64>,
    error: Option<String>,
    requested_at: chrono::DateTime<chrono::Utc>,
    completed_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Serialize)]
struct ExportDetail {
    #[serde(flatten)]
    export: ExportRow,
    /// Present once the export is ready: a presigned URL that fetches the
    /// object directly, so the bytes never pass through this process.
    #[serde(skip_serializing_if = "Option::is_none")]
    download_url: Option<String>,
    /// The same for the captured positions, when the export has them.
    #[serde(skip_serializing_if = "Option::is_none")]
    positions_download_url: Option<String>,
}

/// Build a completed job's results into one downloadable artifact.
///
/// Returns immediately with an id; the work runs on a spawned task and the
/// admin polls `GET`. Only completed jobs qualify — see `exports::start`.
async fn start_export(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    method: Method,
    headers: HeaderMap,
    jar: CookieJar,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    csrf::verify(&method, &headers, &jar)?;
    refuse_while_purging(&state, id)?;

    let job = crate::jobstats::load_job(&state.pool, id).await?;
    let export_id = crate::exports::start(&state, &job, admin.0.id).await?;

    let mut conn = state.pool.acquire().await?;
    audit::log(
        &mut conn,
        "job.export_started",
        Some(admin.0.id),
        None,
        Some("job"),
        Some(id.to_string()),
        Some(id),
    )
    .await?;

    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "id": export_id, "state": "running" })),
    ))
}

/// The newest export for a job, with a download URL once it is ready.
async fn get_export(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<ExportDetail>> {
    let mut export = sqlx::query_as::<_, ExportRow>(
        "SELECT id, state, bytes, sha256, row_count, positions_bytes, positions_sha256,
                positions_row_count, error, requested_at, completed_at
         FROM job_exports WHERE job_id = $1
         ORDER BY requested_at DESC LIMIT 1",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::not_found("this job has never been exported"))?;

    let (download_url, positions_download_url) =
        match crate::exports::newest_ready(&state.pool, id).await? {
            Some(ready) if ready.id == export.id => {
                let ttl = crate::exports::DOWNLOAD_URL_TTL;
                let results = state.artifacts.presigned_get(&ready.artifact_key, ttl).await?;
                let positions = match &ready.positions_artifact_key {
                    Some(key) => Some(state.artifacts.presigned_get(key, ttl).await?),
                    None => None,
                };
                (Some(results), positions)
            }
            // Built, and past the store's lifecycle: say so, rather than
            // `ready` with nothing to download.
            _ if export.state == "ready" => {
                export.state = "expired".to_string();
                (None, None)
            }
            _ => (None, None),
        };

    Ok(Json(ExportDetail { export, download_url, positions_download_url }))
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

/// Queues a build for every wordmap and rack info table the job needs.
///
/// Logged rather than returned on failure: a job that exists without its
/// derived files simply does not dispatch, which is visible at
/// `GET /api/admin/derived-data` and fixed by activating it again. Failing the
/// creation would leave the admin with no job and a rolled-back transaction
/// that had already written the generation-0 artifact.
async fn request_derived_data(state: &AppState, job_id: Uuid) -> AppResult<()> {
    let mut conn = state.pool.acquire().await?;
    match crate::derived::request_for_job(&mut conn, job_id, &state.builders).await {
        Ok(0) => {}
        Ok(n) => tracing::info!(%job_id, requested = n, "queued derived file builds"),
        Err(err) => tracing::error!(
            %job_id, error = %err.message,
            "could not queue this job's derived file builds; it will not dispatch until it can"
        ),
    }
    Ok(())
}

#[derive(Serialize, sqlx::FromRow)]
struct DerivedDataRow {
    role: String,
    name: String,
    builder: String,
    state: String,
    sha256: Option<String>,
    bytes: Option<i64>,
    build_target: Option<String>,
    error: Option<String>,
    attempts: i32,
    requested_at: chrono::DateTime<chrono::Utc>,
    built_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Every wordmap and rack info table the server has been asked to build.
///
/// The admin-facing half of the dispatch gate: a job that asks for a rack info
/// table is not handed out until this says `built`, and a build that failed is
/// the reason a job is quietly doing nothing. Without this page, "the job is
/// active and no worker is claiming from it" has no visible cause.
async fn list_derived_data(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> AppResult<Json<Vec<DerivedDataRow>>> {
    Ok(Json(
        sqlx::query_as::<_, DerivedDataRow>(
            "SELECT role, name, builder, state, sha256, bytes, build_target, error,
                    attempts, requested_at, built_at
             FROM derived_data
             ORDER BY state = 'built', requested_at DESC",
        )
        .fetch_all(&state.pool)
        .await?,
    ))
}

#[derive(Deserialize)]
struct RetryDerivedBody {
    role: String,
    name: String,
}

/// Puts a failed build back in the queue.
///
/// Explicit, because a build is a pure function of its inputs: one that failed
/// three times failed for a reason that a fourth attempt does not change, and
/// re-queueing it automatically would spend every builder run on the same
/// doomed row. An admin retries it after fixing what it named -- most often a
/// lexicon imported before the server stored its bytes.
async fn retry_derived_data(
    State(state): State<AppState>,
    admin: AdminUser,
    method: Method,
    headers: HeaderMap,
    jar: CookieJar,
    ApiJson(body): ApiJson<RetryDerivedBody>,
) -> AppResult<StatusCode> {
    csrf::verify(&method, &headers, &jar)?;
    let reset = sqlx::query(
        "UPDATE derived_data
         SET state = 'pending', attempts = 0, error = NULL, leased_until = NULL
         WHERE role = $1 AND name = $2 AND state = 'failed'",
    )
    .bind(&body.role)
    .bind(&body.name)
    .execute(&state.pool)
    .await?
    .rows_affected();
    if reset == 0 {
        return Err(AppError::not_found("no failed build for that role and name"));
    }
    let mut conn = state.pool.acquire().await?;
    audit::log(
        &mut conn,
        "derived_data.retried",
        Some(admin.0.id),
        None,
        Some("derived_data"),
        Some(format!("{} {}", body.role, body.name)),
        None,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct RebuildQuery {
    /// Rewrite an object whose bytes no longer hash to what was recorded.
    /// Off by default; see `leave_gen::rebuild_artifacts` for why a mismatch
    /// is not on its own a reason to overwrite.
    #[serde(default)]
    force: bool,
}

/// Fold a leave-generation job's staged results into its per-rack totals now,
/// rather than at the next half-hourly sweep.
///
/// Nothing needs this: claims ask for a merge themselves near a generation's
/// end and a transition drains before it reads. It is for an admin who wants
/// the page's "racks at target" to be current, and for the end-to-end suite,
/// which asserts on the merged rows. Waits for a merge already running, so the
/// answer describes a generation with nothing staged behind it.
async fn merge_leave_progress(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
    method: Method,
    headers: HeaderMap,
    jar: CookieJar,
) -> AppResult<Json<crate::jobs::leave_gen::MergeOutcome>> {
    csrf::verify(&method, &headers, &jar)?;
    refuse_while_purging(&state, id)?;
    let job = crate::jobstats::load_job(&state.pool, id).await?;
    if job.job_type != JobType::LeaveGeneration {
        return Err(AppError::bad_request("only a leave-generation job stages results"));
    }
    let outcome = crate::jobs::leave_gen::merge_staged_for_job(&state.pool, id, true).await?;
    // The point of the button is current rack figures.
    super::worker::push_after_change(&state, id);
    Ok(Json(outcome))
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
    refuse_while_purging(&state, id)?;

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
    // Forcing rewrites every generation's object, and a worker that claimed
    // a task a moment before fetches the new bytes against the old hash,
    // declines, and sets the job aside. Deactivate first.
    if query.force && job.status == JobStatus::Active {
        return Err(AppError::conflict(
            "deactivate the job before forcing a rebuild: workers mid-task would refuse the \
             rewritten objects",
        ));
    }
    let job_data = crate::jobs::load_job_data(&mut conn, job.id).await?;
    drop(conn);

    let report = crate::jobs::leave_gen::rebuild_artifacts(
        &state.pool,
        &state.artifacts,
        &state.magpie,
        &state.builders,
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

/// Account deletion anonymizes the account rather than removing it
/// Personal data goes: the username and email become
/// tombstones, the password becomes unusable, API keys, confirmation codes and
/// reset tokens are deleted, and every session is revoked. Contributions stay:
/// the account's claims and results are kept under the tombstone, and no
/// counter is rolled back, so no donated compute is lost -- including captured
/// positions other redundant claims deduplicated against. Open claims are left
/// to time out; nothing can submit for them once the keys are gone.
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

    let anonymized = sqlx::query(
        "UPDATE users SET
             username = 'deleted-' || id::text,
             -- Random, not the id: ids are public (`GET /api/users`), and
             -- `email` is unique, so anyone who registered `<id>@deleted.invalid`
             -- first made the account impossible to delete.
             email = gen_random_uuid()::text || '@deleted.invalid',
             password_hash = '!',
             email_confirmed_at = NULL,
             is_admin = false,
             session_generation = session_generation + 1,
             deleted_at = now()
         WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    if anonymized.rows_affected() == 0 {
        return Err(AppError::not_found("no such user"));
    }
    for table in ["api_keys", "email_confirmations", "password_reset_tokens"] {
        sqlx::query(&format!("DELETE FROM {table} WHERE user_id = $1"))
            .bind(id)
            .execute(&mut *tx)
            .await?;
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

#[derive(Serialize, sqlx::FromRow)]
struct BanRow {
    id: Uuid,
    user_id: Option<Uuid>,
    username: Option<String>,
    anon_uuid: Option<Uuid>,
    reason: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
}

/// Every ban in force, newest first -- what lifting one needs, since
/// `DELETE /workers/ban/:id` takes the ban's id and nothing else listed them:
/// a mistaken ban could be lifted only with SQL. One row per banned identity,
/// so this is bounded by the bans an admin has made.
async fn list_bans(State(state): State<AppState>, _admin: AdminUser) -> AppResult<Json<Vec<BanRow>>> {
    Ok(Json(
        sqlx::query_as::<_, BanRow>(
            "SELECT b.id, b.user_id, u.username, b.anon_uuid, b.reason, b.created_at
             FROM worker_bans b LEFT JOIN users u ON u.id = b.user_id
             ORDER BY b.created_at DESC",
        )
        .fetch_all(&state.pool)
        .await?,
    ))
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
    ApiJson(body): ApiJson<BanBody>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    csrf::verify(&method, &headers, &jar)?;

    if body.user_id.is_some() == body.anon_uuid.is_some() {
        return Err(AppError::bad_request("supply exactly one of user_id or anon_uuid"));
    }
    // Said plainly rather than left to the foreign key, whose refusal read
    // "that is still referenced by other records" -- the usual sign of an
    // anonymous UUID sent as a user id, or the other way round.
    let exists: bool = match (body.user_id, body.anon_uuid) {
        (Some(id), _) => sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")
            .bind(id)
            .fetch_one(&state.pool)
            .await?,
        (_, Some(uuid)) => {
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM anonymous_workers WHERE uuid = $1)")
                .bind(uuid)
                .fetch_one(&state.pool)
                .await?
        }
        (None, None) => false,
    };
    if !exists {
        return Err(AppError::not_found(if body.user_id.is_some() {
            "no account has that id; is it an anonymous worker's UUID?"
        } else {
            "no anonymous worker has that UUID; is it an account's id?"
        }));
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

    // Logged like the ban it lifts, in the same transaction, and naming the
    // identity rather than the ban row: a ban that was applied and then quietly
    // removed is exactly the sequence an audit log exists to make visible, and
    // the ban row is gone by the time anyone reads it.
    let mut tx = state.pool.begin().await?;
    let target: Option<(Option<Uuid>, Option<Uuid>)> =
        sqlx::query_as("DELETE FROM worker_bans WHERE id = $1 RETURNING user_id, anon_uuid")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some((user_id, anon_uuid)) = target else {
        return Err(AppError::not_found("no such ban"));
    };
    audit::log(
        &mut tx,
        "worker.unbanned",
        Some(admin.0.id),
        None,
        Some("worker"),
        Some(
            user_id
                .or(anon_uuid)
                .map(|id| id.to_string())
                .unwrap_or_default(),
        ),
        None,
    )
    .await?;
    tx.commit().await?;
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

fn status_name(status: JobStatus) -> &'static str {
    match status {
        JobStatus::Active => "active",
        JobStatus::Inactive => "inactive",
        JobStatus::Completed => "completed",
    }
}

async fn load_job_for_update(conn: &mut sqlx::PgConnection, id: Uuid) -> AppResult<Job> {
    sqlx::query_as::<_, Job>("SELECT * FROM jobs WHERE id = $1 FOR UPDATE")
        .bind(id)
        .fetch_optional(conn)
        .await?
        .ok_or_else(|| AppError::not_found("no such job"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(config: serde_json::Value) -> CreateJobBody {
        let mut value = serde_json::json!({
            "variant": "classic",
            "letterdist_id": Uuid::nil(),
            "layout_id": Uuid::nil(),
        });
        value.as_object_mut().unwrap().extend(config.as_object().unwrap().clone());
        serde_json::from_value(value).expect("a well-formed body")
    }

    fn game_pairs(overrides: serde_json::Value) -> CreateJobBody {
        let mut config = serde_json::json!({
            "job_type": "game_pairs",
            "player1_config_id": Uuid::nil(),
            "player2_config_id": Uuid::nil(),
            "min_pairs": 100,
            "max_pairs": 1000,
        });
        config.as_object_mut().unwrap().extend(overrides.as_object().unwrap().clone());
        body(config)
    }

    fn fields(result: AppResult<()>) -> Vec<String> {
        result.expect_err("should be rejected").fields.into_iter().map(|(f, _)| f).collect()
    }

    #[test]
    fn ordinary_settings_are_accepted() {
        assert!(validate_job_body(&game_pairs(serde_json::json!({}))).is_ok());
    }

    /// A batch of zero makes every claim regenerate the seed the last claim
    /// took; the job would retry a unique violation forever.
    #[test]
    fn a_zero_batch_is_rejected() {
        assert_eq!(
            fields(validate_job_body(&game_pairs(serde_json::json!({ "pairs_per_batch": 0 })))),
            ["pairs_per_batch"]
        );
    }

    /// Inverted hypotheses flip the LLR's sign: SPRT would accept the wrong one.
    #[test]
    fn inverted_elo_hypotheses_are_rejected() {
        assert_eq!(
            fields(validate_job_body(&game_pairs(
                serde_json::json!({ "elo_low": 10.0, "elo_high": -10.0 })
            ))),
            ["elo_high"]
        );
    }

    #[test]
    fn degenerate_error_rates_are_rejected_and_every_problem_is_reported() {
        let got = fields(validate_job_body(&game_pairs(serde_json::json!({
            "sprt_alpha": 0.0, "sprt_beta": 1.0, "max_pairs": 0, "redundancy": 0
        }))));
        for expected in ["sprt_alpha", "sprt_beta", "max_pairs", "redundancy"] {
            assert!(got.iter().any(|f| f == expected), "missing {expected} in {got:?}");
        }
    }

    #[test]
    fn leave_generation_bounds_are_enforced() {
        let leave = body(serde_json::json!({
            "job_type": "leave_generation",
            "kwg_id": Uuid::nil(),
            "num_iterations": 0,
            "target_rack_count": 10,
            "racks_per_task": 0,
        }));
        assert_eq!(fields(validate_job_body(&leave)), ["num_iterations", "racks_per_task"]);
    }

    #[test]
    fn leave_generation_runs_at_redundancy_one() {
        let mut leave = body(serde_json::json!({
            "job_type": "leave_generation",
            "kwg_id": Uuid::nil(),
            "num_iterations": 1,
            "target_rack_count": 10,
            "racks_per_task": 1,
        }));
        assert!(validate_job_body(&leave).is_ok());
        leave.redundancy = 2;
        assert_eq!(fields(validate_job_body(&leave)), ["redundancy"]);
    }

    #[test]
    fn an_unknown_variant_is_rejected() {
        let mut job = game_pairs(serde_json::json!({}));
        job.variant = "scrabble-but-different".into();
        assert_eq!(fields(validate_job_body(&job)), ["variant"]);
    }
}
