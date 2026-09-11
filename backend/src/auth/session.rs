use crate::config::Config;
use crate::error::{AppError, AppResult};
use chrono::{Duration as ChronoDuration, Utc};
use pasetors::claims::{Claims, ClaimsValidationRules};
use pasetors::keys::SymmetricKey;
use pasetors::token::UntrustedToken;
use pasetors::version4::V4;
use pasetors::{local, Local};
use uuid::Uuid;

pub const SESSION_COOKIE: &str = "birdtest_session";

/// What a decoded session cookie tells us about the caller.
///
/// Only the subject is consumed: username and admin status are re-read from the
/// database on every request so a deleted or demoted account cannot keep acting
/// on a token minted before the change.
#[derive(Debug, Clone)]
pub struct SessionClaims {
    pub user_id: Uuid,
}

fn key(cfg: &Config) -> SymmetricKey<V4> {
    SymmetricKey::<V4>::from(&cfg.session_signing_key)
        .expect("a 32-byte key is always valid for v4.local")
}

pub fn issue(cfg: &Config, user_id: Uuid, username: &str, is_admin: bool) -> AppResult<String> {
    let now = Utc::now();
    let expiry = now + ChronoDuration::from_std(cfg.session_ttl).unwrap_or(ChronoDuration::days(7));

    let mut claims = Claims::new().map_err(|e| AppError::internal(e.to_string()))?;
    claims
        .subject(&user_id.to_string())
        .and_then(|_| claims.issued_at(&now.to_rfc3339()))
        .and_then(|_| claims.expiration(&expiry.to_rfc3339()))
        .and_then(|_| claims.add_additional("username", username.to_string()))
        .and_then(|_| claims.add_additional("is_admin", is_admin))
        .map_err(|e| AppError::internal(e.to_string()))?;

    local::encrypt(&key(cfg), &claims, None, None)
        .map_err(|e| AppError::internal(format!("failed to mint session token: {e}")))
}

pub fn verify(cfg: &Config, token: &str) -> AppResult<SessionClaims> {
    let untrusted = UntrustedToken::<Local, V4>::try_from(token)
        .map_err(|_| AppError::unauthorized("invalid session"))?;
    let rules = ClaimsValidationRules::new();
    let trusted = local::decrypt(&key(cfg), &untrusted, &rules, None, None)
        .map_err(|_| AppError::unauthorized("invalid or expired session"))?;
    let claims = trusted
        .payload_claims()
        .ok_or_else(|| AppError::unauthorized("session carries no claims"))?;

    let get = |name: &str| claims.get_claim(name).cloned();
    let user_id = get("sub")
        .and_then(|v| v.as_str().map(str::to_owned))
        .and_then(|v| Uuid::parse_str(&v).ok())
        .ok_or_else(|| AppError::unauthorized("session has no subject"))?;
    Ok(SessionClaims { user_id })
}
