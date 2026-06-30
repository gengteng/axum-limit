#![doc = include_str!("../README.md")]
#![deny(unsafe_code, missing_docs, clippy::unwrap_used)]

mod key;
mod policy;
mod quota;

use axum_core::extract::{FromRef, FromRequestParts};
use axum_core::response::{IntoResponse, Response};
use dashmap::DashMap;
use http::request::Parts;
use http::StatusCode;
use policy::{RateLimitPolicy, RateLimitState};
use std::error::Error;
use std::fmt::Display;
use std::hash::Hash;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::time::Instant;

pub use policy::{FixedWindowPolicy, TokenBucketPolicy};
pub use quota::Quota;

macro_rules! define_limit_extractor {
    (
        $(#[$struct_meta:meta])*
        $name:ident => $policy:ty
    ) => {
        $(#[$struct_meta])*
        #[derive(Debug, Clone, Copy, Default)]
        pub struct $name<const COUNT: usize, const PER: u64, K>(pub K::Extractor)
        where
            K: Key;

        impl<const COUNT: usize, const PER: u64, K> AsRef<K::Extractor> for $name<COUNT, PER, K>
        where
            K: Key,
        {
            fn as_ref(&self) -> &K::Extractor {
                &self.0
            }
        }

        impl<const COUNT: usize, const PER: u64, K> AsMut<K::Extractor> for $name<COUNT, PER, K>
        where
            K: Key,
        {
            fn as_mut(&mut self) -> &mut K::Extractor {
                &mut self.0
            }
        }

        impl<const COUNT: usize, const PER: u64, K> Deref for $name<COUNT, PER, K>
        where
            K: Key,
        {
            type Target = K::Extractor;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl<const COUNT: usize, const PER: u64, K> DerefMut for $name<COUNT, PER, K>
        where
            K: Key,
        {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }

        impl<const COUNT: usize, const PER: u64, K> Display for $name<COUNT, PER, K>
        where
            K: Key,
            K::Extractor: Display,
        {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }

        impl<const COUNT: usize, const PER: u64, K> $name<COUNT, PER, K>
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

            /// Returns the quota configured by this extractor.
            pub const fn quota() -> Quota {
                Quota::new(COUNT, PER)
            }

            /// Consumes the limit and returns the inner extractor.
            pub fn into_inner(self) -> K::Extractor {
                self.0
            }
        }

        #[async_trait::async_trait]
        impl<const C: usize, const P: u64, K, S> FromRequestParts<S> for $name<C, P, K>
        where
            LimitState<K, $policy>: FromRef<S>,
            S: Send + Sync,
            K: Key,
            K::Extractor: FromRequestParts<S>,
        {
            type Rejection =
                LimitRejection<<<K as Key>::Extractor as FromRequestParts<S>>::Rejection>;

            async fn from_request_parts(
                parts: &mut Parts,
                state: &S,
            ) -> Result<Self, Self::Rejection> {
                let key_extractor = match K::Extractor::from_request_parts(parts, state).await {
                    Ok(ke) => ke,
                    Err(rejection) => return Err(LimitRejection::KeyExtractionFailure(rejection)),
                };

                let limit_state: LimitState<K, $policy> = FromRef::from_ref(state);
                let key = K::from_extractor(&key_extractor);
                if limit_state.check(key, Quota::new(C, P)) {
                    Ok(Self(key_extractor))
                } else {
                    Err(LimitRejection::RateLimitExceeded)
                }
            }
        }
    };
}

define_limit_extractor! {
    /// Token-bucket rate limit extractor.
    Limit => TokenBucketPolicy
}

define_limit_extractor! {
    /// Fixed-window rate limit extractor.
    FixedWindowLimit => FixedWindowPolicy
}

/// Token-bucket rate limit configured to apply per second.
pub type LimitPerSecond<const COUNT: usize, K> = Limit<COUNT, 1000, K>;

/// Token-bucket rate limit configured to apply per minute.
pub type LimitPerMinute<const COUNT: usize, K> = Limit<COUNT, 60_000, K>;

/// Token-bucket rate limit configured to apply per hour.
pub type LimitPerHour<const COUNT: usize, K> = Limit<COUNT, 3_600_000, K>;

/// Token-bucket rate limit configured to apply per day.
pub type LimitPerDay<const COUNT: usize, K> = Limit<COUNT, 86_400_000, K>;

/// Fixed-window rate limit configured to apply per second.
pub type FixedWindowPerSecond<const COUNT: usize, K> = FixedWindowLimit<COUNT, 1000, K>;

/// Fixed-window rate limit configured to apply per minute.
pub type FixedWindowPerMinute<const COUNT: usize, K> = FixedWindowLimit<COUNT, 60_000, K>;

/// Fixed-window rate limit configured to apply per hour.
pub type FixedWindowPerHour<const COUNT: usize, K> = FixedWindowLimit<COUNT, 3_600_000, K>;

/// Fixed-window rate limit configured to apply per day.
pub type FixedWindowPerDay<const COUNT: usize, K> = FixedWindowLimit<COUNT, 86_400_000, K>;

/// Trait defining the requirements for a key extractor, which is used to uniquely identify limit subjects
/// and extract rate limit parameters dynamically in request processing.
#[async_trait::async_trait]
pub trait Key: Eq + Hash + Send + Sync {
    /// Extractor used to build a rate limit key from request parts.
    type Extractor;
    /// Creates an instance of `Self` from the provided extractor reference.
    fn from_extractor(extractor: &Self::Extractor) -> Self;
}

/// Manages per-key state for a specific [`RateLimitPolicy`].
#[derive(Clone)]
pub struct LimitState<K, P = TokenBucketPolicy>
where
    K: Key,
    P: RateLimitPolicy,
{
    rate_limits: Arc<DashMap<K, P::State>>,
}

impl<K, P> Default for LimitState<K, P>
where
    K: Key,
    P: RateLimitPolicy,
{
    fn default() -> Self {
        Self {
            rate_limits: Arc::new(DashMap::new()),
        }
    }
}

impl<K, P> LimitState<K, P>
where
    K: Key,
    P: RateLimitPolicy,
{
    /// Checks and updates the rate limit for the given key, returning `true` if the request can proceed.
    pub fn check(&self, key: K, quota: Quota) -> bool {
        let mut state = self
            .rate_limits
            .entry(key)
            .or_insert_with(|| P::create_state(quota));
        state.try_acquire(Instant::now())
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
    use std::time::Duration;

    #[tokio::test]
    async fn token_bucket_limit() {
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
    async fn token_bucket_limit_per_100_millis() {
        const TEST_ROUTE: &str = "/limit_per_100_millis";

        async fn handler(Limit(_uri): Limit<1, 100, Uri>) -> impl IntoResponse {}

        let my_app = Router::new()
            .route(TEST_ROUTE, get(handler))
            .with_state(LimitState::default());

        let server = TestServer::new(my_app).expect("Failed to create test server");

        let response = server.get(TEST_ROUTE).await;
        assert_eq!(response.status_code(), StatusCode::OK);

        let response = server.get(TEST_ROUTE).await;
        assert_eq!(response.status_code(), StatusCode::TOO_MANY_REQUESTS);

        tokio::time::sleep(Duration::from_millis(100)).await;

        let response = server.get(TEST_ROUTE).await;
        assert_eq!(response.status_code(), StatusCode::OK);
    }

    #[tokio::test]
    async fn token_bucket_limit_per_second_allows_count_per_second() {
        const ROUTE: &str = "/per_sec";
        async fn handler(_: LimitPerSecond<5, Uri>) -> impl IntoResponse {}

        let app = Router::new()
            .route(ROUTE, get(handler))
            .with_state(LimitState::default());

        let server = TestServer::new(app).expect("server");

        for _ in 0..5 {
            assert_eq!(server.get(ROUTE).await.status_code(), StatusCode::OK);
        }
        assert_eq!(
            server.get(ROUTE).await.status_code(),
            StatusCode::TOO_MANY_REQUESTS
        );

        tokio::time::sleep(Duration::from_secs(1)).await;
        for _ in 0..5 {
            assert_eq!(server.get(ROUTE).await.status_code(), StatusCode::OK);
        }
        assert_eq!(
            server.get(ROUTE).await.status_code(),
            StatusCode::TOO_MANY_REQUESTS
        );
    }

    #[tokio::test]
    async fn fixed_window_limit_per_second() {
        const ROUTE: &str = "/fixed_per_sec";
        async fn handler(_: FixedWindowPerSecond<3, Uri>) -> impl IntoResponse {}

        let app = Router::new()
            .route(ROUTE, get(handler))
            .with_state(LimitState::<Uri, FixedWindowPolicy>::default());

        let server = TestServer::new(app).expect("server");

        for _ in 0..3 {
            assert_eq!(server.get(ROUTE).await.status_code(), StatusCode::OK);
        }
        assert_eq!(
            server.get(ROUTE).await.status_code(),
            StatusCode::TOO_MANY_REQUESTS
        );

        tokio::time::sleep(Duration::from_secs(1)).await;

        for _ in 0..3 {
            assert_eq!(server.get(ROUTE).await.status_code(), StatusCode::OK);
        }
        assert_eq!(
            server.get(ROUTE).await.status_code(),
            StatusCode::TOO_MANY_REQUESTS
        );
    }
}
