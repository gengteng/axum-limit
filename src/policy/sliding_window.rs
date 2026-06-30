use super::{PolicyState, RateLimitPolicy};
use crate::codec::{decode_json, encode_json, CodecError};
use crate::quota::Quota;
use crate::snapshot::RateLimitSnapshot;
use crate::time::saturating_sub_ms;
use serde::{Deserialize, Serialize};

/// Sliding window counter rate limiting policy.
///
/// Estimates request volume in a rolling window by weighting the previous window,
/// avoiding the sharp reset spikes of fixed windows while remaining memory efficient.
#[derive(Debug, Clone, Copy, Default)]
pub struct SlidingWindowPolicy;

impl RateLimitPolicy for SlidingWindowPolicy {
    type State = SlidingWindowState;
    const STATE_ID: &'static str = "sliding_window";
}

/// Sliding window counter state for a single key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlidingWindowState {
    current_count: usize,
    previous_count: usize,
    window_start_ms: u64,
    window_size_ms: u64,
    max: usize,
}

impl SlidingWindowState {
    pub(crate) fn new_at(quota: Quota, now_ms: u64) -> Self {
        Self {
            current_count: 0,
            previous_count: 0,
            window_start_ms: now_ms,
            window_size_ms: quota.per_ms,
            max: quota.max.max(1),
        }
    }

    fn roll_window(&mut self, now_ms: u64) {
        while saturating_sub_ms(now_ms, self.window_start_ms) >= self.window_size_ms {
            self.previous_count = self.current_count;
            self.current_count = 0;
            self.window_start_ms = self.window_start_ms.saturating_add(self.window_size_ms);
        }
    }

    fn estimated_count(&self, now_ms: u64) -> f64 {
        let elapsed_ms = saturating_sub_ms(now_ms, self.window_start_ms);
        let window_ms = self.window_size_ms.max(1) as f64;
        let weight = 1.0 - (elapsed_ms as f64 / window_ms);
        self.previous_count as f64 * weight + self.current_count as f64
    }

    fn reset_at_ms(&self) -> u64 {
        self.window_start_ms.saturating_add(self.window_size_ms)
    }
}

impl PolicyState for SlidingWindowState {
    fn try_acquire(&mut self, now_ms: u64) -> RateLimitSnapshot {
        self.roll_window(now_ms);
        let limit = self.max;
        let estimated = self.estimated_count(now_ms);

        if estimated >= limit as f64 {
            return RateLimitSnapshot {
                allowed: false,
                limit,
                remaining: 0,
                reset_at_ms: self.reset_at_ms(),
            };
        }

        self.current_count += 1;
        let remaining = limit.saturating_sub(self.estimated_count(now_ms).ceil() as usize);

        RateLimitSnapshot {
            allowed: true,
            limit,
            remaining,
            reset_at_ms: self.reset_at_ms(),
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

    const BASE_MS: u64 = 3_000_000;

    fn at(offset_ms: u64) -> u64 {
        BASE_MS + offset_ms
    }

    fn window(quota: Quota) -> SlidingWindowState {
        SlidingWindowState::new_at(quota, BASE_MS)
    }

    #[test]
    fn allows_up_to_max_requests() {
        let mut state = window(Quota::new(3, 1000));

        for _ in 0..3 {
            assert!(state.try_acquire(at(0)).allowed);
        }
        assert!(!state.try_acquire(at(0)).allowed);
    }

    #[test]
    fn smooths_boundary_between_windows() {
        let mut state = window(Quota::new(4, 1000));

        for _ in 0..4 {
            assert!(state.try_acquire(at(0)).allowed);
        }
        assert!(!state.try_acquire(at(100)).allowed);
        assert!(!state.try_acquire(at(1000)).allowed);

        for _ in 0..4 {
            assert!(state.try_acquire(at(2000)).allowed);
        }
    }
}
