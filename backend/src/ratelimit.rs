use crate::error::AppError;
use governor::clock::DefaultClock;
use governor::state::keyed::DefaultKeyedStateStore;
use governor::{Quota, RateLimiter};
use std::num::NonZeroU32;
use std::sync::Arc;

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
        Self {
            register: Arc::new(RateLimiter::keyed(per_hour)),
            worker: Arc::new(RateLimiter::keyed(per_second)),
            unregistered_worker: Arc::new(RateLimiter::keyed(unregistered)),
            reset: Arc::new(RateLimiter::keyed(resets_per_hour)),
            login: Arc::new(RateLimiter::keyed(logins_per_minute)),
            login_account: Arc::new(RateLimiter::keyed(account_logins_per_minute)),
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
        ] {
            limiter.retain_recent();
        }
    }
}

impl Default for RateLimiters {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns 429 with a `Retry-After` header when the bucket is empty.
pub fn check(limiter: &Keyed, key: &str) -> Result<(), AppError> {
    match limiter.check_key(&key.to_string()) {
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
