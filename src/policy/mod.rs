//! Rate limiting policy implementations.

mod fixed_window;
mod sliding_window;
mod token_bucket;

#[cfg(test)]
mod proptest_policies;

pub use fixed_window::FixedWindowPolicy;
pub use sliding_window::SlidingWindowPolicy;
pub use token_bucket::TokenBucketPolicy;

use crate::quota::Quota;
use crate::snapshot::RateLimitSnapshot;
use std::time::Instant;

/// Creates per-key state for a specific rate limiting algorithm.
pub trait RateLimitPolicy: Send + Sync + 'static {
    /// Per-key algorithm state stored in [`crate::LimitState`].
    type State: RateLimitState;

    /// Creates a fresh state instance for a key using the given quota.
    fn create_state(quota: Quota) -> Self::State;
}

/// Algorithm-specific mutable state for a single rate limit key.
pub trait RateLimitState: Send {
    /// Attempts to consume one unit of quota at `now`.
    fn try_acquire(&mut self, now: Instant) -> RateLimitSnapshot;
}
