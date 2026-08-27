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
    /// 1 request per second per worker identity, applied to task/result/heartbeat.
    pub worker: Arc<Keyed>,
}

impl RateLimiters {
    pub fn new() -> Self {
        let per_hour = Quota::per_hour(NonZeroU32::new(10).unwrap());
        let per_second = Quota::per_second(NonZeroU32::new(1).unwrap())
            .allow_burst(NonZeroU32::new(5).unwrap());
        Self {
            register: Arc::new(RateLimiter::keyed(per_hour)),
            worker: Arc::new(RateLimiter::keyed(per_second)),
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
