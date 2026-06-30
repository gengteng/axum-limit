use super::{RateLimitPolicy, RateLimitState};
use crate::quota::Quota;
use std::time::{Duration, Instant};

/// Token bucket rate limiting policy.
///
/// Allows bursts up to the configured capacity while replenishing tokens smoothly
/// at `quota.max` requests per `quota.per`.
#[derive(Debug, Clone, Copy, Default)]
pub struct TokenBucketPolicy;

/// Token bucket state for a single key.
#[derive(Debug, Clone)]
pub struct TokenBucketState {
    tokens: usize,
    max_tokens: usize,
    last_refill_time: Instant,
    ns_per_token: u64,
}

impl RateLimitPolicy for TokenBucketPolicy {
    type State = TokenBucketState;

    fn create_state(quota: Quota) -> Self::State {
        TokenBucketState::new(quota)
    }
}

impl TokenBucketState {
    fn new(quota: Quota) -> Self {
        Self::new_at(quota, Instant::now())
    }

    fn new_at(quota: Quota, now: Instant) -> Self {
        let max_tokens = quota.burst().max(1);
        let rate = quota.max.max(1);
        let ns_per_token = (quota.per_ms.saturating_mul(1_000_000) / rate as u64).max(1);

        Self {
            tokens: max_tokens,
            max_tokens,
            last_refill_time: now,
            ns_per_token,
        }
    }

    fn refill(&mut self, now: Instant) {
        let elapsed = now.duration_since(self.last_refill_time);
        let elapsed_ns = elapsed.as_nanos() as u64;

        if elapsed_ns >= self.ns_per_token {
            let new_tokens = (elapsed_ns / self.ns_per_token) as usize;
            self.tokens = (self.tokens + new_tokens).min(self.max_tokens);
            let remainder_ns = elapsed_ns % self.ns_per_token;
            self.last_refill_time = now - Duration::from_nanos(remainder_ns);
        }
    }
}

impl RateLimitState for TokenBucketState {
    fn try_acquire(&mut self, now: Instant) -> bool {
        self.refill(now);
        if self.tokens > 0 {
            self.tokens -= 1;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: u64) -> Instant {
        Instant::now() - Duration::from_secs(60) + Duration::from_millis(ms)
    }

    fn bucket(quota: Quota, start: Instant) -> TokenBucketState {
        TokenBucketState::new_at(quota, start)
    }

    #[test]
    fn allows_burst_up_to_capacity() {
        let start = at(0);
        let mut bucket = bucket(Quota::with_burst(5, 1000, 10), start);

        for _ in 0..10 {
            assert!(bucket.try_acquire(start));
        }
        assert!(!bucket.try_acquire(start));
    }

    #[test]
    fn refills_at_sustained_rate() {
        let start = at(0);
        let mut bucket = bucket(Quota::new(5, 1000), start);

        for _ in 0..5 {
            assert!(bucket.try_acquire(start));
        }
        assert!(!bucket.try_acquire(start));

        // One token every 200ms for 5 req/s.
        assert!(!bucket.try_acquire(start + Duration::from_millis(199)));
        assert!(bucket.try_acquire(start + Duration::from_millis(200)));
        assert!(!bucket.try_acquire(start + Duration::from_millis(250)));
    }

    #[test]
    fn refills_full_quota_after_period() {
        let start = at(0);
        let mut bucket = bucket(Quota::new(5, 1000), start);

        for _ in 0..5 {
            assert!(bucket.try_acquire(start));
        }
        assert!(!bucket.try_acquire(start));

        for _ in 0..5 {
            assert!(bucket.try_acquire(start + Duration::from_secs(1)));
        }
        assert!(!bucket.try_acquire(start + Duration::from_secs(1)));
    }

    #[test]
    fn never_exceeds_capacity() {
        let start = at(0);
        let mut bucket = bucket(Quota::new(5, 1000), start);

        for _ in 0..5 {
            assert!(bucket.try_acquire(start));
        }

        // Wait long enough to fully refill multiple times.
        for _ in 0..5 {
            assert!(bucket.try_acquire(start + Duration::from_secs(10)));
        }
        assert!(!bucket.try_acquire(start + Duration::from_secs(10)));
    }

    #[test]
    fn separate_burst_and_rate() {
        let start = at(0);
        let mut bucket = bucket(Quota::with_burst(2, 1000, 6), start);

        for _ in 0..6 {
            assert!(bucket.try_acquire(start));
        }
        assert!(!bucket.try_acquire(start));

        // Sustained rate is 2/s, so one token after 500ms.
        assert!(bucket.try_acquire(start + Duration::from_millis(500)));
        assert!(!bucket.try_acquire(start + Duration::from_millis(500)));
    }
}
