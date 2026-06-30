use super::{RateLimitPolicy, RateLimitState};
use crate::quota::Quota;
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
        FixedWindowState::new(quota)
    }
}

impl FixedWindowState {
    fn new(quota: Quota) -> Self {
        Self::new_at(quota, Instant::now())
    }

    fn new_at(quota: Quota, now: Instant) -> Self {
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
}

impl RateLimitState for FixedWindowState {
    fn try_acquire(&mut self, now: Instant) -> bool {
        self.maybe_reset_window(now);
        if self.count < self.max {
            self.count += 1;
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

    fn window(quota: Quota, start: Instant) -> FixedWindowState {
        FixedWindowState::new_at(quota, start)
    }

    #[test]
    fn allows_up_to_max_requests_in_window() {
        let start = at(0);
        let mut window = window(Quota::new(3, 1000), start);

        assert!(window.try_acquire(start));
        assert!(window.try_acquire(start + Duration::from_millis(100)));
        assert!(window.try_acquire(start + Duration::from_millis(500)));
        assert!(!window.try_acquire(start + Duration::from_millis(900)));
    }

    #[test]
    fn resets_after_window_expires() {
        let start = at(0);
        let mut window = window(Quota::new(2, 1000), start);

        assert!(window.try_acquire(start));
        assert!(window.try_acquire(start + Duration::from_millis(100)));
        assert!(!window.try_acquire(start + Duration::from_millis(200)));

        // New window begins at t=1000ms.
        assert!(window.try_acquire(start + Duration::from_millis(1000)));
        assert!(window.try_acquire(start + Duration::from_millis(1100)));
        assert!(!window.try_acquire(start + Duration::from_millis(1200)));
    }

    #[test]
    fn window_boundary_is_inclusive_of_new_window() {
        let start = at(0);
        let mut window = window(Quota::new(1, 500), start);

        assert!(window.try_acquire(start));
        assert!(!window.try_acquire(start + Duration::from_millis(100)));

        // Exactly at boundary, counter resets.
        assert!(window.try_acquire(start + Duration::from_millis(500)));
    }

    #[test]
    fn ignores_burst_setting() {
        let start = at(0);
        let mut window = window(Quota::with_burst(2, 1000, 10), start);

        assert!(window.try_acquire(start));
        assert!(window.try_acquire(start));
        assert!(!window.try_acquire(start));
    }
}
