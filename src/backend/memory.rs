use super::{apply_policy, build_storage_key, RateLimitBackend};
use crate::policy::RateLimitPolicy;
use crate::quota::Quota;
use crate::snapshot::RateLimitSnapshot;
use async_trait::async_trait;
use dashmap::DashMap;
use std::sync::Arc;

/// In-memory storage backend suitable for single-node deployments.
#[derive(Clone)]
pub struct MemoryBackend {
    namespace: Arc<str>,
    states: Arc<DashMap<String, Vec<u8>>>,
}

impl MemoryBackend {
    /// Creates a backend with the default namespace.
    pub fn new() -> Self {
        Self::with_namespace("axum-limit")
    }

    /// Creates a backend with a custom namespace.
    pub fn with_namespace(namespace: impl Into<Arc<str>>) -> Self {
        Self {
            namespace: namespace.into(),
            states: Arc::new(DashMap::new()),
        }
    }
}

impl Default for MemoryBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl RateLimitBackend for MemoryBackend {
    type Error = crate::codec::CodecError;

    fn namespace(&self) -> &str {
        &self.namespace
    }

    async fn transact<P>(
        &self,
        storage_key: &str,
        quota: Quota,
        now_ms: u64,
    ) -> Result<RateLimitSnapshot, Self::Error>
    where
        P: RateLimitPolicy,
    {
        let entry = self.states.entry(storage_key.to_string());
        match entry {
            dashmap::mapref::entry::Entry::Occupied(mut occupied) => {
                let (encoded, snapshot) = apply_policy::<P>(Some(occupied.get()), quota, now_ms)?;
                *occupied.get_mut() = encoded;
                Ok(snapshot)
            }
            dashmap::mapref::entry::Entry::Vacant(vacant) => {
                let (encoded, snapshot) = apply_policy::<P>(None, quota, now_ms)?;
                vacant.insert(encoded);
                Ok(snapshot)
            }
        }
    }
}

impl MemoryBackend {
    /// Builds a storage key for the given subject and quota.
    pub fn storage_key<P>(namespace: &str, subject: &str, quota: Quota) -> String
    where
        P: RateLimitPolicy,
    {
        build_storage_key::<P>(namespace, subject, quota)
    }
}
