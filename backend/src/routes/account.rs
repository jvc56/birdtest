use crate::auth::{api_key, csrf, CurrentUser};
use crate::error::{AppError, AppResult};
use crate::models::user::ApiKeyRow;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::routing::{get, patch};
use axum::{Json, Router};
use axum_extra::extract::CookieJar;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The plan's cap. Enforced here rather than as a DB constraint so the error is
/// a clean 409 instead of a constraint violation.
const MAX_API_KEYS: i64 = 100;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/me", get(me))
        .route("/api/me/api-keys", get(list_keys).post(create_key))
        .route("/api/me/api-keys/:id", patch(set_key_active).delete(revoke_key))
}

#[derive(Serialize)]
struct Me {
    id: Uuid,
    username: String,
    email: String,
    is_admin: bool,
    tasks_completed: i64,
}

async fn me(State(state): State<AppState>, user: CurrentUser) -> AppResult<Json<Me>> {
    let tasks_completed = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM task_claims WHERE claimed_by_user_id = $1 AND state = 'completed'",
    )
    .bind(user.id)
    .fetch_one(&state.pool)
    .await?;

    Ok(Json(Me {
        id: user.id,
        username: user.username,
        email: user.email,
        is_admin: user.is_admin,
        tasks_completed,
    }))
}

async fn list_keys(
    State(state): State<AppState>,
    user: CurrentUser,
) -> AppResult<Json<Vec<ApiKeyRow>>> {
    // Hashes are never returned, only the metadata a user needs to manage keys.
    Ok(Json(
        sqlx::query_as::<_, ApiKeyRow>(
            "SELECT id, label, is_active, created_at, last_used_at
             FROM api_keys WHERE user_id = $1 ORDER BY created_at DESC",
        )
        .bind(user.id)
        .fetch_all(&state.pool)
        .await?,
    ))
}

#[derive(Deserialize)]
struct CreateKeyBody {
    label: Option<String>,
}

#[derive(Serialize)]
struct CreatedKey {
    id: Uuid,
    label: Option<String>,
    /// Shown exactly once — only the hash is stored.
    key: String,
}

async fn create_key(
    State(state): State<AppState>,
    user: CurrentUser,
    method: Method,
    headers: HeaderMap,
    jar: CookieJar,
    Json(body): Json<CreateKeyBody>,
) -> AppResult<(StatusCode, Json<CreatedKey>)> {
    csrf::verify(&method, &headers, &jar)?;

    let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM api_keys WHERE user_id = $1")
        .bind(user.id)
        .fetch_one(&state.pool)
        .await?;
    if count >= MAX_API_KEYS {
        return Err(AppError::conflict(format!(
            "you already have {MAX_API_KEYS} API keys — revoke one first"
        )));
    }

    let raw = api_key::generate_raw_key();
    let id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO api_keys (user_id, key_hash, label) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(user.id)
    .bind(api_key::hash_key(&raw))
    .bind(&body.label)
    .fetch_one(&state.pool)
    .await?;

    Ok((StatusCode::CREATED, Json(CreatedKey { id, label: body.label, key: raw })))
}

#[derive(Deserialize)]
struct SetActiveBody {
    is_active: bool,
}

async fn set_key_active(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    method: Method,
    headers: HeaderMap,
    jar: CookieJar,
    Json(body): Json<SetActiveBody>,
) -> AppResult<StatusCode> {
    csrf::verify(&method, &headers, &jar)?;

    let updated = sqlx::query("UPDATE api_keys SET is_active = $1 WHERE id = $2 AND user_id = $3")
        .bind(body.is_active)
        .bind(id)
        .bind(user.id)
        .execute(&state.pool)
        .await?;

    if updated.rows_affected() == 0 {
        return Err(AppError::not_found("no such API key"));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn revoke_key(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    method: Method,
    headers: HeaderMap,
    jar: CookieJar,
) -> AppResult<StatusCode> {
    csrf::verify(&method, &headers, &jar)?;

    let deleted = sqlx::query("DELETE FROM api_keys WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user.id)
        .execute(&state.pool)
        .await?;

    if deleted.rows_affected() == 0 {
        return Err(AppError::not_found("no such API key"));
    }
    Ok(StatusCode::NO_CONTENT)
}
