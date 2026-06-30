//! Rate limiting policy implementations.

mod fixed_window;
mod sliding_window;
mod token_bucket;

#[cfg(test)]
mod proptest_policies;

pub use fixed_window::FixedWindowPolicy;
pub use sliding_window::SlidingWindowPolicy;
pub use token_bucket::TokenBucketPolicy;

use crate::codec::CodecError;
use crate::quota::Quota;
use crate::snapshot::RateLimitSnapshot;

/// Serializable and executable state for a rate limiting policy.
pub trait PolicyState: Send {
    /// Attempts to consume one unit of quota at `now_ms`.
    fn try_acquire(&mut self, now_ms: u64) -> RateLimitSnapshot;

    /// Encodes the state for storage backends.
    fn encode(&self) -> Result<Vec<u8>, CodecError>;

    /// Decodes state previously stored by [`Self::encode`].
    fn decode(bytes: &[u8], quota: Quota) -> Result<Self, CodecError>
    where
        Self: Sized;

    /// Creates a fresh state instance for a new storage key.
    fn create(quota: Quota, now_ms: u64) -> Self;
}

/// Creates per-key state for a specific rate limiting algorithm.
pub trait RateLimitPolicy: Send + Sync + 'static {
    /// Per-key algorithm state stored by [`crate::backend::RateLimitBackend`].
    type State: PolicyState;

    /// Stable identifier used in storage keys for this policy.
    const STATE_ID: &'static str;
}
