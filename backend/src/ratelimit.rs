use crate::error::AppError;
use governor::clock::DefaultClock;
use governor::state::keyed::DefaultKeyedStateStore;
use governor::{Quota, RateLimiter};
use std::collections::HashMap;
use std::net::IpAddr;
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type Keyed = RateLimiter<String, DefaultKeyedStateStore<String>, DefaultClock>;

/// In-memory token buckets. State resets on process restart, which is fine for
/// v1 — a persistent backend would only matter once there is more than one
/// instance to coordinate.
#[derive(Clone)]
pub struct RateLimiters {
    /// 10 registrations per hour per IP.
    pub register: Arc<Keyed>,
    /// 1 request per second per worker identity, burst 5, applied to every
    /// worker endpoint.
    pub worker: Arc<Keyed>,
    /// Worker requests that carry no identity at all, keyed by client IP.
    ///
    /// More generous than `worker`, because it is shared: several brand-new
    /// contributors behind one NAT all land in the same bucket until each is
    /// issued a UUID. It exists at all because without it, omitting the
    /// identity header would be a way around the per-identity limit.
    pub unregistered_worker: Arc<Keyed>,
    /// 5 password-reset requests per hour, checked twice: once against the
    /// caller's IP and once against the address they asked for.
    ///
    /// The per-address half is the one that matters. Without it this is an
    /// unauthenticated endpoint that sends mail to any address it is given, so
    /// it is both a way to probe which addresses have accounts and a way to
    /// bury a known contributor in reset emails at the operator's expense.
    /// Limiting by IP alone stops neither, since IPs are cheap.
    pub reset: Arc<Keyed>,
    /// 10 login attempts per minute from one IP. Each attempt costs an Argon2 verify, so
    /// an unlimited login endpoint is both an online password-guessing oracle
    /// and a cheap way to pin the server's CPU.
    pub login: Arc<Keyed>,
    /// 100 login attempts per minute against one username from anywhere: the
    /// bound on a guesser spread over many addresses. Ten times `login`, so
    /// that one address cannot lock an account out.
    pub login_account: Arc<Keyed>,
    /// API keys created per account: a burst of 100 (the key cap), then 10
    /// an hour. `worker` is per key, and revoking a key and making another
    /// would be a fresh bucket each time; this bounds that churn without
    /// holding back a contributor setting up many machines at once (at ten
    /// an hour from the start, fifty machines took five hours). (An account-wide worker bucket was tried and
    /// was too tight for the hundred keys an account may hold: fifty idle
    /// machines filled it, and heartbeats, which are not retried, lapsed.)
    pub key_creation: Arc<Keyed>,
    /// Worker credentials, before their lookup: see [`CredentialGate`].
    pub worker_credentials: Arc<CredentialGate>,
    /// Confirmation and reset links redeemed, per client address: 20 a
    /// minute. The codes are too long to guess, so this is about cost, not
    /// guessing -- both routes are unauthenticated and write on the main pool,
    /// and a reset scores the new password first.
    pub redeem: Arc<Keyed>,
}

/// Worker credentials looked up in the database, and how many of them one
/// address may try.
///
/// Resolving a worker identity is a main-pool query, on the pool claims and
/// submissions need (`db.rs`). Every presented credential is charged its own
/// bucket (`worker`, 1 a second, burst 5) before that query -- the key's hash
/// or the UUID already names the bucket -- so a real identity costs at most
/// that many lookups. A credential that has not resolved in the last ten
/// minutes also pays a cell of its address's bucket (5 a second, burst 100)
/// before its lookup, match or not: made-up keys and UUIDs, each a fresh
/// bucket of its own, are bounded per address, and so are the lookups a
/// burst of them sends at once. Credentials that resolved recently skip the
/// address's bucket, so a misbehaving machine -- or a fleet still running a
/// revoked key -- behind a shared address does not lock out the workers that
/// are fine. The burst admits a hundred machines behind one address at once
/// after a restart, when nothing is known yet.
pub struct CredentialGate {
    unknown_per_address: Keyed,
    known: Mutex<HashMap<String, Instant>>,
}

/// How long a credential that resolved counts as known.
const KNOWN_FOR: Duration = Duration::from_secs(600);
/// At most this many known credentials are remembered (about 7 MB); past it a
/// credential is simply treated as unknown, which costs its address a cell.
const MAX_KNOWN: usize = 50_000;

impl CredentialGate {
    fn new() -> Self {
        Self {
            unknown_per_address: RateLimiter::keyed(
                Quota::per_second(NonZeroU32::new(5).unwrap())
                    .allow_burst(NonZeroU32::new(100).unwrap()),
            ),
            known: Mutex::new(HashMap::new()),
        }
    }

    fn is_known(&self, presented: &str) -> bool {
        let known = self.known.lock().unwrap_or_else(|e| e.into_inner());
        known.get(presented).is_some_and(|at| at.elapsed() < KNOWN_FOR)
    }

    /// Before the lookup: unless the credential resolved recently, a cell of
    /// its address's bucket; then its own. The address first: each made-up
    /// credential is a new key in the `worker` limiter, kept until the sweep,
    /// so charged first, a refused flood from one address still grew memory
    /// by an entry a request (150 MB a million) with no database work to slow
    /// it down.
    pub fn admit(&self, worker: &Keyed, presented: &str, address: IpAddr) -> Result<(), AppError> {
        if !self.is_known(presented) {
            check(&self.unknown_per_address, &format!("ip:{address}"))?;
        }
        check(worker, presented)
    }

    /// After a lookup that resolved.
    pub fn remember(&self, presented: &str) {
        let mut known = self.known.lock().unwrap_or_else(|e| e.into_inner());
        if known.len() < MAX_KNOWN || known.contains_key(presented) {
            known.insert(presented.to_owned(), Instant::now());
        }
    }

    fn retain_recent(&self) {
        self.unknown_per_address.retain_recent();
        self.known
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|_, at| at.elapsed() < KNOWN_FOR);
    }
}

impl RateLimiters {
    pub fn new() -> Self {
        let per_hour = Quota::per_hour(NonZeroU32::new(10).unwrap());
        let per_second = Quota::per_second(NonZeroU32::new(1).unwrap())
            .allow_burst(NonZeroU32::new(5).unwrap());
        let unregistered = Quota::per_second(NonZeroU32::new(5).unwrap())
            .allow_burst(NonZeroU32::new(30).unwrap());
        let resets_per_hour = Quota::per_hour(NonZeroU32::new(5).unwrap());
        let logins_per_minute = Quota::per_minute(NonZeroU32::new(10).unwrap());
        let account_logins_per_minute = Quota::per_minute(NonZeroU32::new(100).unwrap());
        // The burst is the key cap: setting up a machine per key at once is
        // not held back. What refills slowly is churn -- revoking a key and
        // making another, a fresh bucket each time.
        let keys_per_hour = Quota::per_hour(NonZeroU32::new(10).unwrap())
            .allow_burst(NonZeroU32::new(100).unwrap());
        Self {
            register: Arc::new(RateLimiter::keyed(per_hour)),
            worker: Arc::new(RateLimiter::keyed(per_second)),
            unregistered_worker: Arc::new(RateLimiter::keyed(unregistered)),
            reset: Arc::new(RateLimiter::keyed(resets_per_hour)),
            login: Arc::new(RateLimiter::keyed(logins_per_minute)),
            login_account: Arc::new(RateLimiter::keyed(account_logins_per_minute)),
            key_creation: Arc::new(RateLimiter::keyed(keys_per_hour)),
            worker_credentials: Arc::new(CredentialGate::new()),
            redeem: Arc::new(RateLimiter::keyed(Quota::per_minute(NonZeroU32::new(20).unwrap()))),
        }
    }

    /// Drop the buckets that have been full (and so idle) long enough to be
    /// indistinguishable from a caller that has never been seen.
    ///
    /// `governor`'s keyed limiters keep one entry per key forever otherwise,
    /// and every key here comes from the outside: a worker UUID, a client
    /// address, a username tried at the login form, an address typed into
    /// password reset. A long-running process accumulates one entry per
    /// distinct value anyone has ever sent it, which is unbounded memory growth
    /// driven by unauthenticated input rather than by how many contributors
    /// there actually are. Forgetting a full bucket changes no decision: the
    /// next request rebuilds it full.
    pub fn retain_recent(&self) {
        for limiter in [
            &self.register,
            &self.worker,
            &self.unregistered_worker,
            &self.reset,
            &self.login,
            &self.login_account,
            &self.key_creation,
            &self.redeem,
        ] {
            limiter.retain_recent();
        }
        self.worker_credentials.retain_recent();
    }
}

impl Default for RateLimiters {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns 429 with a `Retry-After` header when the bucket is empty.
/// Keys longer than this are kept as a digest; see [`check`].
const MAX_KEY_BYTES: usize = 128;

pub fn check(limiter: &Keyed, key: &str) -> Result<(), AppError> {
    // Bounded: a key is kept until the next sweep, and some are built from
    // what a caller sends -- a username tried, an address asked for -- which
    // could be megabytes each. A long key is kept as its digest.
    let key = if key.len() > MAX_KEY_BYTES {
        use sha2::Digest;
        format!("sha256:{}", hex::encode(sha2::Sha256::digest(key.as_bytes())))
    } else {
        key.to_string()
    };
    match limiter.check_key(&key) {
        Ok(()) => Ok(()),
        // `governor` tells us exactly how long the caller has to wait; rounding up
        // to the next whole second is what `Retry-After` can express.
        Err(negative) => Err(AppError::rate_limited(
            negative.wait_time_from(governor::clock::Clock::now(&DefaultClock::default()))
                .as_secs()
                .max(1),
        )),
    }
}

#[cfg(test)]
mod key_tests {
    use super::*;

    /// A-WORKER-16 (the gate): a credential's own bucket is charged before
    /// its lookup; unknown credentials also pay their address's, and once
    /// that is spent they are refused, while a credential that resolved is
    /// not -- whatever its address's neighbours did.
    #[test]
    fn unknown_credentials_pay_their_address_and_known_ones_do_not() {
        let gate = CredentialGate::new();
        let worker: Keyed = RateLimiter::keyed(
            Quota::per_second(NonZeroU32::new(1).unwrap()).allow_burst(NonZeroU32::new(5).unwrap()),
        );
        let shared: IpAddr = "192.0.2.50".parse().unwrap();

        assert!(gate.admit(&worker, "a:real", shared).is_ok());
        gate.remember("a:real");

        let mut refused = None;
        for i in 0..200 {
            if let Err(e) = gate.admit(&worker, &format!("k:made-up-{i}"), shared) {
                refused = Some(i);
                assert_eq!(e.status, axum::http::StatusCode::TOO_MANY_REQUESTS);
                break;
            }
        }
        assert!(refused.is_some_and(|i| (99..=101).contains(&i)), "{refused:?}");
        assert!(gate.admit(&worker, "a:real", shared).is_ok(), "the known worker is not refused");
        assert!(
            gate.admit(&worker, "k:fresh", "192.0.2.51".parse().unwrap()).is_ok(),
            "another address"
        );

        // A refused flood leaves nothing behind per credential: the address
        // refused it before its own bucket was made.
        let before = worker.len();
        for i in 0..1000 {
            assert!(gate.admit(&worker, &format!("k:flood-{i}"), shared).is_err());
        }
        assert_eq!(worker.len(), before, "refused credentials were given buckets");

        // The credential's own bucket holds whether or not it is known.
        let mut own = 0;
        while gate.admit(&worker, "a:real", shared).is_ok() {
            own += 1;
            assert!(own < 10);
        }
    }

    /// A caller-supplied key is kept as a digest past `MAX_KEY_BYTES`: a
    /// megabyte "username" held a megabyte in the limiter until its sweep.
    /// The same long key still lands in the same bucket.
    #[test]
    fn a_long_key_is_one_bucket_kept_small() {
        let limiter: Keyed = RateLimiter::keyed(Quota::per_hour(NonZeroU32::new(1).unwrap()));
        let long = format!("user:{}", "x".repeat(1 << 20));
        assert!(check(&limiter, &long).is_ok());
        assert!(check(&limiter, &long).is_err(), "the same key, the same bucket");
        assert!(check(&limiter, &format!("{long}y")).is_ok(), "another key, another");
        assert!(limiter.len() == 2);
    }
}
