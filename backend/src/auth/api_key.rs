use crate::error::{AppError, AppResult};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use rand::RngCore;
use sha2::{Digest, Sha256};

/// A raw API key: a URL-safe random string shown to the user exactly once.
pub fn generate_raw_key() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("bt_{}", hex::encode(bytes))
}

/// API keys are looked up by exact hash match on every worker request, so the
/// stored hash has to be deterministic — a per-key Argon2 salt would force a
/// full-table scan and a verify per row. SHA-256 over a 256-bit random key is
/// the right tool here: there is no low-entropy secret to protect against
/// offline guessing, only a need to avoid storing the raw key.
pub fn hash_key(raw: &str) -> String {
    let digest = Sha256::digest(raw.as_bytes());
    hex::encode(digest)
}

/// Passwords, unlike API keys, are low entropy and are only ever verified for a
/// single known row, so they get Argon2 with a per-user salt.
///
/// Argon2 is tens of milliseconds of pure computation by design, so request
/// handlers call the `_off_the_executor` forms below: run inline it occupies an
/// async worker thread for all of it, and a worker that does not yield can be
/// the one the whole runtime's socket events are waiting on (see
/// `exports::upload_rows`, where that was found).
pub fn hash_password(password: &str) -> AppResult<String> {
    let salt = SaltString::generate(&mut rand::rngs::OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| AppError::internal(format!("password hashing failed: {e}")))
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

/// [`hash_password`] on the blocking pool.
pub async fn hash_password_off_the_executor(password: String) -> AppResult<String> {
    tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .map_err(|e| AppError::internal(format!("password hashing task failed: {e}")))?
}

/// [`verify_password`] on the blocking pool. A task that fails verifies
/// nothing.
pub async fn verify_password_off_the_executor(password: String, hash: String) -> bool {
    tokio::task::spawn_blocking(move || verify_password(&password, &hash))
        .await
        .unwrap_or(false)
}

/// Single-use codes emailed to the user (confirmation, password reset). Stored
/// hashed for the same reason API keys are, and looked up the same way.
pub fn generate_code() -> String {
    let mut bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

pub fn hash_code(raw: &str) -> String {
    hash_key(raw)
}
