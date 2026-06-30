use super::{PolicyState, RateLimitPolicy};
use crate::codec::{decode_json, encode_json, CodecError};
use crate::quota::Quota;
use crate::snapshot::RateLimitSnapshot;
use crate::time::saturating_sub_ms;
use serde::{Deserialize, Serialize};

/// Token bucket rate limiting policy.
///
/// Allows bursts up to the configured capacity while replenishing tokens smoothly
/// at `quota.max` requests per `quota.per`.
#[derive(Debug, Clone, Copy, Default)]
pub struct TokenBucketPolicy;

impl RateLimitPolicy for TokenBucketPolicy {
    type State = TokenBucketState;
    const STATE_ID: &'static str = "token_bucket";
}

/// Token bucket state for a single key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenBucketState {
    tokens: usize,
    max_tokens: usize,
    last_refill_time_ms: u64,
    ns_per_token: u64,
}

impl TokenBucketState {
    pub(crate) fn new_at(quota: Quota, now_ms: u64) -> Self {
        let max_tokens = quota.burst().max(1);
        let rate = quota.max.max(1);
        let ns_per_token = (quota.per_ms.saturating_mul(1_000_000) / rate as u64).max(1);

        Self {
            tokens: max_tokens,
            max_tokens,
            last_refill_time_ms: now_ms,
            ns_per_token,
        }
    }

    fn refill(&mut self, now_ms: u64) {
        let elapsed_ms = saturating_sub_ms(now_ms, self.last_refill_time_ms);
        let elapsed_ns = elapsed_ms.saturating_mul(1_000_000);

        if elapsed_ns >= self.ns_per_token {
            let new_tokens = (elapsed_ns / self.ns_per_token) as usize;
            self.tokens = (self.tokens + new_tokens).min(self.max_tokens);
            let remainder_ns = elapsed_ns % self.ns_per_token;
            self.last_refill_time_ms = now_ms.saturating_sub(remainder_ns / 1_000_000);
        }
    }

    fn next_token_at_ms(&self, now_ms: u64) -> u64 {
        if self.tokens > 0 {
            now_ms
        } else {
            now_ms
                .saturating_add(self.ns_per_token / 1_000_000)
                .max(now_ms + 1)
        }
    }
}

impl PolicyState for TokenBucketState {
    fn try_acquire(&mut self, now_ms: u64) -> RateLimitSnapshot {
        self.refill(now_ms);
        let limit = self.max_tokens;

        if self.tokens > 0 {
            self.tokens -= 1;
            RateLimitSnapshot {
                allowed: true,
                limit,
                remaining: self.tokens,
                reset_at_ms: self.next_token_at_ms(now_ms),
            }
        } else {
            RateLimitSnapshot {
                allowed: false,
                limit,
                remaining: 0,
                reset_at_ms: self.next_token_at_ms(now_ms),
            }
        }
    }

    fn encode(&self) -> Result<Vec<u8>, CodecError> {
        encode_json(self)
    }

    fn decode(bytes: &[u8], _quota: Quota) -> Result<Self, CodecError> {
        decode_json(bytes)
    }

    fn create(quota: Quota, now_ms: u64) -> Self {
        Self::new_at(quota, now_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE_MS: u64 = 1_000_000;

    fn at(offset_ms: u64) -> u64 {
        BASE_MS + offset_ms
    }

    fn bucket(quota: Quota) -> TokenBucketState {
        TokenBucketState::new_at(quota, BASE_MS)
    }

    #[test]
    fn allows_burst_up_to_capacity() {
        let mut bucket = bucket(Quota::with_burst(5, 1000, 10));

        for _ in 0..10 {
            assert!(bucket.try_acquire(at(0)).allowed);
        }
        assert!(!bucket.try_acquire(at(0)).allowed);
    }

    #[test]
    fn refills_at_sustained_rate() {
        let mut bucket = bucket(Quota::new(5, 1000));

        for _ in 0..5 {
            assert!(bucket.try_acquire(at(0)).allowed);
        }
        assert!(!bucket.try_acquire(at(0)).allowed);

        assert!(!bucket.try_acquire(at(199)).allowed);
        assert!(bucket.try_acquire(at(200)).allowed);
        assert!(!bucket.try_acquire(at(250)).allowed);
    }

    #[test]
    fn refills_full_quota_after_period() {
        let mut bucket = bucket(Quota::new(5, 1000));

        for _ in 0..5 {
            assert!(bucket.try_acquire(at(0)).allowed);
        }
        assert!(!bucket.try_acquire(at(0)).allowed);

        for _ in 0..5 {
            assert!(bucket.try_acquire(at(1000)).allowed);
        }
        assert!(!bucket.try_acquire(at(1000)).allowed);
    }

    #[test]
    fn round_trips_through_json() {
        let state = bucket(Quota::new(5, 1000));
        let encoded = state.encode().expect("encode");
        let decoded = TokenBucketState::decode(&encoded, Quota::new(5, 1000)).expect("decode");
        assert_eq!(decoded.tokens, state.tokens);
    }
}
