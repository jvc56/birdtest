use crate::audit;
use crate::auth::{csrf, AdminUser};
use crate::backups::{self, BackupStatus};
use crate::extract::ApiJson;
use crate::error::{AppError, AppResult};
use crate::jobs::leave_gen::RACK_SIZE;
use crate::jobs::registry;
use crate::models::job::{ConsensusSettings, Job, JobStatus, JobType, OpeningRackConfig, PlayerConfig};
use crate::state::AppState;
use crate::extract::{ApiPath as Path, ApiQuery as Query};
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode};
use axum::routing::{delete, get, patch, post, put};
use axum::{Json, Router};
use axum_extra::extract::CookieJar;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/player-configs", get(list_player_configs).post(create_player_config))
        .route("/player-configs/:id", get(get_player_config).delete(delete_player_config))
        .route("/jobs", post(create_job))
        .route("/jobs/allocations", put(set_allocations))
        .route("/jobs/:id/complete", post(complete_job))
        .route("/jobs/:id/consensus", patch(update_consensus))
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
        .route("/jobs/:id/derived-data", get(job_derived_data))
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
    /// A branch or tag of the data repository, resolved to a commit at import
    /// time so the record names a commit and never a moving branch.
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

/// Phase 1, in the background. The archive is ~190 MB, so the request returns an
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

    // Resolving the ref is one to six GitHub API calls (a branch, a tag, the
    // tag objects it peels) and its failure modes are worth reporting
    // synchronously -- a typo'd ref should not become a failed background task.
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

    if import.state == "nothing_new" {
        return Err(AppError::conflict("this import has nothing new to insert"));
    }
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

/// The wordmaps and rack info tables a job needs and how their builds stand.
/// A job is not dispatched until every one is built, so this is what an
/// active job that nothing claims from is waiting for.
async fn job_derived_data(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Vec<crate::derived::JobDerivedFile>>> {
    let mut conn = state.pool.acquire().await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM jobs WHERE id = $1)")
        .bind(id)
        .fetch_one(&mut *conn)
        .await?;
    if !exists {
        return Err(AppError::not_found("no such job"));
    }
    Ok(Json(crate::derived::files_for_job(&mut conn, id, &state.builders).await?))
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
///
/// The week's completed claims and the claims open now, each through its own
/// partial index (`task_claims_completed_idx`, `task_claims_open_idx`), so the
/// read is a week of the fleet's work whatever the table's age. It was every
/// claim claimed in the week, which no index serves (KL-32): a sequential scan
/// of every claim ever made, on the main pool with no timeout. A claim that
/// lapsed or was declined is not counted. On the display pool, for its
/// statement timeout and to keep the read off the connections claims use.
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
             FROM (SELECT magpie_version, claimed_by_user_id, claimed_by_anon_uuid
                   FROM task_claims
                   WHERE state = 'completed' AND completed_at > now() - interval '7 days'
                   UNION ALL
                   SELECT magpie_version, claimed_by_user_id, claimed_by_anon_uuid
                   FROM task_claims
                   WHERE state = 'claimed' AND claimed_at > now() - interval '7 days') c
             GROUP BY magpie_version
             ORDER BY workers DESC",
        )
        .fetch_all(&state.read_pool)
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
    use_wit: Option<bool>,
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
    /// Endgame and pre-endgame solving, for games and game-pairs jobs. Off
    /// unless `endgame_plies` is above 0, which PEG needs too; see
    /// [`resolve_solver_settings`].
    #[serde(default)]
    endgame_plies: Option<i32>,
    #[serde(default)]
    peg_max_bag: Option<i32>,
    #[serde(default)]
    peg_stage_top_k: Option<Vec<i32>>,
    #[serde(default)]
    peg_scenario_stride: Option<i32>,
    #[serde(default)]
    peg_opp_model: Option<String>,
    #[serde(default)]
    peg_nested: Option<bool>,
    #[serde(default)]
    peg_nested_cand_caps: Option<Vec<i32>>,
    #[serde(default)]
    peg_nested_max_depth: Option<i32>,
    #[serde(default)]
    peg_nested_strides: Option<Vec<i32>>,
}

/// A player config's endgame and pre-endgame settings as the row stores them.
#[derive(Debug, Default, PartialEq)]
struct SolverSettings {
    endgame_plies: i32,
    peg_max_bag: i32,
    peg_stage_top_k: Option<Vec<i32>>,
    peg_scenario_stride: Option<i32>,
    peg_opp_model: Option<String>,
    peg_nested: Option<bool>,
    peg_nested_cand_caps: Option<Vec<i32>>,
    peg_nested_max_depth: Option<i32>,
    peg_nested_strides: Option<Vec<i32>>,
}

/// MAGPIE's endgame depth ceiling (`MAX_VARIANT_LENGTH`) and largest
/// pre-endgame bag (`PEG_MAX_BAG`).
const MAGPIE_MAX_ENDGAME_PLIES: i32 = 25;
const MAGPIE_PEG_MAX_BAG: i32 = 4;
/// The most stages a PEG schedule, or levels a nested cap list, may have
/// (`AUTOPLAY_SOLVER_MAX_PEG_STAGES`, `AUTOPLAY_SOLVER_MAX_NESTED_CAND_CAPS`).
const MAGPIE_MAX_PEG_STAGES: usize = 16;

/// Resolves and checks a body's endgame and pre-endgame settings.
///
/// They nest, as MAGPIE reads them: `endgame_plies` 0 solves nothing, and PEG
/// is refused without it, since PEG scores its emptier scenarios with endgame
/// solves. A PEG setting without `peg_max_bag` above 0 is refused, as is a
/// nested setting without nested lookahead -- the same rule simulation
/// settings follow without plies: a setting nothing reads would read as one
/// the player uses. Settings left out take MAGPIE's defaults
/// ([`crate::magpie_defaults`]), written into the row like every other.
fn resolve_solver_settings(body: &CreatePlayerConfigBody) -> AppResult<SolverSettings> {
    use crate::magpie_defaults as defaults;
    let mut err = AppError::bad_request("player config is invalid");
    let plies = body.endgame_plies.unwrap_or(0);
    let max_bag = body.peg_max_bag.unwrap_or(0);
    if !(0..=MAGPIE_MAX_ENDGAME_PLIES).contains(&plies) {
        err = err.with_field(
            "endgame_plies",
            format!("must be between 0 (off) and {MAGPIE_MAX_ENDGAME_PLIES}"),
        );
    }
    if !(0..=MAGPIE_PEG_MAX_BAG).contains(&max_bag) {
        err = err
            .with_field("peg_max_bag", format!("must be between 0 (off) and {MAGPIE_PEG_MAX_BAG}"));
    } else if max_bag > 0 && plies == 0 {
        err = err.with_field(
            "peg_max_bag",
            "the pre-endgame needs endgame solving: set an endgame depth (endgame_plies) as well",
        );
    }
    let peg_stated = [
        ("peg_stage_top_k", body.peg_stage_top_k.is_some()),
        ("peg_scenario_stride", body.peg_scenario_stride.is_some()),
        ("peg_opp_model", body.peg_opp_model.is_some()),
        ("peg_nested", body.peg_nested.is_some()),
        ("peg_nested_cand_caps", body.peg_nested_cand_caps.is_some()),
        ("peg_nested_max_depth", body.peg_nested_max_depth.is_some()),
        ("peg_nested_strides", body.peg_nested_strides.is_some()),
    ];
    if max_bag <= 0 {
        for (field, stated) in peg_stated {
            if stated {
                err = err.with_field(
                    field,
                    "is a pre-endgame setting: set peg_max_bag above 0 to run the pre-endgame, \
                     or leave it out",
                );
            }
        }
        return if err.fields.is_empty() {
            Ok(SolverSettings { endgame_plies: plies, peg_max_bag: 0, ..Default::default() })
        } else {
            Err(err)
        };
    }

    // MAGPIE's own check is 1 to 16 stages of 2 to `i32::MAX` plays; the
    // order rule is birdtest's. The one stage `[2147483647]` is MAGPIE's
    // exhaustive mode (`-pegtopk all`), accepted: it ranks every play at 40
    // plies, which plausibility allows a pre-endgame.
    let top_k = body.peg_stage_top_k.clone().unwrap_or_else(|| defaults::PEG_STAGE_TOP_K.to_vec());
    if top_k.is_empty() || top_k.len() > MAGPIE_MAX_PEG_STAGES {
        err = err.with_field(
            "peg_stage_top_k",
            format!("needs 1 to {MAGPIE_MAX_PEG_STAGES} stages"),
        );
    } else if top_k.iter().any(|&k| k < 2) || top_k.windows(2).any(|w| w[1] > w[0]) {
        err = err.with_field(
            "peg_stage_top_k",
            "each stage keeps at least 2 plays, and no more than the stage before it",
        );
    }
    let stride = body.peg_scenario_stride.unwrap_or(defaults::PEG_SCENARIO_STRIDE);
    if stride < 1 {
        err = err.with_field("peg_scenario_stride", "must be at least 1 (1 is full enumeration)");
    }
    let opp_model = body.peg_opp_model.clone().unwrap_or_else(|| defaults::PEG_OPP_MODEL.into());
    if !matches!(opp_model.as_str(), "rational" | "pessimistic") {
        err = err.with_field("peg_opp_model", "must be 'rational' or 'pessimistic'");
    }
    let nested = body.peg_nested.unwrap_or(defaults::PEG_NESTED);
    let mut settings = SolverSettings {
        endgame_plies: plies,
        peg_max_bag: max_bag,
        peg_stage_top_k: Some(top_k),
        peg_scenario_stride: Some(stride),
        peg_opp_model: Some(opp_model),
        peg_nested: Some(nested),
        ..Default::default()
    };
    if !nested {
        for (field, stated) in &peg_stated[4..] {
            if *stated {
                err = err.with_field(
                    *field,
                    "is a nested-lookahead setting: set peg_nested to true, or leave it out",
                );
            }
        }
    } else {
        let caps = body
            .peg_nested_cand_caps
            .clone()
            .unwrap_or_else(|| defaults::PEG_NESTED_CAND_CAPS.to_vec());
        if caps.is_empty() || caps.len() > MAGPIE_MAX_PEG_STAGES || caps.iter().any(|&c| c < 1) {
            err = err.with_field(
                "peg_nested_cand_caps",
                format!("needs 1 to {MAGPIE_MAX_PEG_STAGES} caps, each at least 1"),
            );
        }
        let depth = body.peg_nested_max_depth.unwrap_or(defaults::PEG_NESTED_MAX_DEPTH);
        if !(1..=MAGPIE_PEG_MAX_BAG).contains(&depth) {
            err = err.with_field(
                "peg_nested_max_depth",
                format!("must be between 1 and {MAGPIE_PEG_MAX_BAG}"),
            );
        }
        let strides = body
            .peg_nested_strides
            .clone()
            .unwrap_or_else(|| defaults::PEG_NESTED_STRIDES.to_vec());
        if strides.len() != MAGPIE_PEG_MAX_BAG as usize || strides.iter().any(|&s| s < 1) {
            err = err.with_field(
                "peg_nested_strides",
                format!(
                    "needs one stride per inner bag size 1 to {MAGPIE_PEG_MAX_BAG}, each at least 1"
                ),
            );
        }
        settings.peg_nested_cand_caps = Some(caps);
        settings.peg_nested_max_depth = Some(depth);
        settings.peg_nested_strides = Some(strides);
    }
    if err.fields.is_empty() {
        Ok(settings)
    } else {
        Err(err)
    }
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
    let solver = resolve_solver_settings(&body)?;

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
        // The other way to a simmer that never simulates: autoplay's move
        // list holds `num_plays` plays, and with one MAGPIE plays it without
        // simulating (`get_top_simming_move`), every turn -- a static player
        // pinned, rated and labelled as a simmer.
        if body.num_plays.unwrap_or(crate::magpie_defaults::NUM_PLAYS) < 2 {
            err = err.with_field(
                "num_plays",
                "a simming player needs at least 2 candidate plays: with one, MAGPIE plays it \
                 without simulating",
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
              utility_spread_scale, movegen_margin, created_by,
              endgame_plies, peg_max_bag, peg_stage_top_k, peg_scenario_stride,
              peg_opp_model, peg_nested, peg_nested_cand_caps, peg_nested_max_depth,
              peg_nested_strides, use_wit)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,
                 $18,$19,$20,$21,$22,$23,$24,$25,$26,$27,$28,$29,$30,$31,$32,$33,$34,$35,$36)
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
    // On unless the config says otherwise, as the form has it: a wordmap is a
    // large speedup in move generation, and workers build one on demand.
    .bind(body.use_wordmap.unwrap_or(true))
    // On unless the config says otherwise, as the form has it: a rack info
    // table speeds up move generation further still. It is large -- about
    // 1.9 GB on a contributor's disk and in its memory -- and the server
    // builds it before any job using it dispatches, once per (lexicon,
    // leaves) pair.
    .bind(body.use_rit.unwrap_or(true))
    .bind(min_play_iterations)
    .bind(&threshold)
    .bind(&sampling_rule)
    .bind(inference_margin)
    .bind(utility_w_winpct)
    .bind(utility_w_spread)
    .bind(utility_spread_scale)
    .bind(movegen_margin)
    .bind(admin.0.id)
    .bind(solver.endgame_plies)
    .bind(solver.peg_max_bag)
    .bind(&solver.peg_stage_top_k)
    .bind(solver.peg_scenario_stride)
    .bind(&solver.peg_opp_model)
    .bind(solver.peg_nested)
    .bind(&solver.peg_nested_cand_caps)
    .bind(solver.peg_nested_max_depth)
    .bind(&solver.peg_nested_strides)
    // On unless the config opts out, although MAGPIE has it opt-in (-wit):
    // it speeds move generation, and costs little. The server builds the
    // table before any job using it dispatches, once per lexicon: about three
    // seconds and 122 MB for CSW24.
    .bind(body.use_wit.unwrap_or(true))
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

/// The most blanks a distribution may have for MAGPIE to build a wordmap
/// (`wmp_maker.c`), and with it a rack info table.
const MAGPIE_MAX_WORDMAP_BLANKS: u32 = 2;

/// The most racks one leave task forces: 200 times the form's default of 50,
/// about 80 KB of racks in each claim.
const MAX_RACKS_PER_TASK: i32 = 10_000;

/// The most generations a leave job runs. MAGPIE's own runs are a handful;
/// each generation is a full pass over the rack universe and a KLV build.
const MAX_LEAVE_GENERATIONS: usize = 100;
/// The highest occurrence target a generation may set. A million per rack is
/// some three trillion forced racks for English's 3.2 million -- no run gets
/// there, so a larger number is a typo, not a plan.
const MAX_TARGET_RACK_COUNT: i32 = 1_000_000;

/// Numbers MAGPIE would refuse, or silently read as "use your own default".
///
/// MAGPIE validates these too, but only on a contributor's machine, after a
/// job has been built on the config and dispatched: every worker would fail
/// the task, and the job would sit there producing nothing. Refusing at
/// creation puts the error in front of the admin who can fix it.
const MAGPIE_MAX_PLIES: i32 = 25;
const MAGPIE_MAX_CAPTURED_PLIES: i32 = 10;
/// MAGPIE's largest equity (`EQUITY_MAX_DOUBLE`, `-(INT32_MIN + 3) / 1000`),
/// which it accepts as a margin; above it every worker refuses the task,
/// "server sent an invalid movegen_margin".
const MAGPIE_MAX_MARGIN: f64 = 2_147_483.645;
/// Plays generated. MAGPIE allocates `num_plays + 1` moves per player per
/// thread before it plays — 2e9 was a 16 GB allocation and a core dump on
/// every worker — but a static player ranking every opening play needs more
/// than a rack has: 63,585 for ??EIRST in CSW24. 200,000 is some 11 MB a
/// player a thread.
const MAX_NUM_PLAYS: i32 = 200_000;
/// Plays recorded: each is stored with its rank as a `SMALLINT`.
const MAX_NUM_PLAYS_RECORDED: i32 = i16::MAX as i32;
/// Why MAGPIE would refuse this board layout; see [`crate::board::BoardLayout::parse`].
pub(crate) fn layout_problem(content: &[u8]) -> Option<String> {
    crate::board::layout_problem(content)
}

fn validate_player_config_body(body: &CreatePlayerConfigBody) -> AppResult<()> {
    let mut err = AppError::bad_request("player config is invalid");
    if body.name.trim().is_empty() {
        err = err.with_field("name", "must not be empty");
    } else if let Some(problem) = name_problem(body.name.trim()) {
        err = err.with_field("name", problem);
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
    for (field, value, most) in [
        ("num_plays", body.num_plays, MAX_NUM_PLAYS),
        ("num_plays_recorded", Some(body.num_plays_recorded), MAX_NUM_PLAYS_RECORDED),
    ] {
        if value.is_some_and(|v| v > most) {
            err = err.with_field(field, format!("must be at most {most}"));
        }
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
    for (field, value) in [("inference_margin", body.inference_margin), ("movegen_margin", body.movegen_margin)] {
        if value.is_some_and(|v| v > MAGPIE_MAX_MARGIN) {
            err = err.with_field(field, format!("must be at most {MAGPIE_MAX_MARGIN}"));
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
    // `derived` and README.md's "MAGPIE on the server".
    if err.fields.is_empty() {
        Ok(())
    } else {
        Err(err)
    }
}

/// Player configs are immutable, so there is no update endpoint; deletion is
/// only allowed while nothing references the config.
///
/// Left unindexed on purpose: the request tables, `player_config_ratings` and
/// `rating_run_residuals` have no index leading with the config, so the check
/// below and the delete's foreign-key probes scan them. A delete is a rare
/// admin act on an unused config, and an index would cost an entry on every
/// request row and every fit (thirty-third audit, pass 1).
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
             UNION ALL SELECT 1 FROM job_leave_config WHERE player_config_id = $1
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
    /// What to call the job: shown first wherever jobs are listed. Optional
    /// here, for scripts; the creation form asks for it.
    #[serde(default)]
    name: Option<String>,
    job_type: JobType,
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
    /// Run-wide rules every request states. MAGPIE's defaults
    /// ([`crate::magpie_defaults`]) where the body leaves them out, written
    /// into the row like every other setting. A leave job states no cutoff:
    /// its bot never simulates.
    #[serde(default)]
    bingo_bonus: Option<i32>,
    #[serde(default)]
    sim_cutoff: Option<f64>,
    #[serde(flatten)]
    config: JobTypeConfig,
}

fn one() -> i32 {
    1
}

/// A games job's default batch: even, so each task gives each player the
/// first move equally (see [`validate_job_body`]).
fn two() -> i32 {
    2
}

/// Per-job-type configuration, expanded into typed columns rather than stored
/// as JSON.
///
/// Untagged, so the first variant a body fits is the one it becomes, and extra
/// fields are ignored: `Leave` comes first because a leave body, which names a
/// `player_config_id` too, would otherwise fit `OpeningRack` on its defaults.
/// No other body fits `Leave`, whose settings have no defaults.
#[derive(Deserialize)]
#[serde(untagged)]
enum JobTypeConfig {
    Leave {
        /// The player the leave-generating bot plays as, in both seats: its
        /// lexicon and wordmap setting are the job's. Held to static equity
        /// play without a rack info table (see [`validate_leave_player`]).
        player_config_id: Uuid,
        num_iterations: i32,
        /// One occurrence target per generation, in order; its length is how
        /// many generations the job runs.
        target_rack_counts: Vec<i32>,
        racks_per_task: i32,
    },
    OpeningRack {
        player_config_id: Uuid,
        #[serde(default = "default_racks_per_batch")]
        racks_per_batch: i32,
        #[serde(default = "default_rack_size")]
        rack_size: i32,
        /// Consensus: a rack is analysed until at least `min_results_per_rack`
        /// analyses agree on its best move in this share (percent), or until
        /// it has `max_results_per_rack`. One analysis per rack by default.
        #[serde(default = "default_consensus_pct")]
        consensus_pct: f64,
        #[serde(default = "one")]
        min_results_per_rack: i32,
        #[serde(default = "one")]
        max_results_per_rack: i32,
    },
    Game {
        /// The configs to play each other: one is a self-play job, and n ≥ 2
        /// a round robin of C(n, 2) jobs, one per pairing (see
        /// [`pairings`]).
        player_config_ids: Vec<Uuid>,
        #[serde(default = "two")]
        games_per_batch: i32,
        /// With the test off, the job plays this many games and stops; with it
        /// on, it stops here at the latest.
        max_games: i32,
        /// The fewest games before the test is acted on: a setting of the
        /// match test, required with one and refused without.
        #[serde(default)]
        min_games: Option<i32>,
        #[serde(flatten)]
        test: TestRequest,
        #[serde(default)]
        capture_positions: bool,
        /// Refused: a games job has no pairs to diverge. Read so that it is
        /// refused rather than ignored by this untagged body.
        #[serde(default)]
        capture_first_divergence: bool,
        /// `igp` or `pgp`: see [`crate::jobs::handler::GameRequest::threading_mode`].
        #[serde(default = "default_threading_mode")]
        threading_mode: String,
    },
    GamePair {
        /// As for `Game`: one config is self-play, n ≥ 2 a round robin.
        player_config_ids: Vec<Uuid>,
        #[serde(default = "one")]
        pairs_per_batch: i32,
        /// With the test off, the job plays this many pairs and stops; with it
        /// on, it stops here at the latest.
        max_pairs: i32,
        /// The fewest pairs before the test is acted on: a setting of the
        /// match test, required with one and refused without.
        #[serde(default)]
        min_pairs: Option<i32>,
        #[serde(flatten)]
        test: TestRequest,
        #[serde(default)]
        capture_positions: bool,
        /// Of the captured positions, keep only each pair's first divergence.
        #[serde(default)]
        capture_first_divergence: bool,
        /// As for `Game`.
        #[serde(default = "default_threading_mode")]
        threading_mode: String,
    },
}

/// A games or pairs job's match test (`stats::match_test`), as the request
/// states it.
///
/// Off unless `test_enabled` says otherwise, and then its settings are
/// refused (see [`validate_job_body`]). On, a confidence left out is 95%.
#[derive(Deserialize)]
struct TestRequest {
    #[serde(default)]
    test_enabled: bool,
    #[serde(default)]
    confidence_pct: Option<f64>,
}

/// The test as a config row stores it. A job without one stores the default
/// confidence and a floor of 0, which nothing reads: `TestParams::enabled`
/// gates them all.
struct TestSettings {
    enabled: bool,
    min_units: i32,
    confidence_pct: f64,
}

impl TestRequest {
    fn settings(&self, min_units: Option<i32>) -> TestSettings {
        TestSettings {
            enabled: self.test_enabled,
            min_units: min_units.unwrap_or(0),
            confidence_pct: self.confidence_pct.unwrap_or(95.0),
        }
    }
}

/// IGP: all of a task's threads on one game's simulation at a time, which is
/// what makes a simulation bounded by iterations reproducible. PGP, a game a
/// thread, is the job's to ask for.
fn default_threading_mode() -> String {
    "igp".to_string()
}
fn default_consensus_pct() -> f64 {
    100.0
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
struct CreatedJobs {
    /// Every job the request created, as stored: one, or for a games or
    /// pairs request naming n ≥ 2 configs, one per pairing in the order of
    /// [`pairings`].
    jobs: Vec<Job>,
}

/// One job a request makes: its name and, for games and pairs, its seating.
struct PlannedJob {
    name: String,
    pair: Option<(Uuid, Uuid)>,
    /// "A vs B", for an error to name the pairing it is about; `None` for a
    /// request that makes one job, whose errors need no qualifying.
    pairing: Option<String>,
}

impl PlannedJob {
    /// `err` about this job, named for its pairing when it has one -- in a
    /// round robin, "player configs disagree on the win% model" alone does
    /// not say which two.
    fn qualify(&self, mut err: AppError) -> AppError {
        if let Some(pairing) = &self.pairing {
            if err.status.is_client_error() {
                err.message = format!("{pairing}: {}", err.message);
            }
        }
        err
    }
}

/// The jobs `body` asks for: one, or for a games or pairs request naming
/// n ≥ 2 configs, one per pairing, named "{name}: {A} vs {B}" (or "A vs B"
/// with no name), A and B the configs' names -- two configs too, so a job's
/// name says who plays whom however many were asked for. A self-play job (one
/// config) and every other type keep the name as given.
async fn plan_jobs(conn: &mut sqlx::PgConnection, body: &CreateJobBody) -> AppResult<Vec<PlannedJob>> {
    let name = job_name(body);
    let pairs = pairings(&body.config);
    let one_job = match pairs.as_slice() {
        [] => true,
        [(a, b)] => a == b,
        _ => false,
    };
    if one_job {
        return Ok(vec![PlannedJob { name, pair: pairs.first().copied(), pairing: None }]);
    }
    let ids: Vec<Uuid> = pairs.iter().flat_map(|(a, b)| [*a, *b]).collect();
    let names: std::collections::HashMap<Uuid, String> =
        sqlx::query_as::<_, (Uuid, String)>("SELECT id, name FROM player_configs WHERE id = ANY($1)")
            .bind(&ids)
            .fetch_all(&mut *conn)
            .await?
            .into_iter()
            .collect();
    if let Some(missing) = ids.iter().find(|id| !names.contains_key(id)) {
        return Err(AppError::bad_request("player config not found")
            .with_field("player_config_ids", format!("no such player config: {missing}")));
    }
    let mut planned = Vec::with_capacity(pairs.len());
    for (a, b) in pairs {
        let pairing = format!("{} vs {}", names[&a], names[&b]);
        let full = if name.is_empty() { pairing.clone() } else { format!("{name}: {pairing}") };
        if let Some(problem) = name_problem(&full) {
            return Err(AppError::bad_request("job settings are invalid").with_field(
                "name",
                format!("named for its pairing, {full:?} breaks a job name's rule ({problem}): shorten the name"),
            ));
        }
        planned.push(PlannedJob { name: full, pair: Some((a, b)), pairing: Some(pairing) });
    }
    Ok(planned)
}

/// Jobs are always created inactive at 0%. The allocation is set later, on
/// the allocation page, so the admin sets it while looking at the whole
/// active set -- for a round robin, the whole set of pairings at once.
///
/// A games or pairs request naming n ≥ 2 configs creates every pairing's job
/// or none: each pairing is checked before anything is inserted (its players
/// must agree on what MAGPIE cannot vary per player), and all of them are
/// inserted in one transaction, so a check that only the insert can make (two
/// files under one name, a wordmap for too many blanks) refuses the lot.
async fn create_job(
    State(state): State<AppState>,
    admin: AdminUser,
    method: Method,
    headers: HeaderMap,
    jar: CookieJar,
    ApiJson(body): ApiJson<CreateJobBody>,
) -> AppResult<(StatusCode, Json<CreatedJobs>)> {
    csrf::verify(&method, &headers, &jar)?;

    validate_job_body(&body)?;

    let letterdist_name = require_role(&state.pool, body.letterdist_id, "letterdist").await?;
    require_role(&state.pool, body.layout_id, "layout").await?;
    let layout: Vec<u8> = sqlx::query_scalar("SELECT content FROM input_data WHERE id = $1")
        .bind(body.layout_id)
        .fetch_one(&state.pool)
        .await?;
    if let Some(problem) = layout_problem(&layout) {
        return Err(AppError::bad_request(
            "the board layout cannot be used: every worker would fail every task",
        )
        .with_field("layout_id", problem));
    }
    // Parsed now as every claim will parse it: a file the server or MAGPIE
    // cannot use -- more letters than MAGPIE holds, a malformed row -- was
    // found by the first claim, as a 500, on a job already created.
    let content: Vec<u8> = sqlx::query_scalar("SELECT content FROM input_data WHERE id = $1")
        .bind(body.letterdist_id)
        .fetch_one(&state.pool)
        .await?;
    let distribution = crate::jobs::racks::LetterDistribution::parse(&content, &letterdist_name)
        .map_err(|e| {
            AppError::bad_request("the letter distribution cannot be used")
                .with_field("letterdist_id", e.message)
        })?;
    let blanks = distribution.tiles.iter().find(|t| t.letter == '?').map_or(0, |t| t.count);

    // Defaulted from config rather than typed, so the form shows the effective
    // value; a typed one was checked above.
    let floor = crate::version::Version::parse_or_zero(
        body.min_magpie_version
            .as_deref()
            .unwrap_or(&state.cfg.min_magpie_version),
    );

    let planned = {
        let mut conn = state.pool.acquire().await?;
        let planned = plan_jobs(&mut conn, &body).await?;
        let capture = matches!(
            body.config,
            JobTypeConfig::Game { capture_positions: true, .. }
                | JobTypeConfig::GamePair { capture_positions: true, .. }
        );
        for plan in &planned {
            if let Some(pair) = plan.pair {
                validate_pairing(&mut conn, pair, capture, &letterdist_name)
                    .await
                    .map_err(|e| plan.qualify(e))?;
            }
        }
        planned
    };

    let mut tx = state.pool.begin().await?;
    let mut jobs = Vec::with_capacity(planned.len());
    for plan in &planned {
        let job = insert_job(&mut tx, &body, &admin, floor, plan, &letterdist_name, blanks)
            .await
            .map_err(|e| plan.qualify(e))?;
        jobs.push(job);
    }
    tx.commit().await?;

    for job in &jobs {
        // The zeroed KLV generation 1 starts from (stored as generation 0): a
        // multi-megabyte build and an object-store
        // write, so it happens after the transaction commits rather than inside it.
        registry::initialize_job_artifacts(&state, job).await?;

        // Queued at creation rather than at activation: a rack info table takes
        // minutes to build, and the admin who creates a job typically activates it
        // in the next breath. Requesting it now means the wait happens while they
        // are still deciding rather than after.
        request_derived_data(&state, job.id).await?;
    }

    Ok((StatusCode::CREATED, Json(CreatedJobs { jobs })))
}

/// One planned job's row, config and audit entry, in the creating
/// transaction.
#[allow(clippy::too_many_arguments)]
async fn insert_job(
    tx: &mut sqlx::PgConnection,
    body: &CreateJobBody,
    admin: &AdminUser,
    floor: crate::version::Version,
    plan: &PlannedJob,
    letterdist_name: &str,
    blanks: u32,
) -> AppResult<Job> {
    let job = sqlx::query_as::<_, Job>(
        "INSERT INTO jobs
             (job_type, variant, letterdist_id, layout_id,
              min_magpie_major, min_magpie_minor, min_magpie_patch, bingo_bonus,
              sim_cutoff, created_by, name, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, clock_timestamp()) RETURNING *",
    )
    // `clock_timestamp()`, not the column's `now()`: a round robin's jobs are
    // inserted in one transaction, where `now()` is one instant, and every
    // list that orders by creation then showed its pairings in id order.
    .bind(body.job_type)
    .bind(&body.variant)
    .bind(body.letterdist_id)
    .bind(body.layout_id)
    .bind(floor.major)
    .bind(floor.minor)
    .bind(floor.patch)
    // Written from MAGPIE's defaults, like a player config's settings, so
    // every request states them rather than each worker's build supplying
    // its own.
    .bind(body.bingo_bonus.unwrap_or(crate::magpie_defaults::BINGO_BONUS))
    // Stored for a leave job too, as the column requires; its requests never
    // carry it.
    .bind(body.sim_cutoff.unwrap_or(crate::magpie_defaults::SIM_CUTOFF))
    .bind(admin.0.id)
    .bind(&plan.name)
    .fetch_one(&mut *tx)
    .await?;

    insert_job_config(&mut *tx, &job, &body.config, plan.pair, letterdist_name).await?;
    refuse_one_name_for_two_files(&mut *tx, &job).await?;
    // MAGPIE builds a wordmap, and so a rack info table, for at most two
    // blanks (`cannot create WMP with more than 2 blanks`, an abort): a job
    // that needs either on `english_super` was created, its build failed
    // three times, and it never dispatched, with nothing on its page to say
    // why (the audit's pass 7). A word info table is built from the `.kwg`
    // alone, which has no blanks, so it is no reason to refuse.
    if blanks > MAGPIE_MAX_WORDMAP_BLANKS
        && crate::derived::needs_for_job(&mut *tx, job.id).await?.iter().any(|need| need.role != "wit")
    {
        return Err(AppError::bad_request(
            "no wordmap or rack info table can be built for this letter distribution",
        )
        .with_field(
            "letterdist_id",
            format!(
                "{letterdist_name} has {blanks} blanks, and MAGPIE builds them for at most \
                 {MAGPIE_MAX_WORDMAP_BLANKS}: no player of this job may use either"
            ),
        ));
    }

    audit::log(
        &mut *tx,
        "job.created",
        Some(admin.0.id),
        None,
        Some("job"),
        Some(job.id.to_string()),
        Some(job.id),
    )
    .await?;
    Ok(job)
}

/// The largest opening-rack batch accepted. A task's racks are expanded into
/// its request and every rack comes back analysed in one submission, so this
/// bounds both; 500 is the default.
const MAX_RACKS_PER_BATCH: i32 = 10_000;
/// The most analyses an opening-rack consensus may ask of one rack.
const MAX_RESULTS_PER_RACK: i32 = 100;

/// The most games one task may play (a pair counts two), and the most when
/// the job captures positions. A captured position's game is an `i16`, so a
/// batch past 32,768 games could never be submitted -- every result refused,
/// the job stuck -- and about 4,000 captured games already pass the 64 MiB
/// body limit (the audit's pass 21).
const MAX_GAMES_PER_BATCH: i32 = 10_000;
const MAX_CAPTURED_GAMES_PER_BATCH: i32 = 1_000;

/// The batch-size bound for a games or game-pairs job, in its own unit.
fn games_batch_field(mut err: AppError, unit: &str, games_per_unit: i32, batch: i32, capture: bool) -> AppError {
    let cap = if capture { MAX_CAPTURED_GAMES_PER_BATCH } else { MAX_GAMES_PER_BATCH } / games_per_unit;
    if batch > cap {
        let when = if capture { " when capturing positions" } else { "" };
        err = err.with_field(format!("{unit}s_per_batch"), format!("must be at most {cap}{when}"));
    }
    err
}

/// The longest name a job, player config or rating pool takes, in characters
/// (each column's check).
const MAX_NAME_CHARS: usize = 100;

/// What is wrong with a name as stored (trimmed), if anything. Shown in every
/// list that names it, in job settings and as a page title: a line, not a
/// document, and nothing that breaks or hides in one. A player config's and a
/// pool's name took any text until the thirty-third audit (pass 1).
pub(crate) fn name_problem(name: &str) -> Option<String> {
    if name.chars().count() > MAX_NAME_CHARS {
        Some(format!("at most {MAX_NAME_CHARS} characters"))
    } else if name.chars().any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}')) {
        Some("one line, with no control characters".into())
    } else {
        None
    }
}

/// The job's name as stored: trimmed, empty when none was given.
fn job_name(body: &CreateJobBody) -> String {
    body.name.as_deref().unwrap_or("").trim().to_string()
}

/// An opening-rack job's consensus settings, checked alike at creation and
/// when an admin changes them (`update_consensus`): each problem is a field
/// error added to `err`.
fn consensus_problems(mut err: AppError, consensus_pct: f64, min: i32, max: i32) -> AppError {
    // Above half: at or below it two moves could each hold the consensus, and
    // a tie would settle a rack on whichever sorted first.
    if !(consensus_pct.is_finite() && consensus_pct > 50.0 && consensus_pct <= 100.0) {
        err = err.with_field("consensus_pct", "must be above 50 and at most 100");
    }
    if !(1..=MAX_RESULTS_PER_RACK).contains(&min) {
        err = err.with_field(
            "min_results_per_rack",
            format!("must be between 1 and {MAX_RESULTS_PER_RACK}"),
        );
    } else if !(min..=MAX_RESULTS_PER_RACK).contains(&max) {
        err = err.with_field(
            "max_results_per_rack",
            format!("must be between min_results_per_rack ({min}) and {MAX_RESULTS_PER_RACK}"),
        );
    }
    err
}

/// The largest bingo bonus a job may set; real variants use 0 to 50.
const MAX_BINGO_BONUS: i32 = 500;

/// Settings the schema cannot express and no worker or test could run with.
///
/// Each of these used to be accepted and fail later, far from the admin who
/// typed it: a `games_per_batch` of 0 makes every claim generate the seed the
/// previous claim already took, so the job retries a unique-index violation
/// forever and dispatches nothing; a confidence of 100% puts a logarithm of
/// zero in the test's interval, which then never closes. Every problem is
/// reported at once, like registration does.
/// The most player configs one games or pairs request names. Twelve make 66
/// jobs, every pairing once -- more than a fleet runs at once, and as many as
/// the allocation page can usefully show.
const MAX_ROUND_ROBIN_CONFIGS: usize = 12;

/// A games or pairs request's configs: at least one, at most
/// [`MAX_ROUND_ROBIN_CONFIGS`], none twice. A config named twice would pair
/// with itself in the middle of a round robin -- a self-play job is asked for
/// by naming it alone.
fn round_robin_problems(mut err: AppError, ids: &[Uuid]) -> AppError {
    if ids.is_empty() {
        err = err.with_field("player_config_ids", "must name at least one player config");
    } else if ids.len() > MAX_ROUND_ROBIN_CONFIGS {
        err = err.with_field(
            "player_config_ids",
            format!(
                "must name at most {MAX_ROUND_ROBIN_CONFIGS} player configs ({} jobs)",
                MAX_ROUND_ROBIN_CONFIGS * (MAX_ROUND_ROBIN_CONFIGS - 1) / 2
            ),
        );
    }
    let mut seen = std::collections::HashSet::new();
    if let Some(twice) = ids.iter().find(|id| !seen.insert(**id)) {
        err = err.with_field(
            "player_config_ids",
            format!("names {twice} twice; name a config alone for a self-play job"),
        );
    }
    err
}

/// The seatings a games or pairs request asks for, as (player 1, player 2):
/// one config plays itself, and n ≥ 2 give every pairing once, each seated in
/// the order the configs were listed. The seat matters little -- a pair swaps
/// seats within itself, and a games batch is even, so each player moves first
/// in half of every task's games -- but a fixed rule keeps "A vs B" A's
/// player-1 side wherever the job is shown. Empty for every other job type.
fn pairings(config: &JobTypeConfig) -> Vec<(Uuid, Uuid)> {
    let ids = match config {
        JobTypeConfig::Game { player_config_ids, .. }
        | JobTypeConfig::GamePair { player_config_ids, .. } => player_config_ids,
        _ => return Vec::new(),
    };
    if let [only] = ids.as_slice() {
        return vec![(*only, *only)];
    }
    let mut out = Vec::with_capacity(ids.len() * ids.len().saturating_sub(1) / 2);
    for (i, a) in ids.iter().enumerate() {
        for b in &ids[i + 1..] {
            out.push((*a, *b));
        }
    }
    out
}

fn validate_job_body(body: &CreateJobBody) -> AppResult<()> {
    let mut err = AppError::bad_request("job settings are invalid");
    if let Some(problem) = name_problem(&job_name(body)) {
        err = err.with_field("name", problem);
    }
    if !matches!(body.variant.as_str(), "classic" | "wordsmog") {
        err = err.with_field("variant", "must be 'classic' or 'wordsmog'");
    }
    // MAGPIE takes any integer, but a bonus that takes points away from a
    // bingo is a typo, not a variant anyone plays -- and so is one in the
    // thousands, which the plausibility bounds, absolute and set for an
    // ordinary bonus, would refuse on every honest batch. The column's CHECK.
    if body.bingo_bonus.is_some_and(|b| !(0..=MAX_BINGO_BONUS).contains(&b)) {
        err = err.with_field(
            "bingo_bonus",
            format!("must be between 0 and {MAX_BINGO_BONUS}"),
        );
    }
    if let Some(cutoff) = body.sim_cutoff {
        // The range MAGPIE's -cutoff accepts, and the column's CHECK.
        if !(cutoff.is_finite() && (0.0..=100.0).contains(&cutoff)) {
            err = err.with_field("sim_cutoff", "must be between 0 and 100");
        } else if body.job_type == JobType::LeaveGeneration {
            err = err.with_field(
                "sim_cutoff",
                "a leave job never simulates, so it has no cutoff: leave it out",
            );
        }
    }
    // Read loosely, a typo ("v1.6.0", "1") was 0.0.0: the lowest floor there
    // is, so a raise meant to keep older builds off the job let them all on.
    if let Some(text) = body.min_magpie_version.as_deref() {
        if crate::version::Version::parse_strict(text).is_none() {
            err = err.with_field("min_magpie_version", "must be a version such as 0.1.1");
        }
    }

    let match_test = |mut err: AppError,
                      unit: &str,
                      batch: i32,
                      min_units: Option<i32>,
                      max_units: i32,
                      test: &TestRequest| {
        if batch < 1 {
            err = err.with_field(format!("{unit}s_per_batch"), "must be at least 1");
        }
        if max_units < 1 {
            err = err.with_field(format!("max_{unit}s"), "must be at least 1");
        }
        let min_field = format!("min_{unit}s");
        // Refused rather than ignored: a request that sets the test up
        // without turning it on would, ignored, play to its cap with no test,
        // and nothing would say so until it finished.
        if !test.test_enabled {
            let stated = [
                min_units.is_some().then_some(min_field),
                test.confidence_pct.is_some().then(|| "confidence_pct".to_string()),
            ];
            for field in stated.into_iter().flatten() {
                err = err.with_field(
                    field,
                    "is a setting of the match test: send test_enabled: true to run it, or leave \
                     it out",
                );
            }
            return err;
        }
        // The test's interval is asymptotic: it holds once enough units are
        // in for their mean to be close to normal, which is what the floor
        // is for. Above the cap it would never be reached.
        match min_units {
            None => err = err.with_field(min_field, "is required when the job runs the match test"),
            Some(min_units) if min_units < 1 => err = err.with_field(min_field, "must be at least 1"),
            Some(min_units) if min_units > max_units && max_units >= 1 => {
                err = err.with_field(min_field, format!("must be at most max_{unit}s ({max_units})"))
            }
            Some(_) => {}
        }
        // Not 100: the interval's logarithm of 1 - confidence is then
        // infinite, and it never closes. At or below half it is no test.
        let confidence = test.settings(min_units).confidence_pct;
        if !(confidence.is_finite() && confidence > 50.0 && confidence < 100.0) {
            err = err.with_field("confidence_pct", "must be above 50 and below 100");
        }
        err
    };

    err = match &body.config {
        JobTypeConfig::OpeningRack {
            racks_per_batch, rack_size, consensus_pct, min_results_per_rack,
            max_results_per_rack, ..
        } => {
            if !(1..=MAX_RACKS_PER_BATCH).contains(racks_per_batch) {
                err = err.with_field(
                    "racks_per_batch",
                    format!("must be between 1 and {MAX_RACKS_PER_BATCH}"),
                );
            }
            if !(1..=RACK_SIZE as i32).contains(rack_size) {
                err = err.with_field("rack_size", format!("must be between 1 and {RACK_SIZE}"));
            }
            consensus_problems(err, *consensus_pct, *min_results_per_rack, *max_results_per_rack)
        }
        JobTypeConfig::Game {
            player_config_ids, games_per_batch, min_games, max_games, test, capture_positions,
            capture_first_divergence, threading_mode,
        } => {
            let err = threading_mode_problem(round_robin_problems(err, player_config_ids), threading_mode);
            let mut err = match_test(err, "game", *games_per_batch, *min_games, *max_games, test);
            err = games_batch_field(err, "game", 1, *games_per_batch, *capture_positions);
            if *capture_first_divergence {
                err = err.with_field(
                    "capture_first_divergence",
                    "a games job plays no pairs, so its games have no first divergence",
                );
            }
            // MAGPIE alternates the first mover within one run, from player 1,
            // and every task is a run of its own: at a batch of 1 player 1
            // moved first in every game of the job, so the first move's edge
            // read as player 1's strength: enough for the test of the time to
            // pass two identical players (the audit's pass 18). An even batch
            // gives each player the first move equally in every task. Game
            // pairs swap it within each pair already.
            if *games_per_batch >= 1 && games_per_batch % 2 != 0 {
                err = err.with_field(
                    "games_per_batch",
                    "must be even, so each player moves first in half of every task's games",
                );
            }
            err
        }
        JobTypeConfig::GamePair {
            player_config_ids, pairs_per_batch, min_pairs, max_pairs, test, capture_positions,
            capture_first_divergence, threading_mode,
        } => {
            let err = threading_mode_problem(round_robin_problems(err, player_config_ids), threading_mode);
            let mut err = match_test(err, "pair", *pairs_per_batch, *min_pairs, *max_pairs, test);
            if *capture_first_divergence && !*capture_positions {
                err = err.with_field(
                    "capture_first_divergence",
                    "keeps only some of the positions a job captures, so it needs \
                     capture_positions",
                );
            }
            games_batch_field(err, "pair", 2, *pairs_per_batch, *capture_positions)
        }
        JobTypeConfig::Leave { num_iterations, target_rack_counts, racks_per_task, .. } => {
            for (field, value) in [
                ("num_iterations", *num_iterations),
                ("racks_per_task", *racks_per_task),
            ] {
                if value < 1 {
                    err = err.with_field(field, "must be at least 1");
                }
            }
            if target_rack_counts.is_empty() {
                err = err.with_field("target_rack_counts", "must list at least one generation's target");
            } else if target_rack_counts.len() > MAX_LEAVE_GENERATIONS {
                err = err.with_field(
                    "target_rack_counts",
                    format!("must list at most {MAX_LEAVE_GENERATIONS} generations"),
                );
            } else if let Some(bad) =
                target_rack_counts.iter().find(|t| !(1..=MAX_TARGET_RACK_COUNT).contains(*t))
            {
                err = err.with_field(
                    "target_rack_counts",
                    format!("every target must be between 1 and {MAX_TARGET_RACK_COUNT}, not {bad}"),
                );
            }
            // Every claim carries its task's forced racks, and so does every
            // `leave_requests` row: a typo of millions sent the generation's
            // whole universe with each.
            if *racks_per_task > MAX_RACKS_PER_TASK {
                err = err.with_field(
                    "racks_per_task",
                    format!("must be at most {MAX_RACKS_PER_TASK}"),
                );
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

/// A games or pairs job's threading mode, which MAGPIE takes as `igp` or `pgp`
/// and nothing else (the column's CHECK).
fn threading_mode_problem(err: AppError, threading_mode: &str) -> AppError {
    if matches!(threading_mode, "igp" | "pgp") {
        err
    } else {
        err.with_field("threading_mode", "must be 'igp' or 'pgp'")
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
    max_results_per_rack: i32,
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
                 recorder_type 'all' ('equity' keeps only the moves within its equity \
                 margin), a simulating one, or set num_plays_recorded to 1."
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
    // A static analysis is deterministic: a second one of a rack ranks it
    // exactly as the first did, so asking for more costs the fleet and can
    // only ever agree.
    if plies == 0 && max_results_per_rack > 1 {
        return Err(AppError::bad_request(
            "a static player's analyses of a rack always agree, so there is no consensus to seek",
        )
        .with_field(
            "max_results_per_rack",
            "a static player analyses each rack once: set min_results_per_rack and \
             max_results_per_rack to 1, or use a simulating player",
        ));
    }
    Ok(())
}

/// MAGPIE has one win% model for the whole run, not one per player, even
/// though it lives on `player_configs` (so that table stays the exhaustive
/// source of what a job asked for -- see the migration comment on
/// `winpct_id`). A `games`/`game_pairs` job whose two player configs disagree
/// on it can't be honored, so job creation rejects it here rather than
/// leaving one worker's value to win silently.
///
/// `movegen_margin` is not compared: autoplay generates every move with a
/// margin of 0 and its own record type, so in a games or pairs job a player's
/// margin (and recorder) is never read -- only an opening-rack static analysis
/// reads them. Refusing two margins refused a job over a difference that
/// changes nothing it plays.
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
        "SELECT p1.winpct_id AS p1_winpct_id, p2.winpct_id AS p2_winpct_id
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
    // How many plays and plies a captured position keeps is one setting for
    // the whole run in MAGPIE, which reads both from player 1: player 2's
    // would be shown on the job page and never applied. So they must agree,
    // as the win% model must (see `validate_shared_player_options`).
    let recorded = sqlx::query_as::<_, (Uuid, i32, i32)>(
        "SELECT id, num_plays_recorded, num_plies_recorded FROM player_configs WHERE id = ANY($1)",
    )
    .bind(vec![player1_config_id, player2_config_id])
    .fetch_all(&mut *conn)
    .await?;
    let of = |id: Uuid| {
        recorded
            .iter()
            .find(|(row, _, _)| *row == id)
            .map(|(_, plays, plies)| (*plays, *plies))
            .ok_or_else(|| AppError::bad_request("player config not found"))
    };
    let (cap, plies) = of(player1_config_id)?;
    let (cap2, plies2) = of(player2_config_id)?;
    if (cap, plies) != (cap2, plies2) {
        return Err(AppError::bad_request("job settings are invalid").with_field(
            "capture_positions",
            format!(
                "the players record {cap} and {cap2} plays and {plies} and {plies2} plies per \
                 position; MAGPIE keeps one number of each for the whole run, player 1's, so \
                 with capture on both configs must agree on num_plays_recorded and \
                 num_plies_recorded"
            ),
        ));
    }
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

/// MAGPIE finds a file by its role and name, so a job cannot pin two
/// different files under one: its two players on `NWL23.klv2` of two data
/// releases -- the natural comparison after a MAGPIE-DATA update -- would have
/// every worker hash its one `NWL23.klv2` against both digests, decline every
/// task as missing data, and, with the job the only one active, be told to
/// download data that cannot help (thirty-first audit). Read from the files
/// the job's tasks will state, after its config is written, so every role is
/// covered however the job came to pin it.
async fn refuse_one_name_for_two_files(conn: &mut sqlx::PgConnection, job: &Job) -> AppResult<()> {
    let files = crate::jobs::expected_data(conn, job).await?;
    for (i, file) in files.iter().enumerate() {
        if let Some(other) = files[i + 1..]
            .iter()
            .find(|other| other.role == file.role && other.name == file.name && other.sha256 != file.sha256)
        {
            return Err(AppError::bad_request(format!(
                "this job would pin two different {} files named {:?} (from {} and {}): a worker \
                 finds a file by its name and can hold only one of them. Choose players whose \
                 files come from the same data release.",
                file.role, file.name, file.tarball_date, other.tarball_date
            )));
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

/// A leave job's player, held to what leave generation measures.
///
/// A generation's leave values are the mean equity of every rack the bot
/// drew, as MAGPIE's own `leavegen` computes them: static play, ranked on
/// equity, with the generation's KLV supplying the leave half of that equity.
/// A simulating player would rank on something else and play each game orders
/// of magnitude slower; a score sort ignores the very leave values each
/// generation feeds back in, so no generation would learn from the last; and a
/// rack info table caches the leave values of one fixed KLV, where every
/// generation plays a new one -- MAGPIE loads none for this job type, so the
/// setting would be shown and not honoured. Each is refused rather than
/// quietly overridden.
///
/// Its lexicon must be a kwg that the job's distribution can spell. Its leaves
/// are not checked: every generation plays a server-built KLV, generation 1's
/// being a zeroed one, and the player's own is never loaded.
async fn validate_leave_player(
    conn: &mut sqlx::PgConnection,
    player_config_id: Uuid,
    letterdist_name: &str,
) -> AppResult<()> {
    let (plies, sort, use_rit, endgame_plies, kwg_role, lexicon) =
        sqlx::query_as::<_, (i32, String, bool, i32, String, String)>(
            "SELECT pc.num_plies, pc.sort_strategy, pc.use_rit, pc.endgame_plies, kwg.role,
                    kwg.name
             FROM player_configs pc JOIN input_data kwg ON kwg.id = pc.kwg_id
             WHERE pc.id = $1",
        )
        .bind(player_config_id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or_else(|| AppError::bad_request("no such player config"))?;

    let mut problems = Vec::new();
    if plies > 0 {
        problems.push(format!("simulates {plies} plies (it must play statically: num_plies 0)"));
    }
    if sort != "equity" {
        problems.push(format!("sorts on {sort} (it must sort on equity)"));
    }
    if use_rit {
        problems.push("asks for a rack info table (it must not)".to_string());
    }
    // A leave game ends before the bag is small enough for either solver, so
    // the settings would be shown and never honoured.
    if endgame_plies > 0 {
        problems.push(format!(
            "solves endgames to {endgame_plies} plies (it must not: leave games end before the \
             endgame, so set endgame_plies to 0)"
        ));
    }
    if !problems.is_empty() {
        return Err(AppError::bad_request("this player config cannot generate leaves").with_field(
            "player_config_id",
            format!("leave generation plays statically on equity; this config {}", problems.join(", ")),
        ));
    }
    if kwg_role != "kwg" {
        return Err(AppError::bad_request(format!(
            "expected a kwg row, but {lexicon} is a {kwg_role} row"
        )));
    }
    if !crate::compat::lex_ld_compat(&lexicon, letterdist_name) {
        return Err(AppError::bad_request(format!(
            "lexicon {lexicon:?} is not compatible with letter distribution {letterdist_name:?}"
        )));
    }
    Ok(())
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

/// What a games or pairs job's two players must agree on, checked for one
/// pairing: the win% model, the files MAGPIE loads by name, and with capture
/// on, what a captured position keeps. A round robin checks every pairing
/// before it inserts anything.
async fn validate_pairing(
    conn: &mut sqlx::PgConnection,
    (player1, player2): (Uuid, Uuid),
    capture_positions: bool,
    letterdist_name: &str,
) -> AppResult<()> {
    validate_shared_player_options(&mut *conn, player1, player2).await?;
    validate_player_compatibility(
        &mut *conn,
        &[("player1", player1), ("player2", player2)],
        letterdist_name,
    )
    .await?;
    if capture_positions {
        validate_capture_play_cap(&mut *conn, player1, player2).await?;
    }
    Ok(())
}

/// Inserts the job's config row. `pair` is the games or pairs job's seating
/// (from [`pairings`], already checked by [`validate_pairing`]); `None` for
/// every other type.
async fn insert_job_config(
    conn: &mut sqlx::PgConnection,
    job: &Job,
    config: &JobTypeConfig,
    pair: Option<(Uuid, Uuid)>,
    letterdist_name: &str,
) -> AppResult<()> {
    // The untagged config must actually match the declared job type, or the job
    // would exist with no config row and never dispatch anything.
    let mismatch = || AppError::bad_request("config fields do not match the requested job_type");

    match (job.job_type, config) {
        (
            JobType::OpeningRack,
            JobTypeConfig::OpeningRack {
                player_config_id, racks_per_batch, rack_size, consensus_pct,
                min_results_per_rack, max_results_per_rack,
            },
        ) => {
            validate_player_compatibility(
                &mut *conn,
                &[("player", *player_config_id)],
                letterdist_name,
            )
            .await?;
            validate_opening_rack_player(&mut *conn, *player_config_id, *max_results_per_rack)
                .await?;
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
                     (job_id, player_config_id, racks_per_batch, rack_size, total_racks,
                      consensus_pct, min_results_per_rack, max_results_per_rack)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            )
            .bind(job.id)
            .bind(player_config_id)
            .bind(racks_per_batch)
            .bind(rack_size)
            .bind(total_racks)
            .bind(consensus_pct)
            .bind(min_results_per_rack)
            .bind(max_results_per_rack)
            .execute(conn)
            .await?;
        }
        (
            JobType::Games,
            JobTypeConfig::Game {
                games_per_batch, min_games, max_games, test, capture_positions, threading_mode, ..
            },
        ) => {
            let (player1_config_id, player2_config_id) = pair.ok_or_else(mismatch)?;
            let test = test.settings(*min_games);
            sqlx::query(
                "INSERT INTO job_game_config
                     (job_id, player1_config_id,
                      player2_config_id, games_per_batch, test_enabled, min_games, max_games,
                      confidence_pct, capture_positions, threading_mode)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
            )
            .bind(job.id)
            .bind(player1_config_id).bind(player2_config_id)
            .bind(games_per_batch).bind(test.enabled).bind(test.min_units).bind(max_games)
            .bind(test.confidence_pct)
            .bind(capture_positions)
            .bind(threading_mode)
            .execute(conn)
            .await?;
        }
        (
            JobType::GamePairs,
            JobTypeConfig::GamePair {
                pairs_per_batch, min_pairs, max_pairs, test, capture_positions,
                capture_first_divergence, threading_mode, ..
            },
        ) => {
            let (player1_config_id, player2_config_id) = pair.ok_or_else(mismatch)?;
            let test = test.settings(*min_pairs);
            sqlx::query(
                "INSERT INTO job_game_pair_config
                     (job_id, player1_config_id,
                      player2_config_id, pairs_per_batch, test_enabled, min_pairs, max_pairs,
                      confidence_pct, capture_positions, capture_first_divergence, threading_mode)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
            )
            .bind(job.id)
            .bind(player1_config_id).bind(player2_config_id)
            .bind(pairs_per_batch).bind(test.enabled).bind(test.min_units).bind(max_pairs)
            .bind(test.confidence_pct)
            .bind(capture_positions)
            .bind(capture_first_divergence)
            .bind(threading_mode)
            .execute(conn)
            .await?;
        }
        (
            JobType::LeaveGeneration,
            JobTypeConfig::Leave {
                player_config_id, num_iterations, target_rack_counts, racks_per_task,
            },
        ) => {
            validate_leave_player(&mut *conn, *player_config_id, letterdist_name).await?;
            // Every generation seeds and hands out full racks over the pinned
            // distribution, so one whose racks cannot be spelt is refused now
            // rather than at the first claim.
            let job_data = crate::jobs::load_job_data(&mut *conn, job.id).await?;
            crate::jobs::racks::RackIndex::new(&job_data.letterdist, RACK_SIZE)?;
            sqlx::query(
                "INSERT INTO job_leave_config
                     (job_id, player_config_id, num_iterations,
                      target_rack_counts, racks_per_task)
                 VALUES ($1,$2,$3,$4,$5)",
            )
            .bind(job.id).bind(player_config_id)
            .bind(num_iterations).bind(target_rack_counts)
            .bind(racks_per_task)
            .execute(conn)
            .await?;
        }
        _ => return Err(mismatch()),
    }
    Ok(())
}

/// The most jobs one allocation change names: more than any fleet runs at once.
const MAX_ALLOCATION_ROWS: usize = 200;

#[derive(Deserialize)]
struct AllocationsBody {
    allocations: Vec<AllocationRow>,
}

#[derive(Deserialize)]
struct AllocationRow {
    job_id: Uuid,
    allocation: i32,
}

#[derive(Serialize)]
struct AllocationsResult {
    /// Every job the request named, as it now stands, in the request's order.
    jobs: Vec<Job>,
}

/// Several jobs' allocations at once, checked as a whole: the active jobs
/// must sum to at most 100% *after* the change, not after each step of it.
/// One at a time, moving 20% from a job at 60% to one at 40% meant lowering
/// the first before the second could be raised, and an admin rebalancing
/// three jobs had to work out an order that never passed through 101%.
///
/// This is the only way a job is activated or deactivated: the allocation is
/// the switch (`jobs_allocation_is_status`). A row above 0% leaves its job
/// active at that allocation, activating it if it was not; a row at 0% leaves
/// it inactive at 0%, deactivating it if it was active. Nothing of the old
/// share is kept: a separate activate and deactivate, with a remembered
/// allocation between them, made "inactive" and "0%" two states that could
/// disagree -- an active job at 0% was on offer to nobody while every page
/// called it running. A job the request does not name keeps what it has. A
/// completed job, or one being purged, is refused, and so is everything else
/// in the request with it: nothing changes unless all of it does.
///
/// Each job that changes is audited once: `job.activated` / `job.deactivated`
/// when its status changes, and `job.allocation_changed` when an active job
/// stays active at a new share -- each with the allocation from what to what.
async fn set_allocations(
    State(state): State<AppState>,
    admin: AdminUser,
    method: Method,
    headers: HeaderMap,
    jar: CookieJar,
    ApiJson(body): ApiJson<AllocationsBody>,
) -> AppResult<Json<AllocationsResult>> {
    csrf::verify(&method, &headers, &jar)?;

    let mut err = AppError::bad_request("allocations are invalid");
    if body.allocations.is_empty() {
        err = err.with_field("allocations", "name at least one job");
    } else if body.allocations.len() > MAX_ALLOCATION_ROWS {
        err = err.with_field("allocations", format!("at most {MAX_ALLOCATION_ROWS} jobs at once"));
    }
    let mut seen = std::collections::HashSet::new();
    for (i, row) in body.allocations.iter().enumerate() {
        if !(0..=100).contains(&row.allocation) {
            err = err.with_field(format!("allocations[{i}].allocation"), "must be between 0 and 100");
        }
        if !seen.insert(row.job_id) {
            err = err.with_field(format!("allocations[{i}].job_id"), "names a job named before it");
        }
    }
    if !err.fields.is_empty() {
        return Err(err);
    }

    let mut purges = std::collections::HashMap::new();
    for row in &body.allocations {
        purges.insert(row.job_id, refuse_while_purging(&state, row.job_id)?);
    }

    // First, for each job this activates: a leave job's generation-0 KLV --
    // creation writes it after committing, so a failed object-store write
    // there leaves a job without one, and every claim against it would fail
    // -- and its derived files under this deployment's builder, which may
    // have moved since creation. Outside the transaction, for the reason
    // creation builds them outside its own.
    for row in body.allocations.iter().filter(|r| r.allocation > 0) {
        let unlocked = match crate::jobstats::load_job(&state.pool, row.job_id).await {
            Ok(job) => job,
            // Named below, with the rest of what is wrong.
            Err(_) => continue,
        };
        if unlocked.status == JobStatus::Active || unlocked.status == JobStatus::Completed {
            continue;
        }
        if !registry::job_artifacts_ready(&state.pool, &unlocked).await? {
            registry::initialize_job_artifacts(&state, &unlocked).await?;
        }
        request_derived_data(&state, row.job_id).await?;
    }

    let mut tx = state.pool.begin().await?;
    // Every row first, in id order, then the activation lock, which
    // serializes allocation changes: the row locks cover only the jobs named,
    // so two requests naming different jobs would each read the other's
    // allocations as they were and together exceed 100%.
    let mut ids: Vec<Uuid> = body.allocations.iter().map(|r| r.job_id).collect();
    ids.sort();
    let locked: Vec<Job> = sqlx::query_as::<_, Job>(
        "SELECT * FROM jobs WHERE id = ANY($1) ORDER BY id FOR UPDATE",
    )
    .bind(&ids)
    .fetch_all(&mut *tx)
    .await?;
    let before: std::collections::HashMap<Uuid, Job> =
        locked.into_iter().map(|job| (job.id, job)).collect();
    let mut err = AppError::bad_request("allocations are invalid");
    for (i, row) in body.allocations.iter().enumerate() {
        match before.get(&row.job_id) {
            None => err = err.with_field(format!("allocations[{i}].job_id"), "no such job"),
            Some(job) if job.status == JobStatus::Completed => {
                err = err.with_field(
                    format!("allocations[{i}].job_id"),
                    "the job is completed, and a completed job cannot be reactivated",
                )
            }
            Some(_) => refuse_if_purged_since(&state.dispatch_holds, row.job_id, purges[&row.job_id])?,
        }
    }
    if !err.fields.is_empty() {
        return Err(err);
    }
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('birdtest.activate'))")
        .execute(&mut *tx)
        .await?;

    let others = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT SUM(allocation) FROM jobs WHERE status = 'active' AND id <> ALL($1)",
    )
    .bind(&ids)
    .fetch_one(&mut *tx)
    .await?
    .unwrap_or(0);
    let named: i64 = body.allocations.iter().map(|r| i64::from(r.allocation)).sum();
    if others + named > 100 {
        return Err(AppError::conflict(format!(
            "the active jobs would allocate {}% between them; the most is 100%{}",
            others + named,
            if others > 0 {
                format!(" (the jobs not named here already allocate {others}%)")
            } else {
                String::new()
            }
        )));
    }

    let mut activated = Vec::new();
    let mut changed = Vec::new();
    for row in &body.allocations {
        let job = &before[&row.job_id];
        if row.allocation == job.allocation {
            continue;
        }
        let moved = format!("{}% -> {}%", job.allocation, row.allocation);
        if row.allocation > 0 {
            sqlx::query(
                "UPDATE jobs SET status = 'active', allocation = $1, activated_at = now() WHERE id = $2",
            )
            .bind(row.allocation)
            .bind(row.job_id)
            .execute(&mut *tx)
            .await?;
            // The job joins the others level with the lowest of the jobs
            // being served, rather than with a lifetime deficit to work off at
            // their expense -- and a new allocation rescales its ratio, so an
            // active job's change does the same.
            crate::scheduler::join_at_parity(&mut tx, row.job_id, state.cfg.heartbeat_timeout).await?;
        } else {
            sqlx::query("UPDATE jobs SET status = 'inactive', allocation = 0 WHERE id = $1")
                .bind(row.job_id)
                .execute(&mut *tx)
                .await?;
        }
        // One row per job: a status change when it switched on or off,
        // naming the allocation it moved between, and otherwise the new
        // allocation alone.
        let to = if row.allocation > 0 { JobStatus::Active } else { JobStatus::Inactive };
        if to != job.status {
            let action = if to == JobStatus::Active { "job.activated" } else { "job.deactivated" };
            audit::log_status_change(
                &mut tx,
                action,
                admin.0.id,
                row.job_id,
                status_name(job.status),
                status_name(to),
                Some(&moved),
            )
            .await?;
            if to == JobStatus::Active {
                activated.push(row.job_id);
            }
        } else {
            audit::log_detail(
                &mut tx,
                "job.allocation_changed",
                admin.0.id,
                "job",
                row.job_id.to_string(),
                Some(row.job_id),
                moved,
            )
            .await?;
        }
        changed.push(row.job_id);
    }

    let after: std::collections::HashMap<Uuid, Job> =
        sqlx::query_as::<_, Job>("SELECT * FROM jobs WHERE id = ANY($1)")
            .bind(&ids)
            .fetch_all(&mut *tx)
            .await?
            .into_iter()
            .map(|job| (job.id, job))
            .collect();
    tx.commit().await?;
    for id in &activated {
        state.finish_checks.rearm_idle(*id);
    }
    for id in &changed {
        super::worker::push_after_change(&state, *id);
    }
    let mut jobs = Vec::with_capacity(body.allocations.len());
    for row in &body.allocations {
        if let Some(job) = after.get(&row.job_id) {
            jobs.push(job.clone());
        }
    }
    Ok(Json(AllocationsResult { jobs }))
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
    refuse_if_purged_since(&state.dispatch_holds, id, purges)?;
    // Already completed -- by the server's own finish check, say, while the
    // admin's page was stale -- is a conflict, not a second completion on
    // record (the audit's pass 23).
    if before.status == JobStatus::Completed {
        return Err(AppError::conflict("this job is already completed"));
    }
    // At 0%, as every completed job is: its share goes back to the fleet.
    let job = sqlx::query_as::<_, Job>(
        "UPDATE jobs SET status = 'completed', allocation = 0 WHERE id = $1 RETURNING *",
    )
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
        None,
    )
    .await?;
    tx.commit().await?;
    // Nothing checks a completed job, so its finish-check counters go. Not
    // final for an opening-rack job: a consensus edit that unsettles racks
    // reopens it (and, force-completed mid-first-pass, it then resumes that
    // pass; see `update_consensus`), and its counters start afresh.
    state.finish_checks.forget(id);
    super::worker::push_after_change(&state, id);
    Ok(Json(job))
}

#[derive(Deserialize)]
struct ConsensusBody {
    consensus_pct: Option<f64>,
    min_results_per_rack: Option<i32>,
    max_results_per_rack: Option<i32>,
}

#[derive(Serialize)]
struct ConsensusResult {
    /// The job as it now stands: reopened, when the change unsettled racks of
    /// a completed job; completed, when it settled the last of an active one's.
    job: Job,
    /// Its opening-rack settings as they now stand.
    config: OpeningRackConfig,
    /// How many of its racks the settings leave unsettled.
    unsettled_racks: i64,
    /// Whether the change took a completed job back out of completed. A
    /// reopened job is inactive at 0% until an allocation is set for it.
    reopened: bool,
}

/// Changes an opening-rack job's consensus settings -- the fewest and most
/// analyses a rack gets, and the share of them that must agree on its best
/// move -- and restates every rack under them (`opening_rack::restate_racks`).
/// The only part of a job's configuration that changes after creation. Only
/// the fields given change; none that differ is a `200` that writes nothing.
///
/// The job then starts or stops to match. A completed job the change leaves
/// with unsettled racks is reopened, inactive at 0%: the admin gives it an
/// allocation when it should run. A completed job's final exports become
/// snapshots, since the standings they carry are the old settings': the job
/// has a new final corpus once it completes again, or, left completed, once
/// it is exported again. An active job
/// the change leaves with every rack settled completes now, or with its last
/// in-flight claim's submission. An inactive job keeps its status, and
/// completes when next activated if there is nothing left for it to do.
///
/// Under the locks a purge takes, in its order: the job's dispatch lock, so
/// no claim is issued under the old settings; every open claim, so no
/// submission is storing analyses while the racks are restated (a submission
/// takes its claim first, so one already storing is waited for, and one that
/// starts after reads the new settings once this commits); then the job's
/// row. Claims and submissions read the settings fresh rather than from the
/// job's cached template, which keeps them as the job was created.
///
/// And under the hold a purge takes (`jobs::DispatchHolds`), on a task of its
/// own ([`run_to_completion`]), for the same reasons: restating a full
/// English job rewrites millions of rows, and for that long every claim
/// considering the job waited out the dispatch lock's bounded wait on a pool
/// connection, and every submission for one of its claims its five-second
/// claim lock, which is how a purge used to fill the pool. With the hold,
/// claims skip the job, those submissions are answered `503` at once, and a
/// second edit, a purge or a lifecycle action meanwhile is a `409`. The hold
/// is not counted as a purge (it is not one: the finish check's purge
/// witness must not see it), and it always ends with the reclaim grace,
/// committed or not: the job's claims outlive an edit, and their heartbeats
/// were skipped while it held them.
///
/// So the request is checked, and one that changes nothing answered, before
/// any of that is taken, and checked again under the locks (the settings may
/// have changed between). Checked only under them, a refused edit -- a games
/// job, a typo in the share -- or a double click's second, unchanged one
/// still held the job's claims and submissions off while it locked them, and
/// ended with the grace: its lapsed claims, a games job's included, were not
/// reclaimed for a heartbeat timeout.
async fn update_consensus(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    method: Method,
    headers: HeaderMap,
    jar: CookieJar,
    ApiJson(body): ApiJson<ConsensusBody>,
) -> AppResult<Json<ConsensusResult>> {
    csrf::verify(&method, &headers, &jar)?;
    {
        let mut conn = state.pool.acquire().await?;
        let job = sqlx::query_as::<_, Job>("SELECT * FROM jobs WHERE id = $1")
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?
            .ok_or_else(|| AppError::not_found("no such job"))?;
        let (before, new) = consensus_request(&mut conn, &job, &body, false).await?;
        if new == before.consensus() {
            return Ok(Json(unchanged_consensus(&mut conn, job, before).await?));
        }
    }
    // Read before the hold is taken, for the check under the row lock below.
    let purges = state.dispatch_holds.claims_holds_taken(id);
    let hold = state
        .dispatch_holds
        .try_hold_claims_uncounted(id, state.cfg.heartbeat_timeout)
        .ok_or_else(|| AppError::conflict(ALREADY_RUNNING))?;
    run_to_completion(consensus_body(state, admin.0.id, id, body, purges, hold)).await
}

async fn consensus_body(
    state: AppState,
    admin_id: Uuid,
    id: Uuid,
    body: ConsensusBody,
    purges: u64,
    hold: crate::jobs::DispatchHold,
) -> AppResult<Json<ConsensusResult>> {
    let mut tx = state.pool.begin().await?;
    crate::jobs::lock_job_dispatch(&mut tx, id).await?;
    lock_open_claims(&mut tx, id).await?;
    let job = load_job_for_update(&mut tx, id).await?;
    // By count alone: the claims hold held now is this edit's own, and no
    // other can be taken while it is.
    if state.dispatch_holds.claims_holds_taken(id) != purges {
        return Err(AppError::conflict(PURGED_WHILE_WAITING));
    }
    let (before, new) = consensus_request(&mut tx, &job, &body, true).await?;
    let old = before.consensus();
    if new == old {
        return Ok(Json(unchanged_consensus(&mut tx, job, before).await?));
    }

    let config = sqlx::query_as::<_, OpeningRackConfig>(
        "UPDATE job_opening_rack_config
         SET consensus_pct = $2, min_results_per_rack = $3, max_results_per_rack = $4
         WHERE job_id = $1 RETURNING *",
    )
    .bind(id)
    .bind(new.consensus_pct)
    .bind(new.min_results_per_rack)
    .bind(new.max_results_per_rack)
    .fetch_one(&mut *tx)
    .await?;
    let unsettled = crate::jobs::opening_rack::restate_racks(&mut tx, id, &new).await?;

    let mut changes = Vec::new();
    if new.min_results_per_rack != old.min_results_per_rack {
        changes.push(format!("min {} -> {}", old.min_results_per_rack, new.min_results_per_rack));
    }
    if new.max_results_per_rack != old.max_results_per_rack {
        changes.push(format!("max {} -> {}", old.max_results_per_rack, new.max_results_per_rack));
    }
    if new.consensus_pct != old.consensus_pct {
        changes.push(format!("consensus {}% -> {}%", old.consensus_pct, new.consensus_pct));
    }
    audit::log_detail(
        &mut tx,
        "job.consensus_changed",
        admin_id,
        "job",
        id.to_string(),
        Some(id),
        format!("{}; {unsettled} racks unsettled", changes.join(", ")),
    )
    .await?;

    // A completed job with racks to analyse again is taken back out of
    // completed. A job the server completed has its first pass covered (it
    // completes only once every rack is settled), so what it hands out once
    // given an allocation are the unsettled racks. One an admin force-completed may not: `next_request`
    // resumes its first pass where it stopped before it reissues anything.
    // That is the behaviour as built; whether an edit should undo a
    // force-complete at all is an open question (PLAN.md, "Editing the
    // consensus").
    let reopened = job.status == JobStatus::Completed && unsettled > 0;
    if reopened {
        // Inactive, at the 0% a completed job holds: completion kept nothing
        // of its old share to come back to, and the fleet may have been given
        // to other jobs since. The admin sets its allocation when it should
        // run (on the allocation page), as for a job just created.
        sqlx::query("UPDATE jobs SET status = 'inactive' WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        audit::log_status_change(&mut tx, "job.deactivated", admin_id, id, "completed", "inactive", None)
            .await?;
    }
    // Any change to a completed job's settings, not only one that reopens
    // it: each line of an export carries its rack's standing under the
    // settings it was read with, and an edit that leaves every rack settled
    // still restates them (a lower share turns racks settled without a
    // consensus into agreed ones). Left final, the export went on being
    // served as the completed job's corpus with the old standings.
    if job.status == JobStatus::Completed {
        crate::exports::unfinalize(&mut tx, id).await?;
    }
    let job = sqlx::query_as::<_, Job>("SELECT * FROM jobs WHERE id = $1")
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    // Released before the finish check: its purge witness takes a job whose
    // claims are held for one being purged, and would roll the completion
    // back. Not `committed()`: see above.
    drop(hold);

    // After the commit, nothing here is the edit failing: it committed, and
    // a 5xx would skip the finish check below and invite a second edit.
    // Logged instead, as the purge does.
    if job.status == JobStatus::Active {
        // A check paced out under the old settings must not delay the one
        // that can now complete it -- or wait on racks they unsettled.
        state.finish_checks.rearm_idle(id);
        if unsettled == 0 {
            // Every rack is settled: complete it now if nothing is in flight.
            // With claims out, the last one's submission completes it.
            if let Err(err) = super::worker::finish_idle_job(&state, id).await {
                tracing::error!(job_id = %id, error = %err.message, "finish check after a consensus change failed");
            }
        }
    }
    super::worker::push_after_change(&state, id);
    // The job as the finish check left it, or else as the edit committed it.
    let job = match crate::jobstats::load_job(&state.pool, id).await {
        Ok(job) => job,
        Err(err) => {
            tracing::warn!(job_id = %id, error = %err.message, "reloading an edited job failed; answering with it as committed");
            job
        }
    };
    Ok(Json(ConsensusResult { job, config, unsettled_racks: unsettled, reopened }))
}

/// An edit's request, checked: the job's settings as they stand (`FOR UPDATE`
/// when `lock`), and what the body makes of them. Only an opening-rack job
/// has them, and the result must be what creation would accept.
async fn consensus_request(
    conn: &mut sqlx::PgConnection,
    job: &Job,
    body: &ConsensusBody,
    lock: bool,
) -> AppResult<(OpeningRackConfig, ConsensusSettings)> {
    if job.job_type != JobType::OpeningRack {
        return Err(AppError::bad_request("only an opening-rack job has consensus settings"));
    }
    let before = sqlx::query_as::<_, OpeningRackConfig>(if lock {
        "SELECT * FROM job_opening_rack_config WHERE job_id = $1 FOR UPDATE"
    } else {
        "SELECT * FROM job_opening_rack_config WHERE job_id = $1"
    })
    .bind(job.id)
    .fetch_one(&mut *conn)
    .await?;
    let old = before.consensus();
    let new = ConsensusSettings {
        consensus_pct: body.consensus_pct.unwrap_or(old.consensus_pct),
        min_results_per_rack: body.min_results_per_rack.unwrap_or(old.min_results_per_rack),
        max_results_per_rack: body.max_results_per_rack.unwrap_or(old.max_results_per_rack),
    };
    let err = consensus_problems(
        AppError::bad_request("consensus settings are invalid"),
        new.consensus_pct,
        new.min_results_per_rack,
        new.max_results_per_rack,
    );
    if !err.fields.is_empty() {
        return Err(err);
    }
    validate_opening_rack_player(&mut *conn, before.player_config_id, new.max_results_per_rack).await?;
    Ok((before, new))
}

/// The answer to an edit that changes nothing: the job and its settings as
/// they are, and nothing written.
async fn unchanged_consensus(
    conn: &mut sqlx::PgConnection,
    job: Job,
    config: OpeningRackConfig,
) -> AppResult<ConsensusResult> {
    let unsettled: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM opening_rack_progress WHERE job_id = $1 AND NOT settled",
    )
    .bind(job.id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(ConsensusResult { job, config, unsettled_racks: unsettled, reopened: false })
}

/// What a job is about to lose, as a single line for `audit_log.reason`.
///
/// Counted inside the same transaction as the deletion that follows, so it
/// describes exactly what that statement removes. Cheap relative to the delete
/// itself, and the only record of the job's size that survives it.
/// `rack_standings` are an opening-rack job's `opening_rack_progress` rows
/// (left out until the October 2026 audit), `rack_progress` a leave job's
/// `leave_rack_progress`.
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
             (SELECT count(*) FROM opening_rack_progress WHERE job_id = $1)        AS rack_standings,
             (SELECT count(*) FROM leave_rack_progress WHERE job_id = $1)          AS rack_progress,
             (SELECT count(*) FROM leave_rack_staging WHERE job_id = $1)           AS staged_results,
             (SELECT count(*) FROM leave_generation_artifacts WHERE job_id = $1)   AS artifacts",
    )
    .bind(job_id)
    .fetch_one(conn)
    .await?;

    Ok(format!(
        "tasks={} claims={} game_results={} leave_records={} positions={} \
rack_standings={} rack_progress={} staged_results={} artifacts={}",
        row.get::<i64, _>("tasks"),
        row.get::<i64, _>("claims"),
        row.get::<i64, _>("game_results"),
        row.get::<i64, _>("leave_records"),
        row.get::<i64, _>("positions"),
        row.get::<i64, _>("rack_standings"),
        row.get::<i64, _>("rack_progress"),
        row.get::<i64, _>("staged_results"),
        row.get::<i64, _>("artifacts"),
    ))
}

/// The same, for an account deletion, which anonymizes the account and keeps
/// its work: what is destroyed -- its keys and outstanding codes (and, not
/// counted, its name, address and password) -- and, separately, the claims
/// and results that stay under the tombstone. It said `claims=… accepted=…`
/// as though they were lost with it (the audit's pass 10).
async fn user_census(conn: &mut sqlx::PgConnection, user_id: Uuid) -> AppResult<String> {
    use sqlx::Row;
    let row = sqlx::query(
        "SELECT
             (SELECT count(*) FROM api_keys WHERE user_id = $1)                    AS api_keys,
             (SELECT count(*) FROM email_confirmations WHERE user_id = $1)         AS confirmations,
             (SELECT count(*) FROM password_reset_tokens WHERE user_id = $1)       AS reset_tokens,
             (SELECT count(*) FROM task_claims WHERE claimed_by_user_id = $1)      AS claims,
             (SELECT count(*) FROM task_claims c
               WHERE c.claimed_by_user_id = $1 AND c.state = 'completed')          AS accepted",
    )
    .bind(user_id)
    .fetch_one(conn)
    .await?;

    Ok(format!(
        "destroyed: api_keys={} confirmations={} reset_tokens={}; kept: claims={} accepted={}",
        row.get::<i64, _>("api_keys"),
        row.get::<i64, _>("confirmations"),
        row.get::<i64, _>("reset_tokens"),
        row.get::<i64, _>("claims"),
        row.get::<i64, _>("accepted"),
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
    users: Earned,
    anonymous: Earned,
}

/// One kind of identity's share of a job, as parallel arrays in id order:
/// claims completed, the milliseconds they were held and the move generations
/// they reported -- every counter the submit path adds to.
#[derive(Default)]
struct Earned {
    ids: Vec<Uuid>,
    tasks: Vec<i64>,
    compute_ms: Vec<i64>,
    movegens: Vec<i64>,
}

impl Earned {
    /// Each identity of `column`'s kind with a completed claim of the job, and
    /// what those claims added, summed as the submit path added it.
    async fn count(conn: &mut sqlx::PgConnection, job_id: Uuid, column: &str) -> AppResult<Self> {
        let rows = sqlx::query_as::<_, (Uuid, i64, i64, i64)>(&format!(
            "SELECT c.{column}, COUNT(*)::bigint,
                    COALESCE(SUM({compute}), 0)::bigint,
                    COALESCE(SUM(c.movegens), 0)::bigint
             FROM task_claims c JOIN tasks t ON t.id = c.task_id
             WHERE t.job_id = $1 AND c.state = 'completed' AND c.{column} IS NOT NULL
             GROUP BY 1 ORDER BY 1",
            compute = super::worker::CLAIM_COMPUTE_MS,
        ))
        .bind(job_id)
        .fetch_all(&mut *conn)
        .await?;
        let mut earned = Earned::default();
        for (id, tasks, compute_ms, movegens) in rows {
            earned.ids.push(id);
            earned.tasks.push(tasks);
            earned.compute_ms.push(compute_ms);
            earned.movegens.push(movegens);
        }
        Ok(earned)
    }

    /// Takes it back from `table`, whose `key` the ids are: locked in id order
    /// first, since the update locks rows in whatever order its plan visits
    /// them, which a sorted array does not decide.
    async fn give_back(
        &self,
        conn: &mut sqlx::PgConnection,
        table: &str,
        key: &str,
    ) -> AppResult<()> {
        sqlx::query(&format!(
            "SELECT 1 FROM {table} WHERE {key} = ANY($1) ORDER BY {key} FOR NO KEY UPDATE"
        ))
        .bind(&self.ids)
        .execute(&mut *conn)
        .await?;
        sqlx::query(&format!(
            "UPDATE {table} x
             SET tasks_completed = GREATEST(x.tasks_completed - d.tasks, 0),
                 compute_ms = GREATEST(x.compute_ms - d.compute_ms, 0),
                 movegens = GREATEST(x.movegens - d.movegens, 0)
             FROM UNNEST($1::uuid[], $2::bigint[], $3::bigint[], $4::bigint[])
                  AS d(id, tasks, compute_ms, movegens)
             WHERE x.{key} = d.id"
        ))
        .bind(&self.ids)
        .bind(&self.tasks)
        .bind(&self.compute_ms)
        .bind(&self.movegens)
        .execute(&mut *conn)
        .await?;
        Ok(())
    }
}

impl Contributions {
    async fn count(conn: &mut sqlx::PgConnection, job_id: Uuid) -> AppResult<Self> {
        Ok(Contributions {
            users: Earned::count(conn, job_id, "claimed_by_user_id").await?,
            anonymous: Earned::count(conn, job_id, "claimed_by_anon_uuid").await?,
        })
    }

    /// The caller's last statement before it commits, so the rows are held
    /// for milliseconds. In id order, so two purges sharing contributors lock
    /// them in the same order rather than deadlocking at the end of both.
    async fn give_back(self, conn: &mut sqlx::PgConnection) -> AppResult<()> {
        self.users.give_back(conn, "users", "id").await?;
        self.anonymous.give_back(conn, "anonymous_workers", "uuid").await
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

const ALREADY_RUNNING: &str = "a purge, delete or consensus change of this job is running; its \
     result will show on the job's page and in the audit log when it finishes";

const PURGED_WHILE_WAITING: &str =
    "the job was purged while this waited for it; look at it again before acting";

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
/// and completing it then finished a job the purge had just emptied. A job
/// whose consensus is being changed is refused the same way, for the wait.
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
///
/// A hold held with the count unchanged is a consensus edit's, which is not
/// counted: one that took its hold after the first check and now queues on
/// this row behind the action. Refused too, since the edit is about to
/// restate the job, but as running, not as a purge: nothing was purged.
fn refuse_if_purged_since(holds: &crate::jobs::DispatchHolds, id: Uuid, taken: u64) -> AppResult<()> {
    if holds.claims_holds_taken(id) != taken {
        return Err(AppError::conflict(PURGED_WHILE_WAITING));
    }
    if holds.claims_held(id) {
        return Err(AppError::conflict(ALREADY_RUNNING));
    }
    Ok(())
}

/// Runs a purge, a delete or a consensus edit on a task of its own, and waits
/// for it.
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
        .map_err(|e| AppError::task_failed("the operation", e))?
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
    // And the job starts over as a new one does: inactive at 0%, whatever it
    // was. A completed job has nothing left to be complete about, and one
    // purged in place was an empty job nothing could ever run again. An
    // active one stops too, so a purge leaves every job in the one state a
    // created job is in, and the emptied job takes no claims until the admin
    // gives it an allocation again -- which joins it at parity with the jobs
    // being served then, as any activation does, rather than owed every claim
    // the jobs beside it have issued.
    sqlx::query(
        "UPDATE jobs SET claims_issued = 0, games_completed = 0, racks_analyzed = 0,
                         racks_settled = 0, racks_without_consensus = 0,
                         tasks_total = 0, tasks_completed = 0, last_completed_at = NULL,
                         test_decided_status = NULL, test_decided_lower = NULL,
                         test_decided_upper = NULL, test_decided_units = NULL,
                         status = 'inactive', allocation = 0, claims_baseline = 0
         WHERE id = $1",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM leave_rack_progress WHERE job_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    // An opening-rack consensus job's per-rack standings describe analyses
    // the claims above took with them.
    sqlx::query("DELETE FROM opening_rack_progress WHERE job_id = $1")
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
    /// Whether this is the completed job's final corpus, rather than a
    /// snapshot of a job still taking results (`exports`). False until built.
    is_final: bool,
    /// When the snapshot it was read in was taken; `None` until built.
    snapshot_at: Option<chrono::DateTime<chrono::Utc>>,
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

/// Build a job's results into one downloadable artifact: the final corpus of a
/// completed job, or a snapshot of one still running.
///
/// Returns immediately with an id; the work runs on a spawned task and the
/// admin polls `GET`. See `exports::start`.
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
    // Logged by `exports::start`, in the transaction that records the export.
    let export_id = crate::exports::start(&state, &job, admin.0.id).await?;

    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "id": export_id, "state": "running" })),
    ))
}

/// The newest export for a job, with a download URL once it is ready, and
/// whether it is the final corpus or a snapshot.
async fn get_export(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<ExportDetail>> {
    let mut export = sqlx::query_as::<_, ExportRow>(
        "SELECT id, state, bytes, sha256, row_count, positions_bytes, positions_sha256,
                positions_row_count, is_final, snapshot_at, error, requested_at, completed_at
         FROM job_exports WHERE job_id = $1
         ORDER BY requested_at DESC LIMIT 1",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::not_found("this job has never been exported"))?;

    // This export's own objects, snapshot or final: the stream serves only a
    // final one, but the admin page offers whichever it shows, labelled.
    let (download_url, positions_download_url) =
        match crate::exports::ready_objects(&state.pool, export.id).await? {
            Some(ready) => {
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
/// `GET /api/admin/derived-data`, and the next claim that considers the job
/// queues what has no row (`derived::ready_for_job`). Failing the
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
    // Which files it is built from: two rows can share a role, a name and a
    // builder -- a lexicon re-released under its name, two distributions
    // with one name -- and without these the page could not tell them apart,
    // nor Retry say which it meant.
    kwg_id: Uuid,
    klv_id: Option<Uuid>,
    letterdist_id: Uuid,
    /// The same, as an admin reads them: each file's path and the tarball it
    /// was first imported from.
    made_from: String,
    /// Whether a builder of this MAGPIE takes the row. One queued under
    /// another version is never built here, so retrying it only leaves it
    /// pending for good.
    buildable: bool,
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
            "SELECT d.role, d.name, d.builder, d.kwg_id, d.klv_id, d.letterdist_id,
                    concat_ws(', ', k.path || ' (' || k.tarball_date || ')',
                                    v.path || ' (' || v.tarball_date || ')',
                                    l.path || ' (' || l.tarball_date || ')') AS made_from,
                    d.builder = CASE d.role WHEN 'wmp' THEN $1 WHEN 'rit' THEN $2
                                            WHEN 'wit' THEN $3 END AS buildable,
                    d.state, d.sha256, d.bytes, d.build_target, d.error, d.attempts,
                    d.requested_at, d.built_at
             FROM derived_data d
             JOIN input_data k ON k.id = d.kwg_id
             LEFT JOIN input_data v ON v.id = d.klv_id
             JOIN input_data l ON l.id = d.letterdist_id
             ORDER BY d.state = 'built', d.requested_at DESC",
        )
        .bind(state.builders.wmp())
        .bind(state.builders.rit())
        .bind(state.builders.wit())
        .fetch_all(&state.pool)
        .await?,
    ))
}

#[derive(Deserialize)]
struct RetryDerivedBody {
    role: String,
    name: String,
    /// The one row meant, as the list gives it; all but `klv_id` (null for a
    /// wordmap) are required.
    builder: Option<String>,
    kwg_id: Option<Uuid>,
    klv_id: Option<Uuid>,
    letterdist_id: Option<Uuid>,
}

/// Puts a failed build back in the queue.
///
/// Explicit, because a build is a pure function of its inputs: one that failed
/// three times failed for a reason that a fourth attempt does not change, and
/// re-queueing it automatically would spend every builder run on the same
/// doomed row. An admin retries it after fixing what it named -- most often a
/// lexicon's bytes missing from the object store, which re-importing its
/// tarball uploads again.
async fn retry_derived_data(
    State(state): State<AppState>,
    admin: AdminUser,
    method: Method,
    headers: HeaderMap,
    jar: CookieJar,
    ApiJson(body): ApiJson<RetryDerivedBody>,
) -> AppResult<StatusCode> {
    csrf::verify(&method, &headers, &jar)?;
    // The row, whole: by role and name alone every failed row of that name
    // was reset, other builders' included, which no builder of this MAGPIE
    // then takes; and a partial set matched nothing and was answered "no
    // failed build". `klv_id` is null for a wordmap.
    let (Some(builder), Some(kwg_id), Some(letterdist_id)) =
        (&body.builder, body.kwg_id, body.letterdist_id)
    else {
        return Err(AppError::bad_request(
            "name the build by role, name, builder, kwg_id, klv_id and letterdist_id, as the list gives them",
        ));
    };
    // The reset and its audit row in one transaction: written after, a failed
    // insert left a build reset with no record of who reset it.
    let mut tx = state.pool.begin().await?;
    let reset = sqlx::query(
        "UPDATE derived_data
         SET state = 'pending', attempts = 0, error = NULL, leased_until = NULL
         WHERE role = $1 AND name = $2 AND state = 'failed' AND builder = $3
           AND kwg_id = $4 AND klv_id IS NOT DISTINCT FROM $5 AND letterdist_id = $6
           -- A builder this version has: retried, any other row sat pending
           -- for good.
           AND builder = CASE role WHEN 'wmp' THEN $7 WHEN 'rit' THEN $8 WHEN 'wit' THEN $9 END",
    )
    .bind(&body.role)
    .bind(&body.name)
    .bind(builder)
    .bind(kwg_id)
    .bind(body.klv_id)
    .bind(letterdist_id)
    .bind(state.builders.wmp())
    .bind(state.builders.rit())
    .bind(state.builders.wit())
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if reset == 0 {
        return Err(AppError::not_found(
            "no failed build of that row that a builder of this version takes \
             (retried already, or queued for another version)",
        ));
    }
    audit::log(
        &mut tx,
        "derived_data.retried",
        Some(admin.0.id),
        None,
        Some("derived_data"),
        Some(format!(
            "{} {} {builder} kwg={kwg_id} klv={} letterdist={letterdist_id}",
            body.role,
            body.name,
            body.klv_id.map_or("none".to_string(), |k| k.to_string()),
        )),
        None,
    )
    .await?;
    tx.commit().await?;
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
            "deactivate the job (0% on the allocation page) before forcing a rebuild: workers \
             mid-task would refuse the rewritten objects",
        ));
    }
    let job_data = crate::jobs::load_job_data(&mut conn, job.id).await?;
    // Logged before the first object is rewritten: a rebuild writes one
    // generation at a time, and one that stopped part-way -- an S3 error, a
    // failed build, the load balancer's timeout past about twenty generations
    // (KL-19) -- had replaced objects workers played and written no row at
    // all, the one below being reached only at the end.
    audit::log_detail(
        &mut conn,
        "job.artifacts_rebuild_started",
        admin.0.id,
        "job",
        job.id.to_string(),
        Some(job.id),
        format!("force={}", query.force),
    )
    .await?;
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
/// counter is rolled back, so no donated compute is lost. Open claims are left
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

    // The account is locked first, then counted, then its own rows go, then it
    // is anonymized: a password reset locks the account before its token too.
    // Taken the other way round, a reset and a delete of one account
    // deadlocked, and the delete lost; counted before the lock, a key or code
    // made in between was destroyed uncounted.
    sqlx::query("SELECT id FROM users WHERE id = $1 FOR UPDATE")
        .bind(id)
        .execute(&mut *tx)
        .await?;

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
    for table in ["api_keys", "email_confirmations", "password_reset_tokens"] {
        sqlx::query(&format!("DELETE FROM {table} WHERE user_id = $1"))
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
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

/// The longest ban reason accepted.
const MAX_BAN_REASON_CHARS: usize = 1_000;

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
    // Stored twice (the ban and its audit row) and shown on the admin page: a
    // sentence, not a document; and no NUL, which Postgres refuses as a `500`
    // (the audit's pass 22).
    if let Some(reason) = body.reason.as_deref() {
        if reason.chars().count() > MAX_BAN_REASON_CHARS {
            return Err(AppError::bad_request("the reason is too long")
                .with_field("reason", format!("at most {MAX_BAN_REASON_CHARS} characters")));
        }
        if reason.contains('\0') {
            return Err(AppError::bad_request("the reason holds a NUL character")
                .with_field("reason", "no NUL characters"));
        }
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

/// On the display pool: a filtered page and its count are sequential scans of
/// the log (KL-32), so they take its statement timeout and stay off the
/// connections claims and submissions use.
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
    .fetch_all(&state.read_pool)
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
    .fetch_one(&state.read_pool)
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

    /// I-OR-EDIT-6: an action that waited on a job's row is told a purge
    /// happened only when one did. A consensus edit's hold, which is not
    /// counted, is refused as running: it read "the job was purged".
    #[test]
    fn an_action_that_waited_on_an_edit_is_not_told_the_job_was_purged() {
        let holds = crate::jobs::DispatchHolds::default();
        let job = Uuid::new_v4();
        let grace = std::time::Duration::from_secs(1);
        let taken = holds.claims_holds_taken(job);
        assert!(refuse_if_purged_since(&holds, job, taken).is_ok());

        let edit = holds.try_hold_claims_uncounted(job, grace).unwrap();
        let err = refuse_if_purged_since(&holds, job, taken).unwrap_err();
        assert_eq!((err.status, err.message.as_str()), (StatusCode::CONFLICT, ALREADY_RUNNING));
        drop(edit);
        assert!(refuse_if_purged_since(&holds, job, taken).is_ok());

        let purge = holds.try_hold_claims(job, grace).unwrap();
        let err = refuse_if_purged_since(&holds, job, taken).unwrap_err();
        assert_eq!(err.message, PURGED_WHILE_WAITING);
        drop(purge);
        let err = refuse_if_purged_since(&holds, job, taken).unwrap_err();
        assert_eq!(err.message, PURGED_WHILE_WAITING, "by count, after the hold is gone");
    }

    /// Nothing MAGPIE's loader (`board_layout.c`) refuses is accepted, and what
    /// it loads is too, but for parser quirks birdtest is stricter about (e.g. a
    /// start coordinate past `int` or padded with a vertical tab, a
    /// whitespace-only coordinate, a NUL, a byte above 0x7f). Counting rows
    /// alone accepted six layouts MAGPIE refuses and refused two it loads.
    #[test]
    fn a_layout_magpie_would_refuse_is_refused() {
        let standard15 = include_str!("../../../fixtures/versions/20260101/layouts/standard15.txt");
        assert_eq!(layout_problem(standard15.as_bytes()), None);
        assert_eq!(layout_problem(standard15.replace('\n', "\r\n").as_bytes()), None, "CRLF");
        assert_eq!(layout_problem(standard15.trim_end().as_bytes()), None, "no final newline");
        let blank_inside = standard15.replacen('\n', "\n\n", 3);
        assert_eq!(layout_problem(blank_inside.as_bytes()), None, "blank lines are skipped");

        let mut super21 = String::from("10, 10\n");
        for _ in 0..21 {
            super21.push_str(&" ".repeat(21));
            super21.push('\n');
        }
        let refused = |text: &str, why: &str| {
            assert!(layout_problem(text.as_bytes()).is_some(), "{why} should be refused");
        };
        refused(&super21, "a 21x21 board");
        refused("\n\n", "an empty file");
        let body = standard15.trim_end();
        refused(&format!("{body}\n   \n"), "a trailing line of spaces");
        refused(&format!("{body}\n\r\n"), "a trailing CRLF blank line");
        refused(&standard15.replacen("7, 7", "20, 20", 1), "a start square off the board");
        refused(&standard15.replacen("7, 7", "7", 1), "a start square with one coordinate");
        let rows: Vec<&str> = standard15.lines().collect();
        let narrow = format!("{}\n{}\n{}", rows[0], &rows[1][..14], rows[2..].join("\n"));
        refused(&narrow, "a row 14 squares wide");
        let odd = standard15.replacen('=', "x", 1);
        refused(&odd, "an unknown square");
    }

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
            "player_config_ids": [Uuid::nil()],
            "test_enabled": true,
            "min_pairs": 100,
            "max_pairs": 1000,
        });
        config.as_object_mut().unwrap().extend(overrides.as_object().unwrap().clone());
        body(config)
    }

    fn fields(result: AppResult<()>) -> Vec<String> {
        result.expect_err("should be rejected").fields.into_iter().map(|(f, _)| f).collect()
    }

    /// A games or pairs request names one config (self-play) or a round robin
    /// of up to twelve, none twice; nothing else.
    #[test]
    fn a_round_robin_names_one_to_twelve_configs_once_each() {
        let with = |ids: Vec<Uuid>| game_pairs(serde_json::json!({ "player_config_ids": ids }));
        let ids: Vec<Uuid> = (0..13).map(|_| Uuid::new_v4()).collect();
        for n in [1, 2, 4, 12] {
            assert!(validate_job_body(&with(ids[..n].to_vec())).is_ok(), "{n} configs");
        }
        for refused in [Vec::new(), ids.clone(), vec![ids[0], ids[1], ids[0]]] {
            assert_eq!(fields(validate_job_body(&with(refused.clone()))), ["player_config_ids"], "{refused:?}");
        }
    }

    /// One config plays itself; n ≥ 2 give C(n, 2) pairings, each once, each
    /// seated in the order the configs were listed.
    #[test]
    fn pairings_are_every_pair_once_in_the_order_given() {
        let ids: Vec<Uuid> = (0..4).map(|_| Uuid::new_v4()).collect();
        let pairs = |n: usize| pairings(&game_pairs(serde_json::json!({ "player_config_ids": ids[..n] }))
            .config);
        assert_eq!(pairs(1), [(ids[0], ids[0])]);
        assert_eq!(pairs(2), [(ids[0], ids[1])]);
        assert_eq!(pairs(3), [(ids[0], ids[1]), (ids[0], ids[2]), (ids[1], ids[2])]);
        let four = pairs(4);
        assert_eq!(four.len(), 6);
        assert_eq!(
            four,
            [
                (ids[0], ids[1]), (ids[0], ids[2]), (ids[0], ids[3]),
                (ids[1], ids[2]), (ids[1], ids[3]), (ids[2], ids[3]),
            ]
        );
        let opening = body(serde_json::json!({
            "job_type": "opening_rack", "player_config_id": ids[0],
        }));
        assert!(pairings(&opening.config).is_empty(), "only games and pairs are paired");
    }

    #[test]
    fn ordinary_settings_are_accepted() {
        assert!(validate_job_body(&game_pairs(serde_json::json!({}))).is_ok());
    }

    /// A games or pairs job without a word about the test runs none: it
    /// needs only its target, and stores the defaults, which nothing reads.
    #[test]
    fn a_job_runs_no_test_unless_it_asks_for_one() {
        let target_only = |job_type: &str, target: &str| {
            let mut config = serde_json::json!({
                "job_type": job_type,
                "player_config_ids": [Uuid::nil()],
            });
            config[target] = serde_json::json!(500);
            body(config)
        };
        for (job_type, target) in [("games", "max_games"), ("game_pairs", "max_pairs")] {
            let body = target_only(job_type, target);
            assert!(validate_job_body(&body).is_ok(), "{job_type}");
            let (JobTypeConfig::Game { min_games: min, test, .. }
            | JobTypeConfig::GamePair { min_pairs: min, test, .. }) = &body.config
            else {
                panic!("{job_type} read as another job type");
            };
            let stored = test.settings(*min);
            assert!(!stored.enabled, "{job_type}");
            assert_eq!((stored.min_units, stored.confidence_pct), (0, 95.0));
        }
    }

    /// Settings for a test that is off are refused, each by name, rather than
    /// dropped: the job would otherwise play to its cap with no test and no
    /// word said.
    #[test]
    fn test_settings_without_the_test_are_refused() {
        let off = serde_json::json!({
            "test_enabled": false, "min_pairs": 100, "confidence_pct": 99.0
        });
        assert_eq!(fields(validate_job_body(&game_pairs(off))), ["min_pairs", "confidence_pct"]);
        // Left out, the flag is off too: a pre-flag script's body.
        let unflagged = body(serde_json::json!({
            "job_type": "game_pairs",
            "player_config_ids": [Uuid::nil()],
            "min_pairs": 100,
            "max_pairs": 1000,
        }));
        assert_eq!(fields(validate_job_body(&unflagged)), ["min_pairs"]);
    }

    /// With the test on, its floor is stated, at least 1 and at most the cap:
    /// the interval is asymptotic, and holds once enough units are in.
    #[test]
    fn the_test_needs_its_floor() {
        let mut config = serde_json::json!({
            "job_type": "game_pairs",
            "player_config_ids": [Uuid::nil()],
            "test_enabled": true,
            "max_pairs": 1000,
        });
        assert_eq!(fields(validate_job_body(&body(config.clone()))), ["min_pairs"]);
        config["min_pairs"] = serde_json::json!(0);
        assert_eq!(fields(validate_job_body(&body(config.clone()))), ["min_pairs"]);
        config["min_pairs"] = serde_json::json!(1001);
        assert_eq!(fields(validate_job_body(&body(config.clone()))), ["min_pairs"]);
        config["min_pairs"] = serde_json::json!(1000);
        assert!(validate_job_body(&body(config)).is_ok());
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

    /// An odd games batch gives player 1 the first move more often in every
    /// task; at the old default of 1, in every game. An even one is balanced.
    #[test]
    fn a_games_batch_must_be_even() {
        let games = |batch: serde_json::Value| {
            let mut config = serde_json::json!({
                "job_type": "games",
                "player_config_ids": [Uuid::nil()],
                "test_enabled": true,
                "min_games": 100,
                "max_games": 1000,
            });
            if !batch.is_null() {
                config["games_per_batch"] = batch;
            }
            body(config)
        };
        for odd in [1, 3, 7] {
            assert_eq!(fields(validate_job_body(&games(serde_json::json!(odd)))), ["games_per_batch"]);
        }
        assert!(validate_job_body(&games(serde_json::json!(2))).is_ok());
        assert!(validate_job_body(&games(serde_json::json!(10))).is_ok());
        // The default is even.
        assert!(validate_job_body(&games(serde_json::Value::Null)).is_ok());
    }

    /// A batch no result could carry is refused at creation: past 32,768
    /// captured games the game index overflows, and a thousand captured games
    /// is already a large body.
    #[test]
    fn a_games_batch_is_bounded() {
        let games = |batch: i32, capture: bool| {
            body(serde_json::json!({
                "job_type": "games",
                "player_config_ids": [Uuid::nil()],
                "test_enabled": true,
                "min_games": 100,
                "max_games": 100_000,
                "games_per_batch": batch,
                "capture_positions": capture,
            }))
        };
        assert!(validate_job_body(&games(10_000, false)).is_ok());
        assert_eq!(fields(validate_job_body(&games(10_002, false))), ["games_per_batch"]);
        assert!(validate_job_body(&games(1_000, true)).is_ok());
        assert_eq!(fields(validate_job_body(&games(1_002, true))), ["games_per_batch"]);
        assert_eq!(fields(validate_job_body(&games(40_000, true))), ["games_per_batch"]);
        let pairs = |batch: i32, capture: bool| {
            game_pairs(serde_json::json!({ "pairs_per_batch": batch, "capture_positions": capture }))
        };
        assert!(validate_job_body(&pairs(500, true)).is_ok());
        assert_eq!(fields(validate_job_body(&pairs(501, true))), ["pairs_per_batch"]);
        assert!(validate_job_body(&pairs(5_000, false)).is_ok());
        assert_eq!(fields(validate_job_body(&pairs(5_001, false))), ["pairs_per_batch"]);
    }

    /// A confidence of 100% never closes the interval, and one of half or
    /// less is no test; everything strictly between is a test.
    #[test]
    fn a_confidence_outside_half_to_all_is_refused() {
        for bad in [100.0, 50.0, 0.0, -5.0, 101.0] {
            assert_eq!(
                fields(validate_job_body(&game_pairs(serde_json::json!({ "confidence_pct": bad })))),
                ["confidence_pct"],
                "{bad}"
            );
        }
        for good in [50.5, 80.0, 95.0, 99.9] {
            assert!(validate_job_body(&game_pairs(serde_json::json!({ "confidence_pct": good }))).is_ok());
        }
    }

    #[test]
    fn every_problem_is_reported() {
        let got = fields(validate_job_body(&game_pairs(serde_json::json!({
            "confidence_pct": 100.0, "max_pairs": 0, "pairs_per_batch": 0
        }))));
        for expected in ["confidence_pct", "max_pairs", "pairs_per_batch"] {
            assert!(got.iter().any(|f| f == expected), "missing {expected} in {got:?}");
        }
    }

    /// A leave body names a `player_config_id`, as an opening-rack body does,
    /// and an untagged enum takes the first variant a body fits: read as an
    /// opening-rack config it would pass as one on its defaults, and fail as a
    /// type mismatch after its settings had gone unchecked.
    #[test]
    fn a_leave_body_is_read_as_a_leave_config_and_an_opening_rack_body_is_not() {
        let leave = body(serde_json::json!({
            "job_type": "leave_generation",
            "player_config_id": Uuid::nil(),
            "num_iterations": 10,
            "target_rack_counts": [10],
            "racks_per_task": 10,
        }));
        assert!(matches!(leave.config, JobTypeConfig::Leave { .. }));
        let racks = body(serde_json::json!({
            "job_type": "opening_rack",
            "player_config_id": Uuid::nil(),
        }));
        assert!(matches!(racks.config, JobTypeConfig::OpeningRack { .. }));
    }

    #[test]
    fn leave_generation_bounds_are_enforced() {
        let leave = body(serde_json::json!({
            "job_type": "leave_generation",
            "player_config_id": Uuid::nil(),
            "num_iterations": 0,
            "target_rack_counts": [10],
            "racks_per_task": 0,
        }));
        assert_eq!(fields(validate_job_body(&leave)), ["num_iterations", "racks_per_task"]);
        let mut leave = serde_json::json!({
            "job_type": "leave_generation",
            "player_config_id": Uuid::nil(),
            "num_iterations": 10,
            "target_rack_counts": [10],
            "racks_per_task": MAX_RACKS_PER_TASK,
        });
        assert!(validate_job_body(&body(leave.clone())).is_ok());
        leave["racks_per_task"] = serde_json::json!(MAX_RACKS_PER_TASK + 1);
        assert_eq!(fields(validate_job_body(&body(leave))), ["racks_per_task"]);
    }

    /// An opening rack holds at least a tile and at most the full rack every
    /// build is held to; the bound is the one constant, not a second seven.
    #[test]
    fn an_opening_rack_is_one_tile_to_a_full_rack() {
        let racks = |rack_size: i32| {
            body(serde_json::json!({
                "job_type": "opening_rack",
                "player_config_id": Uuid::nil(),
                "rack_size": rack_size,
            }))
        };
        assert!(validate_job_body(&racks(1)).is_ok());
        assert!(validate_job_body(&racks(RACK_SIZE as i32)).is_ok());
        assert_eq!(fields(validate_job_body(&racks(0))), ["rack_size"]);
        assert_eq!(fields(validate_job_body(&racks(RACK_SIZE as i32 + 1))), ["rack_size"]);
    }

    #[test]
    fn a_leave_job_lists_between_one_and_the_most_generations_each_with_a_sane_target() {
        let with_targets = |targets: serde_json::Value| {
            body(serde_json::json!({
                "job_type": "leave_generation",
                "player_config_id": Uuid::nil(),
                "num_iterations": 10,
                "target_rack_counts": targets,
                "racks_per_task": 10,
            }))
        };
        assert!(validate_job_body(&with_targets(serde_json::json!([100, 200, 500, 1000]))).is_ok());
        let most = vec![MAX_TARGET_RACK_COUNT; MAX_LEAVE_GENERATIONS];
        assert!(validate_job_body(&with_targets(serde_json::json!(most))).is_ok());
        for bad in [
            serde_json::json!([]),
            serde_json::json!([100, 0, 500]),
            serde_json::json!([-5]),
            serde_json::json!([MAX_TARGET_RACK_COUNT + 1]),
            serde_json::json!(vec![10; MAX_LEAVE_GENERATIONS + 1]),
        ] {
            assert_eq!(
                fields(validate_job_body(&with_targets(bad.clone()))),
                ["target_rack_counts"],
                "{bad}"
            );
        }
    }

    #[test]
    fn an_unknown_variant_is_rejected() {
        let mut job = game_pairs(serde_json::json!({}));
        job.variant = "scrabble-but-different".into();
        assert_eq!(fields(validate_job_body(&job)), ["variant"]);
    }
}
