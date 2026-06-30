mod memory;

#[cfg(feature = "redis")]
mod redis;

use crate::codec::CodecError;
use crate::policy::{PolicyState, RateLimitPolicy};
use crate::quota::Quota;
use crate::snapshot::RateLimitSnapshot;
use async_trait::async_trait;
use std::error::Error;
use std::fmt::{Display, Formatter, Result as FmtResult};

pub use memory::MemoryBackend;

#[cfg(feature = "redis")]
pub use redis::RedisBackend;

/// Errors returned by storage backends.
#[derive(Debug)]
pub enum BackendError {
    /// Failed to encode or decode policy state.
    Codec(CodecError),
    /// Optimistic transaction failed after too many retries.
    Contention,
    /// Redis returned an error.
    #[cfg(feature = "redis")]
    Redis(::redis::RedisError),
}

impl Display for BackendError {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            BackendError::Codec(error) => write!(f, "{error}"),
            BackendError::Contention => write!(f, "rate limit storage contention"),
            #[cfg(feature = "redis")]
            BackendError::Redis(error) => write!(f, "redis error: {error}"),
        }
    }
}

impl Error for BackendError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            BackendError::Codec(error) => Some(error),
            BackendError::Contention => None,
            #[cfg(feature = "redis")]
            BackendError::Redis(error) => Some(error),
        }
    }
}

impl From<CodecError> for BackendError {
    fn from(value: CodecError) -> Self {
        Self::Codec(value)
    }
}

/// Encodes a subject key into a stable storage representation.
pub trait StorageKey: crate::Key {
    /// Returns a stable string representation for storage backends.
    fn storage_key(&self) -> String;
}

/// Storage backend for rate limit policy state.
///
/// Implement this trait to provide custom backends such as database or Consul storage.
#[async_trait]
pub trait RateLimitBackend: Send + Sync + Clone + 'static {
    /// Error type returned by this backend.
    type Error: Error + Send + Sync + 'static;

    /// Namespace used to isolate keys for this backend instance.
    fn namespace(&self) -> &str;

    /// Atomically loads policy state, applies the rate limit algorithm, and persists the result.
    async fn transact<P>(
        &self,
        storage_key: &str,
        quota: Quota,
        now_ms: u64,
    ) -> Result<RateLimitSnapshot, Self::Error>
    where
        P: RateLimitPolicy;
}

/// Builds a storage key for a subject and quota fingerprint.
pub fn build_storage_key<P>(namespace: &str, subject: &str, quota: Quota) -> String
where
    P: RateLimitPolicy,
{
    let fingerprint = quota.fingerprint();
    let burst = fingerprint
        .burst
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string());

    format!(
        "{namespace}:{}:{}:{}:{}:{subject}",
        P::STATE_ID,
        fingerprint.max,
        fingerprint.per_ms,
        burst,
    )
}

/// Applies a rate limit policy to optional encoded state and returns updated bytes and snapshot.
///
/// Custom backends can use this helper inside [`RateLimitBackend::transact`].
pub fn apply_policy<P>(
    bytes: Option<&[u8]>,
    quota: Quota,
    now_ms: u64,
) -> Result<(Vec<u8>, RateLimitSnapshot), CodecError>
where
    P: RateLimitPolicy,
{
    let mut state = match bytes {
        Some(payload) => P::State::decode(payload, quota)?,
        None => P::State::create(quota, now_ms),
    };
    let snapshot = state.try_acquire(now_ms);
    Ok((state.encode()?, snapshot))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{FixedWindowPolicy, TokenBucketPolicy};
    use crate::quota::Quota;

    #[test]
    fn build_storage_key_includes_policy_quota_and_subject() {
        let quota = Quota::with_burst(5, 1000, 10);
        let key = build_storage_key::<TokenBucketPolicy>("ns", "user-1", quota);
        assert_eq!(key, "ns:token_bucket:5:1000:10:user-1");
    }

    #[test]
    fn different_quotas_produce_different_storage_keys() {
        let subject = "shared";
        let key_a = build_storage_key::<FixedWindowPolicy>("ns", subject, Quota::new(1, 1000));
        let key_b = build_storage_key::<FixedWindowPolicy>("ns", subject, Quota::new(5, 1000));
        assert_ne!(key_a, key_b);
    }
}
