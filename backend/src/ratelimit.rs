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
    /// Worker credentials that matched nothing, per client address: see
    /// [`MissGate`].
    pub worker_misses: Arc<MissGate>,
    /// Confirmation and reset links redeemed, per client address: 20 a
    /// minute. The codes are too long to guess, so this is about cost, not
    /// guessing -- both routes are unauthenticated and write on the main pool,
    /// and a reset scores the new password first.
    pub redeem: Arc<Keyed>,
}

/// Worker requests whose API key or `X-Worker-UUID` matched nothing, per
/// client address, and the addresses refused for it.
///
/// Resolving a worker identity is a main-pool query, made before any worker
/// bucket can be charged -- the bucket is the identity's. So a made-up key or
/// UUID cost a query and a 401, unmetered, on the pool that claims and
/// submissions need (`db.rs`). A real worker never misses; an address that
/// misses 30 times in a minute is refused without a query until its bucket
/// refills.
pub struct MissGate {
    misses: Keyed,
    refused_until: Mutex<HashMap<IpAddr, Instant>>,
}

impl MissGate {
    fn new() -> Self {
        Self {
            misses: RateLimiter::keyed(Quota::per_minute(NonZeroU32::new(30).unwrap())),
            refused_until: Mutex::new(HashMap::new()),
        }
    }

    /// Before the lookup: a 429, costing nothing, while `address` is refused.
    pub fn check(&self, address: IpAddr) -> Result<(), AppError> {
        let refused = self.refused_until.lock().unwrap_or_else(|e| e.into_inner());
        match refused.get(&address) {
            Some(until) if *until > Instant::now() => Err(AppError::rate_limited(
                until.saturating_duration_since(Instant::now()).as_secs().max(1),
            )),
            _ => Ok(()),
        }
    }

    /// After a lookup that matched nothing: charged, and once the bucket is
    /// empty the address is refused (and this miss answered 429) until it
    /// refills.
    pub fn record_miss(&self, address: IpAddr) -> Result<(), AppError> {
        match self.misses.check_key(&address.to_string()) {
            Ok(()) => Ok(()),
            Err(negative) => {
                let wait = negative
                    .wait_time_from(governor::clock::Clock::now(&DefaultClock::default()))
                    .max(Duration::from_secs(1));
                self.refused_until
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(address, Instant::now() + wait);
                Err(AppError::rate_limited(wait.as_secs().max(1)))
            }
        }
    }

    fn retain_recent(&self) {
        self.misses.retain_recent();
        let now = Instant::now();
        self.refused_until
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|_, until| *until > now);
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
            worker_misses: Arc::new(MissGate::new()),
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
        self.worker_misses.retain_recent();
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

    /// A-WORKER-16 (the gate): an address whose worker credentials keep matching
    /// nothing is refused before the lookup once its bucket is spent, with a
    /// `Retry-After`; another address is not.
    #[test]
    fn an_address_that_keeps_missing_is_refused_before_the_lookup() {
        let gate = MissGate::new();
        let noisy: IpAddr = "192.0.2.50".parse().unwrap();
        let mut refused = None;
        for _ in 0..31 {
            assert!(gate.check(noisy).is_ok(), "not refused before its bucket is spent");
            if let Err(e) = gate.record_miss(noisy) {
                refused = Some(e);
                break;
            }
        }
        let refused = refused.expect("the thirty-first miss is refused");
        assert_eq!(refused.status, axum::http::StatusCode::TOO_MANY_REQUESTS);
        let early = gate.check(noisy).expect_err("refused without a lookup now");
        assert!(early.retry_after.unwrap() >= 1);
        assert!(gate.check("192.0.2.51".parse().unwrap()).is_ok(), "another address");
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
