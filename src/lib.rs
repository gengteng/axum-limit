#![doc = include_str!("../README.md")]
#![deny(unsafe_code, missing_docs, clippy::unwrap_used)]

mod backend;
mod codec;
mod key;
mod policy;
mod quota;
mod snapshot;
mod time;

use axum_core::extract::{FromRef, FromRequestParts};
use axum_core::response::{IntoResponse, Response};
use http::request::Parts;
use http::StatusCode;
use std::error::Error;
use std::fmt::Display;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use time::now_ms;

#[cfg(feature = "redis")]
pub use backend::RedisBackend;
pub use backend::{
    apply_policy, build_storage_key, BackendError, MemoryBackend, RateLimitBackend, StorageKey,
};
pub use codec::CodecError;
pub use policy::{
    FixedWindowPolicy, PolicyState, RateLimitPolicy, SlidingWindowPolicy, TokenBucketPolicy,
};
pub use quota::{Quota, QuotaFingerprint};
pub use snapshot::{RateLimitInfo, RateLimitSnapshot};

macro_rules! define_limit_extractor {
    (
        $(#[$struct_meta:meta])*
        $name:ident => $policy:ty
    ) => {
        $(#[$struct_meta])*
        #[derive(Debug, Clone, Copy, Default)]
        pub struct $name<const COUNT: usize, const PER: u64, K, B = MemoryBackend>(
            pub K::Extractor,
            pub std::marker::PhantomData<B>,
        )
        where
            K: Key,
            B: RateLimitBackend;

        impl<const COUNT: usize, const PER: u64, K, B> AsRef<K::Extractor> for $name<COUNT, PER, K, B>
        where
            K: Key,
            B: RateLimitBackend,
        {
            fn as_ref(&self) -> &K::Extractor {
                &self.0
            }
        }

        impl<const COUNT: usize, const PER: u64, K, B> AsMut<K::Extractor> for $name<COUNT, PER, K, B>
        where
            K: Key,
            B: RateLimitBackend,
        {
            fn as_mut(&mut self) -> &mut K::Extractor {
                &mut self.0
            }
        }

        impl<const COUNT: usize, const PER: u64, K, B> Deref for $name<COUNT, PER, K, B>
        where
            K: Key,
            B: RateLimitBackend,
        {
            type Target = K::Extractor;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl<const COUNT: usize, const PER: u64, K, B> DerefMut for $name<COUNT, PER, K, B>
        where
            K: Key,
            B: RateLimitBackend,
        {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }

        impl<const COUNT: usize, const PER: u64, K, B> Display for $name<COUNT, PER, K, B>
        where
            K: Key,
            B: RateLimitBackend,
            K::Extractor: Display,
        {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }

        impl<const COUNT: usize, const PER: u64, K, B> $name<COUNT, PER, K, B>
        where
            K: Key,
            B: RateLimitBackend,
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
        impl<const C: usize, const P: u64, K, B, S> FromRequestParts<S> for $name<C, P, K, B>
        where
            B: RateLimitBackend,
            B::Error: Display,
            S: Send + Sync,
            K: Key + StorageKey,
            K::Extractor: FromRequestParts<S> + Send,
            LimitState<K, $policy, B>: FromRef<S>,
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

                let limit_state: LimitState<K, $policy, B> = FromRef::from_ref(state);
                let key = K::from_extractor(&key_extractor);
                let snapshot = limit_state
                    .check(key, Quota::new(C, P))
                    .await
                    .map_err(|error| LimitRejection::Backend(error.to_string()))?;

                if snapshot.allowed {
                    parts.extensions.insert(RateLimitInfo(snapshot));
                    Ok(Self(key_extractor, std::marker::PhantomData))
                } else {
                    Err(LimitRejection::RateLimitExceeded(snapshot))
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

define_limit_extractor! {
    /// Sliding-window rate limit extractor.
    SlidingWindowLimit => SlidingWindowPolicy
}

/// Token-bucket rate limit configured to apply per second.
pub type LimitPerSecond<const COUNT: usize, K, B = MemoryBackend> = Limit<COUNT, 1000, K, B>;

/// Token-bucket rate limit configured to apply per minute.
pub type LimitPerMinute<const COUNT: usize, K, B = MemoryBackend> = Limit<COUNT, 60_000, K, B>;

/// Token-bucket rate limit configured to apply per hour.
pub type LimitPerHour<const COUNT: usize, K, B = MemoryBackend> = Limit<COUNT, 3_600_000, K, B>;

/// Token-bucket rate limit configured to apply per day.
pub type LimitPerDay<const COUNT: usize, K, B = MemoryBackend> = Limit<COUNT, 86_400_000, K, B>;

/// Fixed-window rate limit configured to apply per second.
pub type FixedWindowPerSecond<const COUNT: usize, K, B = MemoryBackend> =
    FixedWindowLimit<COUNT, 1000, K, B>;

/// Fixed-window rate limit configured to apply per minute.
pub type FixedWindowPerMinute<const COUNT: usize, K, B = MemoryBackend> =
    FixedWindowLimit<COUNT, 60_000, K, B>;

/// Fixed-window rate limit configured to apply per hour.
pub type FixedWindowPerHour<const COUNT: usize, K, B = MemoryBackend> =
    FixedWindowLimit<COUNT, 3_600_000, K, B>;

/// Fixed-window rate limit configured to apply per day.
pub type FixedWindowPerDay<const COUNT: usize, K, B = MemoryBackend> =
    FixedWindowLimit<COUNT, 86_400_000, K, B>;

/// Sliding-window rate limit configured to apply per second.
pub type SlidingWindowPerSecond<const COUNT: usize, K, B = MemoryBackend> =
    SlidingWindowLimit<COUNT, 1000, K, B>;

/// Sliding-window rate limit configured to apply per minute.
pub type SlidingWindowPerMinute<const COUNT: usize, K, B = MemoryBackend> =
    SlidingWindowLimit<COUNT, 60_000, K, B>;

/// Sliding-window rate limit configured to apply per hour.
pub type SlidingWindowPerHour<const COUNT: usize, K, B = MemoryBackend> =
    SlidingWindowLimit<COUNT, 3_600_000, K, B>;

/// Sliding-window rate limit configured to apply per day.
pub type SlidingWindowPerDay<const COUNT: usize, K, B = MemoryBackend> =
    SlidingWindowLimit<COUNT, 86_400_000, K, B>;

/// Trait defining the requirements for a key extractor, which is used to uniquely identify limit subjects
/// and extract rate limit parameters dynamically in request processing.
#[async_trait::async_trait]
pub trait Key: Send + Sync {
    /// Extractor used to build a rate limit key from request parts.
    type Extractor;
    /// Creates an instance of `Self` from the provided extractor reference.
    fn from_extractor(extractor: &Self::Extractor) -> Self;
}

/// Manages per-key state for a specific [`RateLimitPolicy`] and [`RateLimitBackend`].
#[derive(Clone)]
pub struct LimitState<K, P = TokenBucketPolicy, B = MemoryBackend> {
    backend: B,
    _marker: PhantomData<(K, P)>,
}

impl<K, P, B> LimitState<K, P, B>
where
    K: Key,
    P: RateLimitPolicy,
    B: RateLimitBackend,
{
    /// Creates a limit state backed by the provided storage backend.
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            _marker: PhantomData,
        }
    }

    /// Returns the configured storage backend.
    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// Checks and updates the rate limit for the given key.
    pub async fn check(&self, key: K, quota: Quota) -> Result<RateLimitSnapshot, B::Error>
    where
        K: StorageKey,
    {
        let storage_key =
            build_storage_key::<P>(self.backend.namespace(), &key.storage_key(), quota);
        self.backend
            .transact::<P>(&storage_key, quota, now_ms())
            .await
    }
}

impl<K, P> Default for LimitState<K, P, MemoryBackend>
where
    K: Key,
    P: RateLimitPolicy,
{
    fn default() -> Self {
        Self::new(MemoryBackend::default())
    }
}

/// Enumerates possible failure modes for rate limiting when extracting from request parts.
#[derive(Debug)]
pub enum LimitRejection<R> {
    /// Indicates a failure during key extraction, storing the underlying rejection reason.
    KeyExtractionFailure(R),

    /// Indicates that the rate limit has been exceeded.
    RateLimitExceeded(RateLimitSnapshot),

    /// Indicates that the storage backend failed while checking the limit.
    Backend(String),
}

impl<R: Display> Display for LimitRejection<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LimitRejection::KeyExtractionFailure(r) => write!(f, "{r}"),
            LimitRejection::RateLimitExceeded(_) => write!(f, "Rate limit exceeded."),
            LimitRejection::Backend(message) => write!(f, "Rate limit storage failure: {message}"),
        }
    }
}

impl<R: Error + 'static> Error for LimitRejection<R> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            LimitRejection::KeyExtractionFailure(ve) => Some(ve),
            LimitRejection::RateLimitExceeded(_) | LimitRejection::Backend(_) => None,
        }
    }
}

impl<R: IntoResponse> IntoResponse for LimitRejection<R> {
    fn into_response(self) -> Response {
        match self {
            LimitRejection::KeyExtractionFailure(rejection) => rejection.into_response(),
            LimitRejection::RateLimitExceeded(snapshot) => {
                let now_ms = now_ms();
                let mut response =
                    (StatusCode::TOO_MANY_REQUESTS, "Rate limit exceeded.").into_response();
                response.headers_mut().extend(snapshot.to_headers(now_ms));
                response
            }
            LimitRejection::Backend(message) => (
                StatusCode::SERVICE_UNAVAILABLE,
                format!("Rate limit storage failure: {message}"),
            )
                .into_response(),
        }
    }
}

/// Returns rate limit headers from request extensions when a limit check succeeded.
pub fn rate_limit_headers_from_parts(parts: &Parts) -> Option<http::HeaderMap> {
    parts
        .extensions
        .get::<RateLimitInfo>()
        .map(|info| info.0.to_headers(now_ms()))
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
        async fn handler0(Limit(_uri, _): Limit<1, 1, Uri>) -> impl IntoResponse {}

        async fn handler1(Limit(_uri, _): Limit<3, 1, Uri>) -> impl IntoResponse {}

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
    }

    #[tokio::test]
    async fn sliding_window_limit_per_second() {
        const ROUTE: &str = "/sliding_per_sec";
        async fn handler(_: SlidingWindowPerSecond<3, Uri>) -> impl IntoResponse {}

        let app = Router::new()
            .route(ROUTE, get(handler))
            .with_state(LimitState::<Uri, SlidingWindowPolicy>::default());

        let server = TestServer::new(app).expect("server");

        for _ in 0..3 {
            assert_eq!(server.get(ROUTE).await.status_code(), StatusCode::OK);
        }
        assert_eq!(
            server.get(ROUTE).await.status_code(),
            StatusCode::TOO_MANY_REQUESTS
        );

        tokio::time::sleep(Duration::from_secs(2)).await;

        for _ in 0..3 {
            assert_eq!(server.get(ROUTE).await.status_code(), StatusCode::OK);
        }
    }

    #[tokio::test]
    async fn rate_limit_exceeded_includes_headers() {
        const ROUTE: &str = "/headers";
        async fn handler(_: LimitPerSecond<1, Uri>) -> impl IntoResponse {}

        let app = Router::new()
            .route(ROUTE, get(handler))
            .with_state(LimitState::default());

        let server = TestServer::new(app).expect("server");

        assert_eq!(server.get(ROUTE).await.status_code(), StatusCode::OK);
        let response = server.get(ROUTE).await;
        assert_eq!(response.status_code(), StatusCode::TOO_MANY_REQUESTS);
        assert!(response.headers().get("x-ratelimit-limit").is_some());
        assert!(response.headers().get("retry-after").is_some());
    }

    #[tokio::test]
    async fn different_quotas_on_same_key_are_isolated() {
        let state = LimitState::<Uri>::default();
        let uri: Uri = "/test".parse().expect("valid uri");
        let quota_a = Quota::new(1, 1000);
        let quota_b = Quota::new(5, 1000);

        assert!(
            state
                .check(uri.clone(), quota_a)
                .await
                .expect("check")
                .allowed
        );
        assert!(
            !state
                .check(uri.clone(), quota_a)
                .await
                .expect("check")
                .allowed
        );
        assert!(state.check(uri, quota_b).await.expect("check").allowed);
    }

    #[tokio::test]
    async fn custom_memory_namespace_isolates_state() {
        let left = LimitState::<Uri>::new(MemoryBackend::with_namespace("left"));
        let right = LimitState::<Uri>::new(MemoryBackend::with_namespace("right"));
        let uri: Uri = "/shared".parse().expect("valid uri");
        let quota = Quota::per_second(1);

        assert!(left.check(uri.clone(), quota).await.expect("left").allowed);
        assert!(!left.check(uri.clone(), quota).await.expect("left").allowed);
        assert!(right.check(uri, quota).await.expect("right").allowed);
    }
}
