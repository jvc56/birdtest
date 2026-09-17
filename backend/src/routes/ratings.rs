//! Rating pool endpoints: public reads of the latest fit, admin writes to
//! membership.
//!
//! Split across two routers (mounted under `/api` and `/api/admin`) because the
//! read side is public — ratings are the point of the exercise — while who is
//! *in* a pool is an admin decision.

use crate::audit;
use crate::auth::{csrf, AdminUser};
use crate::error::{AppError, AppResult};
use crate::ratings::{self, Trigger};
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::routing::{delete, get, post};
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

#[derive(Serialize)]
struct PoolDetail {
    id: Uuid,
    name: String,
    variant: String,
    letter_distribution: String,
    layout: String,
    anchor_player_config_id: Uuid,
    anchor_rating: f64,
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
         FROM rating_runs WHERE pool_id = $1 ORDER BY computed_at DESC LIMIT 1",
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
/// active job at ten members is over 200,000 points on every page view, for a
/// chart a few hundred pixels wide.
const MAX_HISTORY_RUNS: i64 = 500;

/// The pool's rating history, oldest first: the chart's time axis. Snapshots per
/// run rather than a mutated current value are what make this possible at all.
///
/// Thinned to at most [`MAX_HISTORY_RUNS`] runs, evenly spaced over the pool's
/// whole history, with the first and the newest always kept -- so the chart
/// still starts where the pool started and ends at the rating the page shows.
async fn pool_history(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Vec<HistoryPoint>>> {
    let rows = sqlx::query(
        "WITH runs AS (
             SELECT id, computed_at,
                    row_number() OVER (ORDER BY computed_at) AS n,
                    count(*) OVER () AS total
             FROM rating_runs WHERE pool_id = $1
         ),
         kept AS (
             SELECT id, computed_at FROM runs
             WHERE total <= $2
                OR (n - 1) % ((total + $2 - 1) / $2) = 0
                OR n = total
         )
         SELECT kept.computed_at, r.player_config_id, c.name, r.rating, r.stderr
         FROM kept
         JOIN player_config_ratings r ON r.run_id = kept.id
         JOIN player_configs c        ON c.id = r.player_config_id
         WHERE r.connected_to_anchor
         ORDER BY kept.computed_at ASC, c.name ASC",
    )
    .bind(id)
    .bind(MAX_HISTORY_RUNS)
    .fetch_all(&state.read_pool)
    .await?;

    Ok(Json(
        rows.iter()
            .map(|row| HistoryPoint {
                computed_at: row.get("computed_at"),
                player_config_id: row.get("player_config_id"),
                name: row.get("name"),
                rating: row.get("rating"),
                stderr: row.get("stderr"),
            })
            .collect(),
    ))
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

async fn create_pool(
    State(state): State<AppState>,
    admin: AdminUser,
    method: Method,
    headers: HeaderMap,
    jar: CookieJar,
    Json(body): Json<CreatePoolBody>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    csrf::verify(&method, &headers, &jar)?;

    let mut tx = state.pool.begin().await?;
    let pool_id: Uuid = sqlx::query_scalar(
        "INSERT INTO rating_pools
             (name, variant, letterdist_id, layout_id, anchor_player_config_id, anchor_rating)
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
    )
    .bind(&body.name)
    .bind(&body.variant)
    .bind(body.letterdist_id)
    .bind(body.layout_id)
    .bind(body.anchor_player_config_id)
    .bind(body.anchor_rating)
    .fetch_one(&mut *tx)
    .await?;

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

#[derive(Deserialize)]
struct MemberBody {
    player_config_id: Uuid,
}

/// Adds a config and refits the pool.
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
    Json(body): Json<MemberBody>,
) -> AppResult<Json<serde_json::Value>> {
    csrf::verify(&method, &headers, &jar)?;

    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "INSERT INTO rating_pool_members (pool_id, player_config_id, added_by)
         VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
    )
    .bind(pool_id)
    .bind(body.player_config_id)
    .bind(admin.0.id)
    .execute(&mut *tx)
    .await?;
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

    let is_anchor = sqlx::query_scalar::<_, bool>(
        "SELECT anchor_player_config_id = $2 FROM rating_pools WHERE id = $1",
    )
    .bind(pool_id)
    .bind(config_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::not_found("rating pool not found"))?;

    if is_anchor {
        return Err(AppError::bad_request(
            "cannot remove the pool's anchor: every other rating is measured against it. \
             Point the pool at a different anchor first.",
        ));
    }

    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM rating_pool_members WHERE pool_id = $1 AND player_config_id = $2")
        .bind(pool_id)
        .bind(config_id)
        .execute(&mut *tx)
        .await?;
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
