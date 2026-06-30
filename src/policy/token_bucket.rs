use super::{RateLimitPolicy, RateLimitState};
use crate::quota::Quota;
use crate::snapshot::RateLimitSnapshot;
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
        TokenBucketState::new_at(quota, Instant::now())
    }
}

impl TokenBucketState {
    pub(crate) fn new_at(quota: Quota, now: Instant) -> Self {
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

    fn next_token_at(&self, now: Instant) -> Instant {
        if self.tokens > 0 {
            now
        } else {
            now + Duration::from_nanos(self.ns_per_token)
        }
    }
}

impl RateLimitState for TokenBucketState {
    fn try_acquire(&mut self, now: Instant) -> RateLimitSnapshot {
        self.refill(now);
        let limit = self.max_tokens;

        if self.tokens > 0 {
            self.tokens -= 1;
            RateLimitSnapshot {
                allowed: true,
                limit,
                remaining: self.tokens,
                reset_at: self.next_token_at(now),
            }
        } else {
            RateLimitSnapshot {
                allowed: false,
                limit,
                remaining: 0,
                reset_at: self.next_token_at(now),
            }
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
            assert!(bucket.try_acquire(start).allowed);
        }
        assert!(!bucket.try_acquire(start).allowed);
    }

    #[test]
    fn refills_at_sustained_rate() {
        let start = at(0);
        let mut bucket = bucket(Quota::new(5, 1000), start);

        for _ in 0..5 {
            assert!(bucket.try_acquire(start).allowed);
        }
        assert!(!bucket.try_acquire(start).allowed);

        assert!(!bucket.try_acquire(start + Duration::from_millis(199)).allowed);
        assert!(bucket.try_acquire(start + Duration::from_millis(200)).allowed);
        assert!(!bucket.try_acquire(start + Duration::from_millis(250)).allowed);
    }

    #[test]
    fn refills_full_quota_after_period() {
        let start = at(0);
        let mut bucket = bucket(Quota::new(5, 1000), start);

        for _ in 0..5 {
            assert!(bucket.try_acquire(start).allowed);
        }
        assert!(!bucket.try_acquire(start).allowed);

        for _ in 0..5 {
            assert!(bucket.try_acquire(start + Duration::from_secs(1)).allowed);
        }
        assert!(!bucket.try_acquire(start + Duration::from_secs(1)).allowed);
    }

    #[test]
    fn never_exceeds_capacity() {
        let start = at(0);
        let mut bucket = bucket(Quota::new(5, 1000), start);

        for _ in 0..5 {
            assert!(bucket.try_acquire(start).allowed);
        }

        for _ in 0..5 {
            assert!(bucket.try_acquire(start + Duration::from_secs(10)).allowed);
        }
        assert!(!bucket.try_acquire(start + Duration::from_secs(10)).allowed);
    }

    #[test]
    fn separate_burst_and_rate() {
        let start = at(0);
        let mut bucket = bucket(Quota::with_burst(2, 1000, 6), start);

        for _ in 0..6 {
            assert!(bucket.try_acquire(start).allowed);
        }
        assert!(!bucket.try_acquire(start).allowed);

        assert!(bucket.try_acquire(start + Duration::from_millis(500)).allowed);
        assert!(!bucket.try_acquire(start + Duration::from_millis(500)).allowed);
    }
}
