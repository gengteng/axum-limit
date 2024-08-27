#![doc = include_str!("../README.md")]
#![deny(unsafe_code, missing_docs, clippy::unwrap_used)]

mod key;

use axum_core::extract::{FromRef, FromRequestParts};
use axum_core::response::{IntoResponse, Response};
use dashmap::DashMap;
use http::request::Parts;
use http::StatusCode;
use std::error::Error;
use std::fmt::Display;
use std::hash::Hash;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Represents a rate limit configuration with generic parameters for count and time period.
/// This struct uses generics to allow flexible integration with any extractor that implements the `Key` trait.
#[derive(Debug, Clone, Copy, Default)]
pub struct Limit<const COUNT: usize, const PER: u64, K>(pub K::Extractor)
where
    K: Key;

/// Rate limit configured to apply per second.
pub type LimitPerSecond<const COUNT: usize, K> = Limit<COUNT, 1000, K>;

/// Rate limit configured to apply per minute.
pub type LimitPerMinute<const COUNT: usize, K> = Limit<COUNT, 60_000, K>;

/// Rate limit configured to apply per hour.
pub type LimitPerHour<const COUNT: usize, K> = Limit<COUNT, 3_600_000, K>;

/// Rate limit configured to apply per day.
pub type LimitPerDay<const COUNT: usize, K> = Limit<COUNT, 86_400_000, K>;

impl<const COUNT: usize, const PER: u64, K> AsRef<K::Extractor> for Limit<COUNT, PER, K>
where
    K: Key,
{
    fn as_ref(&self) -> &K::Extractor {
        &self.0
    }
}

impl<const COUNT: usize, const PER: u64, K> AsMut<K::Extractor> for Limit<COUNT, PER, K>
where
    K: Key,
{
    fn as_mut(&mut self) -> &mut K::Extractor {
        &mut self.0
    }
}

impl<const COUNT: usize, const PER: u64, K> Deref for Limit<COUNT, PER, K>
where
    K: Key,
{
    type Target = K::Extractor;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<const COUNT: usize, const PER: u64, K> DerefMut for Limit<COUNT, PER, K>
where
    K: Key,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<const COUNT: usize, const PER: u64, K> Display for Limit<COUNT, PER, K>
where
    K: Key,
    K::Extractor: Display,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl<const COUNT: usize, const PER: u64, K> Limit<COUNT, PER, K>
where
    K: Key,
{
    /// Returns the count of requests allowed within the specified period.
    pub const fn count() -> usize {
        COUNT
    }

    /// Returns the period (in milliseconds) for which the limit applies.
    pub const fn per() -> u64 {
        PER
    }

    /// Consumes the limit and returns the inner extractor, allowing direct access to the underlying mechanism.
    pub fn into_inner(self) -> K::Extractor {
        self.0
    }
}

/// Trait defining the requirements for a key extractor, which is used to uniquely identify limit subjects
/// and extract rate limit parameters dynamically in request processing.
#[async_trait::async_trait]
pub trait Key: Eq + Hash + Send + Sync {
    /// The `Extractor` associated type represents a component capable of extracting key-specific information from request parts.
    /// This information is then used to manage and enforce rate limits dynamically within the application.
    type Extractor;
    /// Creates an instance of `Self` from the provided extractor reference, allowing extraction of key data.
    fn from_extractor(extractor: &Self::Extractor) -> Self;
}

/// Implements a token bucket for rate limiting.
/// This struct manages the tokens for rate limiting, providing methods to acquire and refill tokens based on time elapsed.
struct TokenBucket {
    tokens: usize,
    last_refill_time: Instant,
    refill_duration: Duration,
}

impl TokenBucket {
    /// Constructs a new `TokenBucket` with a specific number of tokens and a refill period.
    fn new(tokens: impl Into<usize>, per: impl Into<u64>) -> Self {
        Self {
            tokens: tokens.into(),
            last_refill_time: Instant::now(),
            refill_duration: Duration::from_millis(per.into()),
        }
    }

    /// Attempts to acquire a token. Returns `true` if a token was successfully acquired.
    fn try_acquire(&mut self) -> bool {
        self.refill();
        if self.tokens > 0 {
            self.tokens -= 1;
            true
        } else {
            false
        }
    }

    /// Refills tokens based on time elapsed since the last refill.
    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill_time);

        // Calculate the elapsed time in milliseconds
        if elapsed >= self.refill_duration {
            let elapsed_millis = elapsed.as_millis() as u64; // Convert elapsed time to milliseconds
            let refill_duration_millis = self.refill_duration.as_millis() as u64; // Convert refill duration to milliseconds

            // Calculate the number of new tokens to add
            let new_tokens = (elapsed_millis / refill_duration_millis) as usize;
            self.tokens += new_tokens;

            // Reset the last refill time to avoid under-refilling tokens
            self.last_refill_time =
                now - Duration::from_millis(elapsed_millis % refill_duration_millis);
        }
    }
}

/// Manages the state of rate limits for various keys.
/// This struct holds a concurrent map of keys to their corresponding `TokenBucket` instances,
/// enabling efficient state management across asynchronous tasks.
#[derive(Clone)]
pub struct LimitState<K>
where
    K: Key,
{
    rate_limits: Arc<DashMap<K, TokenBucket>>,
}

impl<K> Default for LimitState<K>
where
    K: Key,
{
    /// Constructs a new `LimitState` with an empty map of rate limits.
    fn default() -> Self {
        Self {
            rate_limits: Arc::new(DashMap::new()),
        }
    }
}

impl<K> LimitState<K>
where
    K: Key,
{
    /// Checks and updates the rate limit for the given key, returning `true` if the request can proceed.
    pub fn check(&self, key: K, count: usize, per: u64) -> bool {
        let mut bucket = self
            .rate_limits
            .entry(key)
            .or_insert_with(|| TokenBucket::new(count, per));
        bucket.try_acquire()
    }
}

#[async_trait::async_trait]
impl<const C: usize, const P: u64, K, S> FromRequestParts<S> for Limit<C, P, K>
where
    LimitState<K>: FromRef<S>,
    S: Send + Sync,
    K: Key,
    K::Extractor: FromRequestParts<S>,
{
    type Rejection = LimitRejection<<<K as Key>::Extractor as FromRequestParts<S>>::Rejection>;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let key_extractor = match K::Extractor::from_request_parts(parts, state).await {
            Ok(ke) => ke,
            Err(rejection) => return Err(LimitRejection::KeyExtractionFailure(rejection)),
        };

        let limit_state: LimitState<K> = FromRef::from_ref(state);
        let key = K::from_extractor(&key_extractor);
        if limit_state.check(key, C, P) {
            Ok(Self(key_extractor))
        } else {
            Err(LimitRejection::RateLimitExceeded)
        }
    }
}

/// Enumerates possible failure modes for rate limiting when extracting from request parts.
#[derive(Debug)]
pub enum LimitRejection<R> {
    /// Indicates a failure during key extraction, storing the underlying rejection reason.
    KeyExtractionFailure(R),

    /// Indicates that the rate limit has been exceeded.
    RateLimitExceeded,
}

impl<R: Display> Display for LimitRejection<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LimitRejection::KeyExtractionFailure(r) => write!(f, "{r}"),
            LimitRejection::RateLimitExceeded => write!(f, "Rate limit exceeded."),
        }
    }
}

impl<R: Error + 'static> Error for LimitRejection<R> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            LimitRejection::KeyExtractionFailure(ve) => Some(ve),
            LimitRejection::RateLimitExceeded => None,
        }
    }
}

impl<R: IntoResponse> IntoResponse for LimitRejection<R> {
    fn into_response(self) -> Response {
        match self {
            LimitRejection::KeyExtractionFailure(rejection) => rejection.into_response(),
            LimitRejection::RateLimitExceeded => {
                (StatusCode::TOO_MANY_REQUESTS, "Rate limit exceeded.").into_response()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::get;
    use axum::Router;
    use axum_test::TestServer;
    use http::Uri;
    use std::future::IntoFuture;

    #[tokio::test]
    async fn limit() {
        const TEST_ROUTE0: &str = "/limit0";
        const TEST_ROUTE1: &str = "/limit1";
        async fn handler0(Limit(_uri): Limit<1, 1, Uri>) -> impl IntoResponse {}

        async fn handler1(Limit(_uri): Limit<3, 1, Uri>) -> impl IntoResponse {}

        let my_app = Router::new()
            .route(TEST_ROUTE0, get(handler0))
            .route(TEST_ROUTE1, get(handler1))
            .with_state(LimitState::default());

        let server = TestServer::new(my_app).expect("Failed to create test server");

        let response = server.get(TEST_ROUTE0).await;
        assert_eq!(response.status_code(), StatusCode::OK);
        let response = server.get(TEST_ROUTE0).await;
        assert_eq!(response.status_code(), StatusCode::TOO_MANY_REQUESTS);
        tokio::time::sleep(Duration::from_secs(1)).await;
        let response = server.get(TEST_ROUTE0).await;
        assert_eq!(response.status_code(), StatusCode::OK);

        let gets = vec![
            server.get(TEST_ROUTE1).into_future(),
            server.get(TEST_ROUTE1).into_future(),
            server.get(TEST_ROUTE1).into_future(),
        ];

        let resp = futures::future::join_all(gets).await;
        assert!(!resp.iter().any(|r| !r.status_code().is_success()));
        assert_eq!(
            server.get(TEST_ROUTE1).await.status_code(),
            StatusCode::TOO_MANY_REQUESTS
        );
        tokio::time::sleep(Duration::from_secs(1)).await;
        let response = server.get(TEST_ROUTE1).await;
        assert_eq!(response.status_code(), StatusCode::OK);
    }

    #[tokio::test]
    async fn limit_per_100_millis() {
        const TEST_ROUTE: &str = "/limit_per_100_millis";

        async fn handler(Limit(_uri): Limit<1, 100, Uri>) -> impl IntoResponse {}

        let my_app = Router::new()
            .route(TEST_ROUTE, get(handler))
            .with_state(LimitState::default());

        let server = TestServer::new(my_app).expect("Failed to create test server");

        // 第一次请求应该成功
        let response = server.get(TEST_ROUTE).await;
        assert_eq!(response.status_code(), StatusCode::OK);

        // 马上再发起一次请求应该被限制
        let response = server.get(TEST_ROUTE).await;
        assert_eq!(response.status_code(), StatusCode::TOO_MANY_REQUESTS);

        // 等待 100 毫秒
        tokio::time::sleep(Duration::from_millis(100)).await;

        // 再次请求应该成功
        let response = server.get(TEST_ROUTE).await;
        assert_eq!(response.status_code(), StatusCode::OK);
    }
}
