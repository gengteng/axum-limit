use super::{RateLimitPolicy, RateLimitState};
use crate::quota::Quota;
use crate::snapshot::RateLimitSnapshot;
use std::time::{Duration, Instant};

/// Sliding window counter rate limiting policy.
///
/// Estimates request volume in a rolling window by weighting the previous window,
/// avoiding the sharp reset spikes of fixed windows while remaining memory efficient.
#[derive(Debug, Clone, Copy, Default)]
pub struct SlidingWindowPolicy;

/// Sliding window counter state for a single key.
#[derive(Debug, Clone)]
pub struct SlidingWindowState {
    current_count: usize,
    previous_count: usize,
    window_start: Instant,
    window_size: Duration,
    max: usize,
}

impl RateLimitPolicy for SlidingWindowPolicy {
    type State = SlidingWindowState;

    fn create_state(quota: Quota) -> Self::State {
        SlidingWindowState::new_at(quota, Instant::now())
    }
}

impl SlidingWindowState {
    pub(crate) fn new_at(quota: Quota, now: Instant) -> Self {
        Self {
            current_count: 0,
            previous_count: 0,
            window_start: now,
            window_size: quota.per(),
            max: quota.max.max(1),
        }
    }

    fn roll_window(&mut self, now: Instant) {
        while now.duration_since(self.window_start) >= self.window_size {
            self.previous_count = self.current_count;
            self.current_count = 0;
            self.window_start += self.window_size;
        }
    }

    fn estimated_count(&self, now: Instant) -> f64 {
        let elapsed = now.duration_since(self.window_start);
        let window_ms = self.window_size.as_millis().max(1) as f64;
        let weight = 1.0 - (elapsed.as_millis() as f64 / window_ms);
        self.previous_count as f64 * weight + self.current_count as f64
    }

    fn reset_at(&self) -> Instant {
        self.window_start + self.window_size
    }
}

impl RateLimitState for SlidingWindowState {
    fn try_acquire(&mut self, now: Instant) -> RateLimitSnapshot {
        self.roll_window(now);
        let limit = self.max;
        let estimated = self.estimated_count(now);

        if estimated >= limit as f64 {
            return RateLimitSnapshot {
                allowed: false,
                limit,
                remaining: 0,
                reset_at: self.reset_at(),
            };
        }

        self.current_count += 1;
        let remaining = limit.saturating_sub(self.estimated_count(now).ceil() as usize);

        RateLimitSnapshot {
            allowed: true,
            limit,
            remaining,
            reset_at: self.reset_at(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: u64) -> Instant {
        Instant::now() - Duration::from_secs(120) + Duration::from_millis(ms)
    }

    fn window(quota: Quota, start: Instant) -> SlidingWindowState {
        SlidingWindowState::new_at(quota, start)
    }

    #[test]
    fn allows_up_to_max_requests() {
        let start = at(0);
        let mut state = window(Quota::new(3, 1000), start);

        for _ in 0..3 {
            assert!(state.try_acquire(start).allowed);
        }
        assert!(!state.try_acquire(start).allowed);
    }

    #[test]
    fn smooths_boundary_between_windows() {
        let start = at(0);
        let mut state = window(Quota::new(4, 1000), start);

        for _ in 0..4 {
            assert!(state.try_acquire(start).allowed);
        }
        assert!(!state.try_acquire(start + Duration::from_millis(100)).allowed);

        // Previous window still contributes weight at the boundary.
        assert!(!state.try_acquire(start + Duration::from_millis(1000)).allowed);

        // After a full quiet window, quota is available again.
        for _ in 0..4 {
            assert!(state.try_acquire(start + Duration::from_millis(2000)).allowed);
        }
    }

    #[test]
    fn remaining_never_exceeds_limit() {
        let start = at(0);
        let mut state = window(Quota::new(5, 1000), start);

        for ms in (0..2500).step_by(50) {
            let snapshot = state.try_acquire(start + Duration::from_millis(ms));
            assert!(snapshot.remaining <= snapshot.limit);
        }
    }
}
