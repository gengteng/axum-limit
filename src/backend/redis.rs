use super::{apply_policy, BackendError, RateLimitBackend};
use crate::policy::RateLimitPolicy;
use crate::quota::Quota;
use crate::snapshot::RateLimitSnapshot;
use async_trait::async_trait;
use redis::{AsyncCommands, ServerErrorKind};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

const MAX_RETRIES: usize = 8;

fn ttl_seconds_for_quota(quota: Quota) -> i64 {
    let seconds = quota.per_ms.div_ceil(1000).saturating_mul(2).max(60);
    i64::try_from(seconds).unwrap_or(i64::MAX)
}

/// Redis-backed storage for multi-node deployments.
#[derive(Clone)]
pub struct RedisBackend {
    namespace: Arc<str>,
    client: redis::Client,
    connection: Arc<Mutex<redis::aio::MultiplexedConnection>>,
}

impl RedisBackend {
    /// Connects to Redis using the provided URL.
    pub async fn connect(url: impl AsRef<str>) -> Result<Self, BackendError> {
        Self::connect_with_namespace(url, "axum-limit").await
    }

    /// Connects to Redis and sets a custom namespace.
    pub async fn connect_with_namespace(
        url: impl AsRef<str>,
        namespace: impl Into<Arc<str>>,
    ) -> Result<Self, BackendError> {
        let client = redis::Client::open(url.as_ref()).map_err(BackendError::Redis)?;
        let connection = client
            .get_multiplexed_async_connection()
            .await
            .map_err(BackendError::Redis)?;

        Ok(Self {
            namespace: namespace.into(),
            client,
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    /// Returns the underlying Redis client.
    pub fn client(&self) -> &redis::Client {
        &self.client
    }
}

#[async_trait]
impl RateLimitBackend for RedisBackend {
    type Error = BackendError;

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
        let ttl = ttl_seconds_for_quota(quota);

        for _ in 0..MAX_RETRIES {
            let mut connection = self.connection.lock().await;

            redis::cmd("WATCH")
                .arg(storage_key)
                .query_async::<()>(&mut *connection)
                .await
                .map_err(BackendError::Redis)?;

            let payload: Option<Vec<u8>> = connection
                .get(storage_key)
                .await
                .map_err(BackendError::Redis)?;

            let (encoded, snapshot) = apply_policy::<P>(payload.as_deref(), quota, now_ms)?;

            let mut pipe = redis::pipe();
            pipe.atomic()
                .set(storage_key, encoded)
                .ignore()
                .expire(storage_key, ttl)
                .ignore();

            match pipe.query_async::<Option<()>>(&mut *connection).await {
                Ok(Some(())) => return Ok(snapshot),
                Ok(None) => continue,
                Err(error) => {
                    if error.kind() == redis::ErrorKind::Server(ServerErrorKind::ExecAbort) {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                        continue;
                    }
                    return Err(BackendError::Redis(error));
                }
            }
        }

        Err(BackendError::Contention)
    }
}
