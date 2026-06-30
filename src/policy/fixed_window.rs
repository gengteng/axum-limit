use super::{RateLimitPolicy, RateLimitState};
use crate::quota::Quota;
use crate::snapshot::RateLimitSnapshot;
use std::time::{Duration, Instant};

/// Fixed window rate limiting policy.
///
/// Allows up to `quota.max` requests per window of `quota.per`. The counter resets
/// when a new window starts.
#[derive(Debug, Clone, Copy, Default)]
pub struct FixedWindowPolicy;

/// Fixed window state for a single key.
#[derive(Debug, Clone)]
pub struct FixedWindowState {
    count: usize,
    window_start: Instant,
    window_size: Duration,
    max: usize,
}

impl RateLimitPolicy for FixedWindowPolicy {
    type State = FixedWindowState;

    fn create_state(quota: Quota) -> Self::State {
        FixedWindowState::new_at(quota, Instant::now())
    }
}

impl FixedWindowState {
    pub(crate) fn new_at(quota: Quota, now: Instant) -> Self {
        Self {
            count: 0,
            window_start: now,
            window_size: quota.per(),
            max: quota.max.max(1),
        }
    }

    fn maybe_reset_window(&mut self, now: Instant) {
        if now.duration_since(self.window_start) >= self.window_size {
            self.count = 0;
            self.window_start = now;
        }
    }

    fn reset_at(&self) -> Instant {
        self.window_start + self.window_size
    }
}

impl RateLimitState for FixedWindowState {
    fn try_acquire(&mut self, now: Instant) -> RateLimitSnapshot {
        self.maybe_reset_window(now);
        let limit = self.max;

        if self.count < limit {
            self.count += 1;
            RateLimitSnapshot {
                allowed: true,
                limit,
                remaining: limit.saturating_sub(self.count),
                reset_at: self.reset_at(),
            }
        } else {
            RateLimitSnapshot {
                allowed: false,
                limit,
                remaining: 0,
                reset_at: self.reset_at(),
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

    fn window(quota: Quota, start: Instant) -> FixedWindowState {
        FixedWindowState::new_at(quota, start)
    }

    #[test]
    fn allows_up_to_max_requests_in_window() {
        let start = at(0);
        let mut window = window(Quota::new(3, 1000), start);

        assert!(window.try_acquire(start).allowed);
        assert!(window.try_acquire(start + Duration::from_millis(100)).allowed);
        assert!(window.try_acquire(start + Duration::from_millis(500)).allowed);
        assert!(!window.try_acquire(start + Duration::from_millis(900)).allowed);
    }

    #[test]
    fn resets_after_window_expires() {
        let start = at(0);
        let mut window = window(Quota::new(2, 1000), start);

        assert!(window.try_acquire(start).allowed);
        assert!(window.try_acquire(start + Duration::from_millis(100)).allowed);
        assert!(!window.try_acquire(start + Duration::from_millis(200)).allowed);

        assert!(window.try_acquire(start + Duration::from_millis(1000)).allowed);
        assert!(window.try_acquire(start + Duration::from_millis(1100)).allowed);
        assert!(!window.try_acquire(start + Duration::from_millis(1200)).allowed);
    }

    #[test]
    fn window_boundary_is_inclusive_of_new_window() {
        let start = at(0);
        let mut window = window(Quota::new(1, 500), start);

        assert!(window.try_acquire(start).allowed);
        assert!(!window.try_acquire(start + Duration::from_millis(100)).allowed);
        assert!(window.try_acquire(start + Duration::from_millis(500)).allowed);
    }

    #[test]
    fn ignores_burst_setting() {
        let start = at(0);
        let mut window = window(Quota::with_burst(2, 1000, 10), start);

        assert!(window.try_acquire(start).allowed);
        assert!(window.try_acquire(start).allowed);
        assert!(!window.try_acquire(start).allowed);
    }
}
