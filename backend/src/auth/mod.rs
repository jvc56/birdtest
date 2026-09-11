pub mod api_key;
pub mod csrf;
pub mod session;

use crate::clientip::ClientIp;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum_extra::extract::CookieJar;
use std::net::IpAddr;
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

/// Who is asking for work.
#[derive(Debug, Clone)]
pub enum WorkerIdentity {
    /// An API key tied to an account.
    User { user_id: Uuid },
    /// A UUID the server issued earlier, presented in `X-Worker-UUID`.
    Anonymous { uuid: Uuid },
    /// No identity presented at all: a contributor that has never been issued
    /// one. A UUID is drawn for the request, but it is **not persisted here** --
    /// only a claim that actually hands out a task writes the
    /// `anonymous_workers` row, in the same transaction as the claim, and only
    /// then is the UUID returned to the client.
    ///
    /// Persisting on sight would write a row for every request that arrives
    /// without a header, and a new contributor polling a quiet server gets a
    /// `204` with no body to carry the UUID in -- so it would come back
    /// identity-less and mint another one on every poll, and anyone could fill
    /// the table by omitting a header.
    Unregistered { uuid: Uuid, client_ip: IpAddr },
}

impl WorkerIdentity {
    pub fn user_id(&self) -> Option<Uuid> {
        match self {
            WorkerIdentity::User { user_id } => Some(*user_id),
            WorkerIdentity::Anonymous { .. } | WorkerIdentity::Unregistered { .. } => None,
        }
    }

    pub fn anon_uuid(&self) -> Option<Uuid> {
        match self {
            WorkerIdentity::User { .. } => None,
            WorkerIdentity::Anonymous { uuid } | WorkerIdentity::Unregistered { uuid, .. } => {
                Some(*uuid)
            }
        }
    }

    /// The UUID to hand back to the client, when the server just minted one
    /// for a request that arrived with no identity at all.
    pub fn newly_assigned_uuid(&self) -> Option<Uuid> {
        match self {
            WorkerIdentity::Unregistered { uuid, .. } => Some(*uuid),
            _ => None,
        }
    }

    /// Stable string used to key per-worker rate limits. An unregistered
    /// worker has no stable identity yet, so it is limited by address -- keying
    /// on its freshly drawn UUID would give every request its own bucket.
    pub fn rate_key(&self) -> String {
        match self {
            WorkerIdentity::User { user_id } => format!("u:{user_id}"),
            WorkerIdentity::Anonymous { uuid } => format!("a:{uuid}"),
            WorkerIdentity::Unregistered { client_ip, .. } => format!("ip:{client_ip}"),
        }
    }

    /// Everything but a claim acts on something a claim created, so it needs
    /// the identity that claim was issued to.
    pub fn require_registered(&self) -> AppResult<()> {
        match self {
            WorkerIdentity::Unregistered { .. } => Err(AppError::unauthorized(
                "this request needs a worker identity: send the X-Worker-UUID the \
                 server issued with your first task, or an API key",
            )),
            _ => Ok(()),
        }
    }

    /// Applies the right per-worker rate limit for this identity.
    pub fn check_rate_limit(&self, state: &AppState) -> AppResult<()> {
        let limiter = match self {
            WorkerIdentity::Unregistered { .. } => &state.limits.unregistered_worker,
            _ => &state.limits.worker,
        };
        crate::ratelimit::check(limiter, &self.rate_key())
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

            // Throttled: `last_used_at` answers "is this key in use", which a
            // minute's resolution answers as well as a write per request does.
            sqlx::query(
                "UPDATE api_keys SET last_used_at = now()
                 WHERE id = $1
                   AND (last_used_at IS NULL OR last_used_at < now() - interval '60 seconds')",
            )
            .bind(row.0)
            .execute(&state.pool)
            .await?;

            WorkerIdentity::User { user_id: row.1 }
        } else {
            let raw = parts
                .headers
                .get("x-worker-uuid")
                .and_then(|v| v.to_str().ok());

            match raw {
                Some(raw) => {
                    let uuid = Uuid::parse_str(raw.trim()).map_err(|_| {
                        AppError::bad_request("X-Worker-UUID is not a valid UUID")
                    })?;

                    // Only identities the server itself issued are accepted. A
                    // client-invented UUID would otherwise let anyone
                    // manufacture contributors, attributing work to identities
                    // that never claimed anything. `last_seen_at` is refreshed
                    // at most once a minute, for the same reason as above.
                    let known = sqlx::query_scalar::<_, bool>(
                        "WITH known AS (
                             SELECT uuid, last_seen_at FROM anonymous_workers WHERE uuid = $1
                         ),
                         touched AS (
                             UPDATE anonymous_workers w SET last_seen_at = now()
                             FROM known
                             WHERE w.uuid = known.uuid
                               AND known.last_seen_at < now() - interval '60 seconds'
                             RETURNING 1
                         )
                         SELECT EXISTS (SELECT 1 FROM known)",
                    )
                    .bind(uuid)
                    .fetch_one(&state.pool)
                    .await?;

                    if !known {
                        return Err(AppError::unauthorized(
                            "unrecognized worker UUID. Omit the X-Worker-UUID \
                             header to be issued one, or authenticate with an \
                             API key.",
                        ));
                    }
                    WorkerIdentity::Anonymous { uuid }
                }
                None => {
                    let ClientIp(client_ip) = ClientIp::from_request_parts(parts, state).await?;
                    // Nothing to ban: this identity does not exist yet.
                    return Ok(WorkerIdentity::Unregistered { uuid: Uuid::new_v4(), client_ip });
                }
            }
        };

        ensure_not_banned(state, &identity).await?;
        Ok(identity)
    }
}

async fn ensure_not_banned(state: &AppState, identity: &WorkerIdentity) -> AppResult<()> {
    let banned = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (
             SELECT 1 FROM worker_bans
             WHERE user_id = $1 OR anon_uuid = $2
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
