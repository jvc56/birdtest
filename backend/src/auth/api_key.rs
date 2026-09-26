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

/// How many Argon2 runs may hold memory at once. Each takes 19 MiB (the
/// crate's default parameters) on the blocking pool, which would run hundreds
/// together: seven addresses' worth of logins and registrations — within
/// every per-address limit, needing no account — took the process past its
/// 2 GiB task, which the single instance does not survive (thirty-second
/// audit). Four is 76 MiB, and four at a time on one vCPU is as fast as more.
const ARGON2_CONCURRENCY: usize = 4;
/// How long a request waits for a turn before it is told to come back: past
/// this the queue is a flood, not a busy minute.
const ARGON2_QUEUE_WAIT: std::time::Duration = std::time::Duration::from_secs(10);
static ARGON2_PERMITS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(ARGON2_CONCURRENCY);

async fn argon2_turn() -> AppResult<tokio::sync::SemaphorePermit<'static>> {
    match tokio::time::timeout(ARGON2_QUEUE_WAIT, ARGON2_PERMITS.acquire()).await {
        Ok(Ok(permit)) => Ok(permit),
        Ok(Err(_)) => Err(AppError::internal("the password queue is closed")),
        Err(_) => Err(AppError {
            retry_after: Some(10),
            ..AppError::new(
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "unavailable",
                "too many sign-ins at once; try again shortly",
            )
        }),
    }
}

type Argon2Job = Box<dyn FnOnce() + Send + 'static>;

/// The threads Argon2 runs on: exactly `ARGON2_CONCURRENCY`, for good. Each
/// run allocates its 19 MiB on the thread it runs on, and glibc keeps a
/// thread's freed memory for that thread; on the blocking pool, which rotates
/// work across many threads, that held 36 runs' worth after a flood although
/// only four ran at a time (694 MB measured). On four threads of its own it
/// is four runs' worth.
fn argon2_threads() -> &'static std::sync::mpsc::Sender<Argon2Job> {
    static SENDER: std::sync::OnceLock<std::sync::mpsc::Sender<Argon2Job>> =
        std::sync::OnceLock::new();
    SENDER.get_or_init(|| {
        let (sender, receiver) = std::sync::mpsc::channel::<Argon2Job>();
        let receiver = std::sync::Arc::new(std::sync::Mutex::new(receiver));
        for n in 0..ARGON2_CONCURRENCY {
            let receiver = receiver.clone();
            std::thread::Builder::new()
                .name(format!("argon2-{n}"))
                .spawn(move || loop {
                    let job = match receiver.lock() {
                        Ok(receiver) => receiver.recv(),
                        Err(_) => return,
                    };
                    match job {
                        Ok(job) => job(),
                        Err(_) => return,
                    }
                })
                .expect("an Argon2 thread starts");
        }
        sender
    })
}

/// Runs `work` on an Argon2 thread once a turn is free. A panic in it is
/// caught there, so the thread lives on, and reported as a failure.
///
/// The turn travels with the job and is given back when the job is done, not
/// when the request is: held by the request, a client that hung up gave its
/// turn back while its run stayed queued, so the queue grew past the turns
/// with nothing shedding it — 2,000 abandoned logins left a fresh one waiting
/// 46 s, and no request was ever told `503` (the audit's adversarial check). A
/// job whose requester has gone is skipped.
async fn on_an_argon2_thread<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> AppResult<T> {
    let turn = argon2_turn().await?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    argon2_threads()
        .send(Box::new(move || {
            let _turn = turn;
            if sender.is_closed() {
                return;
            }
            if let Ok(value) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)) {
                let _ = sender.send(value);
            }
        }))
        .map_err(|_| AppError::internal("the password threads are gone"))?;
    receiver
        .await
        .map_err(|_| AppError::internal("hashing a password did not finish"))
}

/// [`hash_password`] on an Argon2 thread, one of at most `ARGON2_CONCURRENCY`.
pub async fn hash_password_off_the_executor(password: String) -> AppResult<String> {
    on_an_argon2_thread(move || hash_password(&password)).await?
}

/// [`verify_password`] on an Argon2 thread, one of at most
/// `ARGON2_CONCURRENCY`. A run that fails verifies nothing; a request that
/// waited too long for its turn is a `503`.
pub async fn verify_password_off_the_executor(password: String, hash: String) -> AppResult<bool> {
    match on_an_argon2_thread(move || verify_password(&password, &hash)).await {
        Ok(verified) => Ok(verified),
        // Waited too long for a turn: the caller is told to come back.
        Err(err) if err.status == axum::http::StatusCode::SERVICE_UNAVAILABLE => Err(err),
        Err(_) => Ok(false),
    }
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

    /// Argon2 runs wait for one of `ARGON2_CONCURRENCY` turns: with every
    /// turn taken, a hash does not start, and it runs as soon as one is given
    /// back. Unbounded, a flood of logins held 19 MiB each on hundreds of
    /// blocking threads and took the process past its memory.
    #[tokio::test]
    async fn argon2_runs_wait_for_a_turn() {
        let held = ARGON2_PERMITS.acquire_many(ARGON2_CONCURRENCY as u32).await.unwrap();
        let hashing = tokio::spawn(hash_password_off_the_executor("correct horse".into()));
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(!hashing.is_finished(), "a hash ran with every turn taken");
        drop(held);
        let hash = tokio::time::timeout(std::time::Duration::from_secs(10), hashing)
            .await
            .expect("it runs once a turn is free")
            .unwrap()
            .unwrap();
        assert!(verify_password("correct horse", &hash));
    }

    /// A request that goes away gives its turn back only when its run is
    /// done or skipped: abandoned requests cannot queue runs past the turns.
    #[tokio::test]
    async fn abandoned_argon2_runs_do_not_queue_past_the_turns() {
        let mut abandoned = Vec::new();
        // A run takes ~30 ms here, so a thousand queued runs are seconds of
        // wait for four threads; skipped, they are nothing.
        for _ in 0..1000 {
            let request = tokio::spawn(hash_password_off_the_executor("abandoned".into()));
            tokio::time::sleep(std::time::Duration::from_micros(200)).await;
            request.abort();
            abandoned.push(request);
        }
        let started = std::time::Instant::now();
        let hash = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            hash_password_off_the_executor("fresh".into()),
        )
        .await
        .expect("a fresh request is not stuck behind abandoned ones")
        .unwrap();
        assert!(verify_password("fresh", &hash));
        assert!(started.elapsed() < std::time::Duration::from_secs(1), "{:?}", started.elapsed());
    }

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
