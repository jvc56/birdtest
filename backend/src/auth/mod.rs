pub mod api_key;
pub mod csrf;
pub mod session;

use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum_extra::extract::CookieJar;
use uuid::Uuid;

/// An authenticated browser session, resolved from the session cookie and
/// re-checked against the database so a deleted or demoted account cannot keep
/// acting on a still-valid token.
#[derive(Debug, Clone)]
pub struct CurrentUser {
    pub id: Uuid,
    pub username: String,
    pub email: String,
    pub is_admin: bool,
}

#[axum::async_trait]
impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let jar = CookieJar::from_headers(&parts.headers);
        let token = jar
            .get(session::SESSION_COOKIE)
            .map(|c| c.value().to_string())
            .ok_or_else(|| AppError::unauthorized("not signed in"))?;
        let claims = session::verify(&state.cfg, &token)?;

        let row = sqlx::query_as::<_, (Uuid, String, String, bool)>(
            "SELECT id, username, email, is_admin FROM users WHERE id = $1",
        )
        .bind(claims.user_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::unauthorized("account no longer exists"))?;

        Ok(CurrentUser { id: row.0, username: row.1, email: row.2, is_admin: row.3 })
    }
}

/// Same as [`CurrentUser`] but rejects non-admins with 403. Every Admin API
/// route takes this instead of `CurrentUser`, so the authorization check cannot
/// be forgotten in an individual handler.
#[derive(Debug, Clone)]
pub struct AdminUser(pub CurrentUser);

#[axum::async_trait]
impl FromRequestParts<AppState> for AdminUser {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let user = CurrentUser::from_request_parts(parts, state).await?;
        if !user.is_admin {
            return Err(AppError::forbidden("admin privileges required"));
        }
        Ok(AdminUser(user))
    }
}

/// Who is asking for work. Authenticated workers present an API key; anonymous
/// workers present a self-generated UUID.
#[derive(Debug, Clone)]
pub enum WorkerIdentity {
    User { user_id: Uuid },
    Anonymous { uuid: Uuid },
}

impl WorkerIdentity {
    pub fn user_id(&self) -> Option<Uuid> {
        match self {
            WorkerIdentity::User { user_id } => Some(*user_id),
            WorkerIdentity::Anonymous { .. } => None,
        }
    }

    pub fn anon_uuid(&self) -> Option<Uuid> {
        match self {
            WorkerIdentity::User { .. } => None,
            WorkerIdentity::Anonymous { uuid } => Some(*uuid),
        }
    }

    /// Stable string used to key per-worker rate limits.
    pub fn rate_key(&self) -> String {
        match self {
            WorkerIdentity::User { user_id } => format!("u:{user_id}"),
            WorkerIdentity::Anonymous { uuid } => format!("a:{uuid}"),
        }
    }
}

#[axum::async_trait]
impl FromRequestParts<AppState> for WorkerIdentity {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let bearer = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(str::to_owned);

        let identity = if let Some(raw_key) = bearer {
            let hash = api_key::hash_key(&raw_key);
            let row = sqlx::query_as::<_, (Uuid, Uuid)>(
                "SELECT k.id, u.id
                 FROM api_keys k JOIN users u ON u.id = k.user_id
                 WHERE k.key_hash = $1 AND k.is_active",
            )
            .bind(&hash)
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(|| AppError::unauthorized("unknown or inactive API key"))?;

            sqlx::query("UPDATE api_keys SET last_used_at = now() WHERE id = $1")
                .bind(row.0)
                .execute(&state.pool)
                .await?;

            WorkerIdentity::User { user_id: row.1 }
        } else {
            let raw = parts
                .headers
                .get("x-worker-uuid")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| {
                    AppError::unauthorized("provide an API key or an X-Worker-UUID header")
                })?;
            let uuid = Uuid::parse_str(raw)
                .map_err(|_| AppError::bad_request("X-Worker-UUID is not a valid UUID"))?;

            // Upsert so contribution tracking works for a worker we've never seen.
            sqlx::query(
                "INSERT INTO anonymous_workers (uuid) VALUES ($1)
                 ON CONFLICT (uuid) DO UPDATE SET last_seen_at = now()",
            )
            .bind(uuid)
            .execute(&state.pool)
            .await?;

            WorkerIdentity::Anonymous { uuid }
        };

        ensure_not_banned(state, &identity).await?;
        Ok(identity)
    }
}

async fn ensure_not_banned(state: &AppState, identity: &WorkerIdentity) -> AppResult<()> {
    let banned = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (
             SELECT 1 FROM worker_bans
             WHERE (user_id IS NOT DISTINCT FROM $1 AND $1 IS NOT NULL)
                OR (anon_uuid IS NOT DISTINCT FROM $2 AND $2 IS NOT NULL)
         )",
    )
    .bind(identity.user_id())
    .bind(identity.anon_uuid())
    .fetch_one(&state.pool)
    .await?;

    if banned {
        return Err(AppError::forbidden("this worker identity is banned"));
    }
    Ok(())
}
