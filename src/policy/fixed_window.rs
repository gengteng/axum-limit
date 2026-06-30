use super::{PolicyState, RateLimitPolicy};
use crate::codec::{decode_json, encode_json, CodecError};
use crate::quota::Quota;
use crate::snapshot::RateLimitSnapshot;
use crate::time::saturating_sub_ms;
use serde::{Deserialize, Serialize};

/// Fixed window rate limiting policy.
///
/// Allows up to `quota.max` requests per window of `quota.per`. The counter resets
/// when a new window starts.
#[derive(Debug, Clone, Copy, Default)]
pub struct FixedWindowPolicy;

impl RateLimitPolicy for FixedWindowPolicy {
    type State = FixedWindowState;
    const STATE_ID: &'static str = "fixed_window";
}

/// Fixed window state for a single key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixedWindowState {
    count: usize,
    window_start_ms: u64,
    window_size_ms: u64,
    max: usize,
}

impl FixedWindowState {
    pub(crate) fn new_at(quota: Quota, now_ms: u64) -> Self {
        Self {
            count: 0,
            window_start_ms: now_ms,
            window_size_ms: quota.per_ms,
            max: quota.max.max(1),
        }
    }

    fn maybe_reset_window(&mut self, now_ms: u64) {
        if saturating_sub_ms(now_ms, self.window_start_ms) >= self.window_size_ms {
            self.count = 0;
            self.window_start_ms = now_ms;
        }
    }

    fn reset_at_ms(&self) -> u64 {
        self.window_start_ms.saturating_add(self.window_size_ms)
    }
}

impl PolicyState for FixedWindowState {
    fn try_acquire(&mut self, now_ms: u64) -> RateLimitSnapshot {
        self.maybe_reset_window(now_ms);
        let limit = self.max;

        if self.count < limit {
            self.count += 1;
            RateLimitSnapshot {
                allowed: true,
                limit,
                remaining: limit.saturating_sub(self.count),
                reset_at_ms: self.reset_at_ms(),
            }
        } else {
            RateLimitSnapshot {
                allowed: false,
                limit,
                remaining: 0,
                reset_at_ms: self.reset_at_ms(),
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

    const BASE_MS: u64 = 2_000_000;

    fn at(offset_ms: u64) -> u64 {
        BASE_MS + offset_ms
    }

    fn window(quota: Quota) -> FixedWindowState {
        FixedWindowState::new_at(quota, BASE_MS)
    }

    #[test]
    fn allows_up_to_max_requests_in_window() {
        let mut window = window(Quota::new(3, 1000));

        assert!(window.try_acquire(at(0)).allowed);
        assert!(window.try_acquire(at(100)).allowed);
        assert!(window.try_acquire(at(500)).allowed);
        assert!(!window.try_acquire(at(900)).allowed);
    }

    #[test]
    fn resets_after_window_expires() {
        let mut window = window(Quota::new(2, 1000));

        assert!(window.try_acquire(at(0)).allowed);
        assert!(window.try_acquire(at(100)).allowed);
        assert!(!window.try_acquire(at(200)).allowed);

        assert!(window.try_acquire(at(1000)).allowed);
        assert!(window.try_acquire(at(1100)).allowed);
        assert!(!window.try_acquire(at(1200)).allowed);
    }

    #[test]
    fn round_trips_through_json() {
        let mut state = window(Quota::new(3, 1000));
        assert!(state.try_acquire(at(0)).allowed);
        assert!(state.try_acquire(at(100)).allowed);

        let encoded = state.encode().expect("encode");
        let mut decoded = FixedWindowState::decode(&encoded, Quota::new(3, 1000)).expect("decode");

        assert!(decoded.try_acquire(at(200)).allowed);
        assert!(!decoded.try_acquire(at(300)).allowed);
    }
}
