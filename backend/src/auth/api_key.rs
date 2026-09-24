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

#[cfg(test)]
mod tests {
    use super::*;

    /// U-AUTH-1: lookup is an exact match on the hash, so hashing must be
    /// deterministic -- and two generated keys must not collide.
    #[test]
    fn a_key_hashes_the_same_every_time_and_two_keys_differ() {
        let a = generate_raw_key();
        let b = generate_raw_key();
        assert_ne!(a, b);
        assert_eq!(hash_key(&a), hash_key(&a));
        assert_ne!(hash_key(&a), hash_key(&b));
        assert_eq!(hash_key(&a).len(), 64, "a hex SHA-256");
        assert_ne!(hash_key(&a), a, "the stored form is not the key");
    }

    /// U-AUTH-2: a raw key is URL-safe and carries 32 bytes of entropy.
    #[test]
    fn a_generated_key_is_url_safe_with_32_bytes_of_entropy() {
        let key = generate_raw_key();
        let hex_part = key.strip_prefix("bt_").expect("keys are prefixed bt_");
        assert_eq!(hex::decode(hex_part).expect("hex after the prefix").len(), 32);
        // RFC 3986 unreserved characters only: nothing to escape in a URL,
        // a header or a shell.
        assert!(key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'), "{key}");
    }

    /// U-AUTH-3: a password verifies against its own hash only, and each hash
    /// is salted, so equal passwords do not produce equal hashes.
    #[test]
    fn a_password_verifies_against_its_own_salted_hash_only() {
        let first = hash_password("correct horse battery").unwrap();
        let second = hash_password("correct horse battery").unwrap();
        let other = hash_password("tr0ub4dor&3").unwrap();
        assert_ne!(first, second, "per-password salt");
        assert!(verify_password("correct horse battery", &first));
        assert!(verify_password("correct horse battery", &second));
        assert!(!verify_password("correct horse battery", &other));
        assert!(!verify_password("tr0ub4dor&3", &first));
        assert!(!verify_password("correct horse battery", "not a phc string"));
    }

    #[test]
    fn a_confirmation_code_is_stored_only_as_its_hash() {
        let code = generate_code();
        assert_eq!(hex::decode(&code).unwrap().len(), 24);
        assert_ne!(generate_code(), code);
        assert_eq!(hash_code(&code), hash_code(&code));
        assert_ne!(hash_code(&code), code);
    }
}
