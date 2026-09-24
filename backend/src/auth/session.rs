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
/// Only the subject and the session generation are consumed: username and admin
/// status are re-read from the database on every request so a deleted or
/// demoted account cannot keep acting on a token minted before the change, and
/// the generation is compared with the account's so a password reset or "sign
/// out everywhere" revokes every earlier token.
#[derive(Debug, Clone)]
pub struct SessionClaims {
    pub user_id: Uuid,
    pub generation: i32,
}

fn key(cfg: &Config) -> SymmetricKey<V4> {
    SymmetricKey::<V4>::from(&cfg.session_signing_key)
        .expect("a 32-byte key is always valid for v4.local")
}

pub fn issue(
    cfg: &Config,
    user_id: Uuid,
    username: &str,
    is_admin: bool,
    generation: i32,
) -> AppResult<String> {
    let now = Utc::now();
    let expiry = now + ChronoDuration::from_std(cfg.session_ttl).unwrap_or(ChronoDuration::days(7));

    let mut claims = Claims::new().map_err(|e| AppError::internal(e.to_string()))?;
    claims
        .subject(&user_id.to_string())
        .and_then(|_| claims.issued_at(&now.to_rfc3339()))
        .and_then(|_| claims.expiration(&expiry.to_rfc3339()))
        .and_then(|_| claims.add_additional("username", username.to_string()))
        .and_then(|_| claims.add_additional("is_admin", is_admin))
        .and_then(|_| claims.add_additional("gen", generation))
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
    let generation = get("gen")
        .and_then(|v| v.as_i64())
        .and_then(|v| i32::try_from(v).ok())
        .ok_or_else(|| AppError::unauthorized("session has no generation; sign in again"))?;
    Ok(SessionClaims { user_id, generation })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(key: &str, ttl_seconds: &str) -> Config {
        let pairs = [
            ("DATABASE_URL", "postgres://a:b@c/d".to_string()),
            ("SESSION_SIGNING_KEY", key.to_string()),
            ("SESSION_TTL_SECONDS", ttl_seconds.to_string()),
        ];
        Config::from_lookup(&move |k: &str| {
            pairs.iter().find(|(name, _)| *name == k).map(|(_, v)| v.clone())
        })
        .unwrap()
    }

    const KEY: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
    const OTHER_KEY: &str = "ffeeddccbbaa99887766554433221100ffeeddccbbaa99887766554433221100";

    /// U-AUTH-4: a token carries its subject and generation through a round
    /// trip, and any altered byte makes it fail. (Username and the admin flag
    /// are in the token too, but nothing trusts them: the extractor reads the
    /// user's row, which is what lets a demotion take effect at once.)
    #[test]
    fn a_session_round_trips_and_any_altered_byte_fails() {
        let cfg = config(KEY, "3600");
        let user = Uuid::new_v4();
        let token = issue(&cfg, user, "alice", true, 7).unwrap();
        let claims = verify(&cfg, &token).unwrap();
        assert_eq!(claims.user_id, user);
        assert_eq!(claims.generation, 7);

        let prefix = "v4.local.".len();
        for i in (prefix..token.len()).step_by(7) {
            let mut bytes = token.clone().into_bytes();
            bytes[i] = if bytes[i] == b'A' { b'B' } else { b'A' };
            let altered = String::from_utf8(bytes).unwrap();
            if altered == token {
                continue;
            }
            let err = verify(&cfg, &altered).unwrap_err();
            assert_eq!(err.status, axum::http::StatusCode::UNAUTHORIZED, "byte {i}");
        }
    }

    /// U-AUTH-5: a token minted under another key is not a session here.
    #[test]
    fn a_session_signed_with_another_key_fails() {
        let token = issue(&config(OTHER_KEY, "3600"), Uuid::new_v4(), "bob", false, 0).unwrap();
        let err = verify(&config(KEY, "3600"), &token).unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::UNAUTHORIZED);
    }

    /// U-AUTH-6: an expired session is refused. Issued with a zero TTL it has
    /// expired by the time it is checked; no waiting.
    #[test]
    fn an_expired_session_is_rejected() {
        let cfg = config(KEY, "0");
        let token = issue(&cfg, Uuid::new_v4(), "carol", false, 0).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let err = verify(&cfg, &token).unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::UNAUTHORIZED);
        assert!(err.message.contains("expired"), "{}", err.message);
    }

    #[test]
    fn garbage_is_not_a_session() {
        let cfg = config(KEY, "3600");
        for token in ["", "v4.local.", "v4.public.abc", "not-a-token"] {
            assert!(verify(&cfg, token).is_err(), "{token:?}");
        }
    }
}
