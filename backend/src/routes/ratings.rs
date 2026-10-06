//! Rating pool endpoints: public reads of the latest fit, admin writes to
//! membership.
//!
//! Split across two routers (mounted under `/api` and `/api/admin`) because the
//! read side is public — ratings are the point of the exercise — while who is
//! *in* a pool is an admin decision.

use crate::audit;
use crate::auth::{csrf, AdminUser};
use crate::extract::ApiJson;
use crate::error::{AppError, AppResult};
use crate::ratings::{self, Trigger};
use crate::state::AppState;
use crate::extract::ApiPath as Path;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode};
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use axum_extra::extract::CookieJar;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

pub fn public_router() -> Router<AppState> {
    Router::new()
        .route("/rating-pools", get(list_pools))
        .route("/rating-pools/:id", get(pool_detail))
        .route("/rating-pools/:id/history", get(pool_history))
}

pub fn admin_router() -> Router<AppState> {
    Router::new()
        .route("/rating-pools", post(create_pool))
        .route("/rating-pools/:id", patch(update_pool).delete(delete_pool))
        .route("/rating-pools/:id/members", post(add_member))
        .route("/rating-pools/:id/members/:config_id", delete(remove_member))
        .route("/rating-pools/:id/recompute", post(recompute_pool))
}

// ---------------------------------------------------------------------------
// Reads
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct PoolListItem {
    id: Uuid,
    name: String,
    variant: String,
    letter_distribution: String,
    layout: String,
    members: i64,
    last_computed_at: Option<chrono::DateTime<chrono::Utc>>,
}

async fn list_pools(State(state): State<AppState>) -> AppResult<Json<Vec<PoolListItem>>> {
    let rows = sqlx::query(
        "SELECT p.id, p.name, p.variant, ld.name AS letterdist, lay.name AS layout,
                (SELECT COUNT(*) FROM rating_pool_members m WHERE m.pool_id = p.id) AS members,
                (SELECT MAX(r.computed_at) FROM rating_runs r WHERE r.pool_id = p.id) AS last_computed_at
         FROM rating_pools p
         JOIN input_data ld  ON ld.id = p.letterdist_id
         JOIN input_data lay ON lay.id = p.layout_id
         ORDER BY p.name",
    )
    .fetch_all(&state.read_pool)
    .await?;

    Ok(Json(
        rows.iter()
            .map(|row| PoolListItem {
                id: row.get("id"),
                name: row.get("name"),
                variant: row.get("variant"),
                letter_distribution: row.get("letterdist"),
                layout: row.get("layout"),
                members: row.get::<Option<i64>, _>("members").unwrap_or(0),
                last_computed_at: row.get("last_computed_at"),
            })
            .collect(),
    ))
}

#[derive(Serialize)]
struct RatingRow {
    player_config_id: Uuid,
    name: String,
    rating: f64,
    stderr: f64,
    pairs_played: i64,
    /// False when no chain of games links this config to the anchor, in which
    /// case `rating` is an artefact of the fit's prior and the page must show
    /// it as unrated rather than as a number.
    connected_to_anchor: bool,
    is_anchor: bool,
}

/// One head-to-head, with what the ratings predict against what happened.
#[derive(Serialize)]
struct MatrixCell {
    row: Uuid,
    col: Uuid,
    pairs: f64,
    actual: f64,
    predicted: f64,
}

#[derive(Serialize)]
struct RunSummary {
    id: Uuid,
    computed_at: chrono::DateTime<chrono::Utc>,
    trigger: String,
    iterations: i32,
    converged: bool,
    pairs_used: i64,
    jobs_used: i32,
}

/// A config in the pool now, rated or not.
#[derive(Serialize)]
struct PoolMember {
    player_config_id: Uuid,
    name: String,
}

#[derive(Serialize)]
struct PoolDetail {
    id: Uuid,
    name: String,
    variant: String,
    letter_distribution: String,
    layout: String,
    anchor_player_config_id: Uuid,
    anchor_rating: f64,
    /// Who is in the pool now, by name. Not the same set as `ratings`, which
    /// is the latest fit's: a config added since (or whose refit failed) is a
    /// member with no rating yet, and one removed since is rated but no longer
    /// a member. The page's membership controls work from this list.
    members: Vec<PoolMember>,
    run: Option<RunSummary>,
    ratings: Vec<RatingRow>,
    /// Actual-versus-predicted for every head-to-head, worst first. This is
    /// where non-transitivity becomes visible: no single rating per player can
    /// reproduce an A-beats-B-beats-C-beats-A triangle, so the model's failure
    /// shows up here as large residuals rather than silently distorting the
    /// ratings.
    residuals: Vec<MatrixCell>,
}

async fn pool_detail(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<PoolDetail>> {
    let pool_row = sqlx::query(
        "SELECT p.id, p.name, p.variant, p.anchor_player_config_id, p.anchor_rating,
                ld.name AS letterdist, lay.name AS layout
         FROM rating_pools p
         JOIN input_data ld  ON ld.id = p.letterdist_id
         JOIN input_data lay ON lay.id = p.layout_id
         WHERE p.id = $1",
    )
    .bind(id)
    .fetch_optional(&state.read_pool)
    .await?
    .ok_or_else(|| AppError::not_found("rating pool not found"))?;

    let run = sqlx::query(
        "SELECT id, computed_at, trigger, iterations, converged, pairs_used, jobs_used
         FROM rating_runs WHERE pool_id = $1 ORDER BY computed_at DESC, id DESC LIMIT 1",
    )
    .bind(id)
    .fetch_optional(&state.read_pool)
    .await?
    .map(|row| RunSummary {
        id: row.get("id"),
        computed_at: row.get("computed_at"),
        trigger: row.get("trigger"),
        iterations: row.get("iterations"),
        converged: row.get("converged"),
        pairs_used: row.get("pairs_used"),
        jobs_used: row.get("jobs_used"),
    });

    let members = sqlx::query(
        "SELECT m.player_config_id, c.name
         FROM rating_pool_members m
         JOIN player_configs c ON c.id = m.player_config_id
         WHERE m.pool_id = $1
         ORDER BY c.name, m.player_config_id",
    )
    .bind(id)
    .fetch_all(&state.read_pool)
    .await?
    .iter()
    .map(|row| PoolMember { player_config_id: row.get("player_config_id"), name: row.get("name") })
    .collect();

    let mut ratings = Vec::new();
    if let Some(run) = run.as_ref() {
        let rows = sqlx::query(
            "SELECT r.player_config_id, c.name, r.rating, r.stderr, r.pairs_played,
                    r.connected_to_anchor, r.is_anchor
             FROM player_config_ratings r
             JOIN player_configs c ON c.id = r.player_config_id
             WHERE r.run_id = $1
             ORDER BY r.rating DESC",
        )
        .bind(run.id)
        .fetch_all(&state.read_pool)
        .await?;
        ratings = rows
            .iter()
            .map(|row| RatingRow {
                player_config_id: row.get("player_config_id"),
                name: row.get("name"),
                rating: row.get("rating"),
                stderr: row.get("stderr"),
                pairs_played: row.get("pairs_played"),
                connected_to_anchor: row.get("connected_to_anchor"),
                is_anchor: row.get("is_anchor"),
            })
            .collect();
    }

    // Read from the run, not recomputed: recomputing rebuilt the pool's
    // evidence matrix -- a grouped scan over every paired result it counts --
    // on every view of a public, unauthenticated page, holding a connection
    // from the pool claims and submissions share. A fit stores them in the
    // same transaction as its ratings, so the two cannot disagree.
    let residuals = match run.as_ref() {
        Some(run) => sqlx::query(
            "SELECT row_player_config_id, col_player_config_id, pairs, actual, predicted
             FROM rating_run_residuals
             WHERE run_id = $1
             ORDER BY abs(actual - predicted) DESC, row_player_config_id, col_player_config_id",
        )
        .bind(run.id)
        .fetch_all(&state.read_pool)
        .await?
        .iter()
        .map(|row| MatrixCell {
            row: row.get("row_player_config_id"),
            col: row.get("col_player_config_id"),
            pairs: row.get("pairs"),
            actual: row.get("actual"),
            predicted: row.get("predicted"),
        })
        .collect(),
        None => Vec::new(),
    };

    Ok(Json(PoolDetail {
        id: pool_row.get("id"),
        name: pool_row.get("name"),
        variant: pool_row.get("variant"),
        letter_distribution: pool_row.get("letterdist"),
        layout: pool_row.get("layout"),
        anchor_player_config_id: pool_row.get("anchor_player_config_id"),
        anchor_rating: pool_row.get("anchor_rating"),
        members,
        run,
        ratings,
        residuals,
    }))
}

#[derive(Serialize)]
struct HistoryPoint {
    computed_at: chrono::DateTime<chrono::Utc>,
    player_config_id: Uuid,
    name: String,
    rating: f64,
    stderr: f64,
}

/// The most runs one history response carries.
///
/// A pool with an active job is refit every two minutes -- 720 runs a day, each
/// with a row per member -- and this is a public page. Returning every run made
/// its cost and its payload grow for the life of the pool: a month of one
/// active job at ten members is over 200,000 points on every request. Sized for
/// the ratings page's chart, a few hundred pixels wide; no page draws the
/// history now, and the bound stays as the response's.
const MAX_HISTORY_RUNS: i64 = 500;

/// How many configs the history carries: the six series the ratings page's
/// chart drew, kept as the response's bound now that only API callers read it.
const HISTORY_CONFIGS: i64 = 6;

/// The pool's rating history, oldest first. Snapshots per run rather than a
/// mutated current value are what make this possible at all. API-only: the
/// ratings page drew it as a chart until the chart was removed.
///
/// Thinned to at most [`MAX_HISTORY_RUNS`] runs, evenly spaced over the pool's
/// whole history, with the first and the newest always kept -- so a series
/// still starts where the pool started and ends at the rating the page shows.
///
/// Only the [`HISTORY_CONFIGS`] current members rated highest in the newest
/// run. Every member's points went out on every view of what was then a
/// public page -- 9.5 MB at 100 members, a second of the display
/// pool's time, and forty at once answered `503` to other readers (the audit's
/// pass 25) -- and a removed config could take one of the six places.
async fn pool_history(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Vec<HistoryPoint>>> {
    // The kept runs, by themselves: a plain query on the pool's runs.
    let runs: Vec<(Uuid, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "WITH runs AS (
             SELECT id, computed_at,
                    row_number() OVER (ORDER BY computed_at, id) AS n,
                    count(*) OVER () AS total
             FROM rating_runs WHERE pool_id = $1
         )
         SELECT id, computed_at FROM runs
         WHERE total <= $2
            OR (n - 1) % ((total + $2 - 1) / $2) = 0
            OR n = total
         ORDER BY n",
    )
    .bind(id)
    .bind(MAX_HISTORY_RUNS)
    .fetch_all(&state.read_pool)
    .await?;
    let Some(&(newest, _)) = runs.last() else {
        return Ok(Json(Vec::new()));
    };
    let shown: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT r.player_config_id, c.name
         FROM player_config_ratings r
         JOIN player_configs c ON c.id = r.player_config_id
         JOIN rating_pool_members m ON m.pool_id = $2 AND m.player_config_id = r.player_config_id
         WHERE r.run_id = $1 AND r.connected_to_anchor
         ORDER BY r.rating DESC, r.player_config_id
         LIMIT $3",
    )
    .bind(newest)
    .bind(id)
    .bind(HISTORY_CONFIGS)
    .fetch_all(&state.read_pool)
    .await?;
    // Each (run, config) by the ratings' primary key: at most 501 × 6 rows.
    let run_ids: Vec<Uuid> = runs.iter().map(|(run, _)| *run).collect();
    let config_ids: Vec<Uuid> = shown.iter().map(|(config, _)| *config).collect();
    let rows: Vec<(Uuid, Uuid, f64, f64)> = sqlx::query_as(
        "SELECT run_id, player_config_id, rating, stderr
         FROM player_config_ratings
         WHERE run_id = ANY($1) AND player_config_id = ANY($2) AND connected_to_anchor",
    )
    .bind(&run_ids)
    .bind(&config_ids)
    .fetch_all(&state.read_pool)
    .await?;

    let at: std::collections::HashMap<Uuid, chrono::DateTime<chrono::Utc>> = runs.into_iter().collect();
    let names: std::collections::HashMap<Uuid, String> = shown.into_iter().collect();
    let mut points: Vec<HistoryPoint> = rows
        .into_iter()
        .map(|(run, config, rating, stderr)| HistoryPoint {
            computed_at: at[&run],
            player_config_id: config,
            name: names[&config].clone(),
            rating,
            stderr,
        })
        .collect();
    points.sort_by(|a, b| a.computed_at.cmp(&b.computed_at).then_with(|| a.name.cmp(&b.name)));
    Ok(Json(points))
}

// ---------------------------------------------------------------------------
// Admin writes
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct CreatePoolBody {
    name: String,
    variant: String,
    letterdist_id: Uuid,
    layout_id: Uuid,
    /// The config pinned at `anchor_rating`. Ratings are identifiable only up
    /// to an additive constant, so a pool without one has no scale.
    anchor_player_config_id: Uuid,
    #[serde(default = "default_anchor_rating")]
    anchor_rating: f64,
}

fn default_anchor_rating() -> f64 {
    2000.0
}

/// How far from zero an anchor may be pinned. Ratings are a logistic scale
/// (`10^(r/400)`), which overflows a double a little past ±123,000 and turned
/// every rating in the pool into ±inf -- stored, serialized as `null`, and the
/// pool's page broke on it. Nothing plausible is anywhere near this.
const MAX_ABS_ANCHOR_RATING: f64 = 10_000.0;

async fn create_pool(
    State(state): State<AppState>,
    admin: AdminUser,
    method: Method,
    headers: HeaderMap,
    jar: CookieJar,
    ApiJson(body): ApiJson<CreatePoolBody>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    csrf::verify(&method, &headers, &jar)?;

    // Validated the way a job is: a pool's scope is compared with its jobs'
    // (variant, distribution, layout), so a pool scoped to a variant no job
    // can have, or to a distribution row that is really a layout, matches
    // nothing and rates no one, silently.
    let mut err = AppError::bad_request("rating pool details are invalid");
    if body.name.trim().is_empty() {
        err = err.with_field("name", "must not be empty");
    } else if let Some(problem) = super::admin::name_problem(body.name.trim()) {
        err = err.with_field("name", problem);
    }
    if !matches!(body.variant.as_str(), "classic" | "wordsmog") {
        err = err.with_field("variant", "must be 'classic' or 'wordsmog'");
    }
    if !body.anchor_rating.is_finite() || body.anchor_rating.abs() > MAX_ABS_ANCHOR_RATING {
        err = err.with_field(
            "anchor_rating",
            format!("must be a number between -{MAX_ABS_ANCHOR_RATING} and {MAX_ABS_ANCHOR_RATING}"),
        );
    }
    if !err.fields.is_empty() {
        return Err(err);
    }
    let letterdist_name = super::admin::require_role(&state.pool, body.letterdist_id, "letterdist").await?;
    super::admin::require_role(&state.pool, body.layout_id, "layout").await?;
    // Nor a distribution no job can be created on (`create_job` parses it).
    let letterdist: Vec<u8> = sqlx::query_scalar("SELECT content FROM input_data WHERE id = $1")
        .bind(body.letterdist_id)
        .fetch_one(&state.pool)
        .await?;
    crate::jobs::racks::LetterDistribution::parse(&letterdist, &letterdist_name).map_err(|e| {
        AppError::bad_request("no job can be created on that letter distribution")
            .with_field("letterdist_id", e.message)
    })?;
    // A board no job can be created on is a pool no job will ever match: it
    // would rate no one, silently.
    let layout: Vec<u8> = sqlx::query_scalar("SELECT content FROM input_data WHERE id = $1")
        .bind(body.layout_id)
        .fetch_one(&state.pool)
        .await?;
    if let Some(problem) = super::admin::layout_problem(&layout) {
        return Err(AppError::bad_request("no job can be created on that board layout")
            .with_field("layout_id", problem));
    }

    let mut tx = state.pool.begin().await?;
    let pool_id: Uuid = sqlx::query_scalar(
        "INSERT INTO rating_pools
             (name, variant, letterdist_id, layout_id, anchor_player_config_id, anchor_rating)
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
    )
    // Trimmed, as a job's and a player config's are: stored as typed, "X"
    // and "X " were two pools to the unique index and one to a reader.
    .bind(body.name.trim())
    .bind(&body.variant)
    .bind(body.letterdist_id)
    .bind(body.layout_id)
    .bind(body.anchor_player_config_id)
    .bind(body.anchor_rating)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| {
        unknown_config(
            e.into(),
            "rating_pools_anchor_player_config_id_fkey",
            "anchor_player_config_id",
            body.anchor_player_config_id,
        )
    })?;

    // The anchor is a member by construction: a pool whose fixed point is not
    // in the pool has nothing to fix.
    sqlx::query(
        "INSERT INTO rating_pool_members (pool_id, player_config_id, added_by)
         VALUES ($1, $2, $3)",
    )
    .bind(pool_id)
    .bind(body.anchor_player_config_id)
    .bind(admin.0.id)
    .execute(&mut *tx)
    .await?;

    audit::log(
        &mut tx,
        "rating_pool.created",
        Some(admin.0.id),
        None,
        Some("rating_pool"),
        Some(pool_id.to_string()),
        None,
    )
    .await?;
    tx.commit().await?;

    Ok((StatusCode::CREATED, Json(serde_json::json!({ "id": pool_id }))))
}

/// A foreign-key failure on `constraint`, a config reference, is the caller
/// naming a config that does not exist: a 400 on that field, not the generic
/// 409 ("still referenced by other records"), which says the opposite. Any
/// other failure -- another key of the same insert included -- is left as it
/// was.
pub(crate) fn unknown_config(err: AppError, constraint: &str, field: &str, id: Uuid) -> AppError {
    if err.db_code.as_deref() == Some(crate::error::FOREIGN_KEY_VIOLATION)
        && err.db_constraint.as_deref() == Some(constraint)
    {
        AppError::bad_request("that player config does not exist")
            .with_field(field, format!("no player config {id}"))
    } else {
        err
    }
}

#[derive(Deserialize)]
struct MemberBody {
    player_config_id: Uuid,
}

/// Adds a config and refits the pool; a config already a member is answered
/// `run_id: null`, neither logged nor refitted.
///
/// The refit is the whole point of the endpoint: a new member brings its games
/// in as evidence, which moves every other rating too, so there is no such
/// thing as adding one player without recomputing everyone.
async fn add_member(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(pool_id): Path<Uuid>,
    method: Method,
    headers: HeaderMap,
    jar: CookieJar,
    ApiJson(body): ApiJson<MemberBody>,
) -> AppResult<Json<serde_json::Value>> {
    csrf::verify(&method, &headers, &jar)?;

    let mut tx = state.pool.begin().await?;
    // Locked against a delete until this commits, so a pool seen here is still
    // there for the insert, and a foreign-key failure on it can only be the
    // config. A plain read let a delete in between, and the insert's failure on
    // the pool's key came back as a 409 about the config.
    sqlx::query("SELECT 1 FROM rating_pools WHERE id = $1 FOR KEY SHARE")
        .bind(pool_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| AppError::not_found("rating pool not found"))?;
    let added = sqlx::query(
        "INSERT INTO rating_pool_members (pool_id, player_config_id, added_by)
         VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
    )
    .bind(pool_id)
    .bind(body.player_config_id)
    .bind(admin.0.id)
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        unknown_config(
            e.into(),
            "rating_pool_members_player_config_id_fkey",
            "player_config_id",
            body.player_config_id,
        )
    })?
    .rows_affected();
    // A config that is already a member -- a second click, or the anchor,
    // which the pool was created with -- adds nothing, and is answered so
    // rather than logged as an addition and refitted, as a removal's second
    // click is.
    if added == 0 {
        return Ok(Json(serde_json::json!({ "run_id": null })));
    }
    audit::log(
        &mut tx,
        "rating_pool.member_added",
        Some(admin.0.id),
        None,
        Some("player_config"),
        Some(body.player_config_id.to_string()),
        None,
    )
    .await?;
    tx.commit().await?;

    let run_id = ratings::recompute(&state.pool, pool_id, Trigger::Membership).await?;
    Ok(Json(serde_json::json!({ "run_id": run_id })))
}

/// Removes a config and refits.
///
/// Removal takes the config's games out of the evidence as well as its rating
/// out of the table, so everyone else's rating moves. That is correct — the
/// remaining ratings are now the answer to a different question — and it is why
/// this cannot be a targeted delete.
async fn remove_member(
    State(state): State<AppState>,
    admin: AdminUser,
    Path((pool_id, config_id)): Path<(Uuid, Uuid)>,
    method: Method,
    headers: HeaderMap,
    jar: CookieJar,
) -> AppResult<Json<serde_json::Value>> {
    csrf::verify(&method, &headers, &jar)?;

    let mut tx = state.pool.begin().await?;
    // Under the fit lock, which an anchor change takes too: checked outside
    // it, a removal could pass the check, an anchor change make this config
    // the anchor, and the delete below then leave the pool with an anchor
    // that is not a member, which no fit accepts.
    ratings::lock_pool_fit(&mut tx, pool_id).await?;
    let is_anchor = sqlx::query_scalar::<_, bool>(
        "SELECT anchor_player_config_id = $2 FROM rating_pools WHERE id = $1",
    )
    .bind(pool_id)
    .bind(config_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::not_found("rating pool not found"))?;

    if is_anchor {
        return Err(AppError::bad_request(
            "cannot remove the pool's anchor: every other rating is measured against it. \
             Make another member the anchor first, then remove this one.",
        ));
    }

    // A config that is not a member -- a second click -- removes nothing, and
    // is answered so rather than logged and refitted as a removal (the audit's
    // pass 22).
    let removed = sqlx::query("DELETE FROM rating_pool_members WHERE pool_id = $1 AND player_config_id = $2")
        .bind(pool_id)
        .bind(config_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if removed == 0 {
        return Err(AppError::not_found("that player config is not in this pool"));
    }
    audit::log(
        &mut tx,
        "rating_pool.member_removed",
        Some(admin.0.id),
        None,
        Some("player_config"),
        Some(config_id.to_string()),
        None,
    )
    .await?;
    tx.commit().await?;

    let run_id = ratings::recompute(&state.pool, pool_id, Trigger::Membership).await?;
    Ok(Json(serde_json::json!({ "run_id": run_id })))
}

#[derive(Deserialize)]
struct UpdatePoolBody {
    anchor_player_config_id: Option<Uuid>,
    anchor_rating: Option<f64>,
}

/// Moves a pool's anchor, or the rating it is pinned at, and refits.
///
/// The ratings are only defined up to where the anchor pins them, so this
/// rescales everyone: the refit commits with the change, so the page never
/// shows the new anchor beside ratings on the old scale. Past runs keep the
/// scale they were fitted on, and the history steps at the change -- which is
/// what happened.
///
/// A new anchor that is not yet a member is added first: a pool's fixed point
/// has to be in the pool, as `create_pool` makes it.
async fn update_pool(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(pool_id): Path<Uuid>,
    method: Method,
    headers: HeaderMap,
    jar: CookieJar,
    ApiJson(body): ApiJson<UpdatePoolBody>,
) -> AppResult<Json<serde_json::Value>> {
    csrf::verify(&method, &headers, &jar)?;

    if body.anchor_player_config_id.is_none() && body.anchor_rating.is_none() {
        return Err(AppError::bad_request(
            "name an anchor_player_config_id, an anchor_rating, or both",
        ));
    }
    if let Some(rating) = body.anchor_rating {
        if !rating.is_finite() || rating.abs() > MAX_ABS_ANCHOR_RATING {
            return Err(AppError::bad_request("rating pool details are invalid").with_field(
                "anchor_rating",
                format!(
                    "must be a number between -{MAX_ABS_ANCHOR_RATING} and {MAX_ABS_ANCHOR_RATING}"
                ),
            ));
        }
    }

    // The lock first, then the read: a fit in flight finishes on the old
    // anchor before this reads it, and a removal of the new anchor either
    // lands before (and the anchor is re-added below) or waits and is refused.
    let mut tx = state.pool.begin().await?;
    ratings::lock_pool_fit(&mut tx, pool_id).await?;
    let (old_anchor, old_rating): (Uuid, f64) = sqlx::query_as(
        "SELECT anchor_player_config_id, anchor_rating FROM rating_pools WHERE id = $1 FOR UPDATE",
    )
    .bind(pool_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::not_found("rating pool not found"))?;
    let anchor = body.anchor_player_config_id.unwrap_or(old_anchor);
    let rating = body.anchor_rating.unwrap_or(old_rating);
    if anchor == old_anchor && rating == old_rating {
        // A second click: nothing moved, so nothing is logged or refitted.
        return Ok(Json(serde_json::json!({ "run_id": null })));
    }

    let added = sqlx::query(
        "INSERT INTO rating_pool_members (pool_id, player_config_id, added_by)
         VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
    )
    .bind(pool_id)
    .bind(anchor)
    .bind(admin.0.id)
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        unknown_config(
            e.into(),
            "rating_pool_members_player_config_id_fkey",
            "anchor_player_config_id",
            anchor,
        )
    })?
    .rows_affected();
    if added > 0 {
        audit::log(
            &mut tx,
            "rating_pool.member_added",
            Some(admin.0.id),
            None,
            Some("player_config"),
            Some(anchor.to_string()),
            None,
        )
        .await?;
    }
    sqlx::query("UPDATE rating_pools SET anchor_player_config_id = $2, anchor_rating = $3 WHERE id = $1")
        .bind(pool_id)
        .bind(anchor)
        .bind(rating)
        .execute(&mut *tx)
        .await?;
    audit::log_detail(
        &mut tx,
        "rating_pool.anchor_changed",
        admin.0.id,
        "rating_pool",
        pool_id.to_string(),
        None,
        format!("anchor={old_anchor}@{old_rating} -> {anchor}@{rating}"),
    )
    .await?;
    let run_id = ratings::recompute_within(&mut tx, pool_id, Trigger::Anchor).await?;
    tx.commit().await?;
    Ok(Json(serde_json::json!({ "run_id": run_id })))
}

/// Deletes a pool with its members, runs, ratings and residuals (they
/// cascade). The games stay: they belong to their jobs, and a pool is only a
/// view over them, so a pool recreated with the same scope and members fits the
/// same ratings again. What goes is the history, which is why the census says
/// how much of it there was.
///
/// Deleting frees the anchor and member configs, and the pool's letter
/// distribution and layout, for their own deletes.
async fn delete_pool(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(pool_id): Path<Uuid>,
    method: Method,
    headers: HeaderMap,
    jar: CookieJar,
) -> AppResult<StatusCode> {
    csrf::verify(&method, &headers, &jar)?;

    let mut tx = state.pool.begin().await?;
    // Waits for a fit in flight: its run would otherwise be written against a
    // pool whose rows this is cascading away.
    ratings::lock_pool_fit(&mut tx, pool_id).await?;
    let census: (String, i64, i64) = sqlx::query_as(
        "SELECT p.name,
                (SELECT COUNT(*) FROM rating_pool_members m WHERE m.pool_id = p.id),
                (SELECT COUNT(*) FROM rating_runs r WHERE r.pool_id = p.id)
         FROM rating_pools p WHERE p.id = $1 FOR UPDATE",
    )
    .bind(pool_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::not_found("rating pool not found"))?;
    let (name, members, runs) = census;
    audit::log_detail(
        &mut tx,
        "rating_pool.deleted.census",
        admin.0.id,
        "rating_pool",
        pool_id.to_string(),
        None,
        format!("name={name} members={members} runs={runs}"),
    )
    .await?;
    audit::log(
        &mut tx,
        "rating_pool.deleted",
        Some(admin.0.id),
        None,
        Some("rating_pool"),
        Some(pool_id.to_string()),
        None,
    )
    .await?;
    sqlx::query("DELETE FROM rating_pools WHERE id = $1")
        .bind(pool_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn recompute_pool(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(pool_id): Path<Uuid>,
    method: Method,
    headers: HeaderMap,
    jar: CookieJar,
) -> AppResult<Json<serde_json::Value>> {
    csrf::verify(&method, &headers, &jar)?;
    let run_id = ratings::recompute(&state.pool, pool_id, Trigger::Manual).await?;
    Ok(Json(serde_json::json!({ "run_id": run_id })))
}
