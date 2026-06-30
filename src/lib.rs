#![doc = include_str!("../README.md")]
#![deny(unsafe_code, missing_docs, clippy::unwrap_used)]

mod backend;
mod codec;
mod key;
mod policy;
mod quota;
mod quota_source;
mod snapshot;
mod time;

use axum_core::extract::{FromRef, FromRequestParts};
use axum_core::response::{IntoResponse, Response};
use http::request::Parts;
use http::StatusCode;
use std::convert::Infallible;
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
pub use quota_source::{FixedQuota, QuotaSource};
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

            fn from_request_parts(
                parts: &mut Parts,
                state: &S,
            ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
                async move {
                    let key_extractor = match K::Extractor::from_request_parts(parts, state).await {
                        Ok(ke) => ke,
                        Err(rejection) => {
                            return Err(LimitRejection::KeyExtractionFailure(rejection));
                        }
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

macro_rules! define_dynamic_limit_extractor {
    (
        $(#[$struct_meta:meta])*
        $name:ident => $policy:ty
    ) => {
        $(#[$struct_meta])*
        #[derive(Debug, Clone)]
        pub struct $name<K, Q, B = MemoryBackend>(
            pub K::Extractor,
            pub Quota,
            pub std::marker::PhantomData<(Q, B)>,
        )
        where
            K: Key,
            B: RateLimitBackend;

        impl<K, Q, B> AsRef<K::Extractor> for $name<K, Q, B>
        where
            K: Key,
            B: RateLimitBackend,
        {
            fn as_ref(&self) -> &K::Extractor {
                &self.0
            }
        }

        impl<K, Q, B> AsMut<K::Extractor> for $name<K, Q, B>
        where
            K: Key,
            B: RateLimitBackend,
        {
            fn as_mut(&mut self) -> &mut K::Extractor {
                &mut self.0
            }
        }

        impl<K, Q, B> Deref for $name<K, Q, B>
        where
            K: Key,
            B: RateLimitBackend,
        {
            type Target = K::Extractor;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl<K, Q, B> DerefMut for $name<K, Q, B>
        where
            K: Key,
            B: RateLimitBackend,
        {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }

        impl<K, Q, B> Display for $name<K, Q, B>
        where
            K: Key,
            B: RateLimitBackend,
            K::Extractor: Display,
        {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }

        impl<K, Q, B> $name<K, Q, B>
        where
            K: Key,
            B: RateLimitBackend,
        {
            /// Returns the quota resolved for this request.
            pub fn resolved_quota(&self) -> Quota {
                self.1
            }

            /// Consumes the limit and returns the inner extractor and resolved quota.
            pub fn into_parts(self) -> (K::Extractor, Quota) {
                (self.0, self.1)
            }

            /// Consumes the limit and returns the inner extractor.
            pub fn into_inner(self) -> K::Extractor {
                self.0
            }
        }

        impl<K, Q, B, S> FromRequestParts<S> for $name<K, Q, B>
        where
            B: RateLimitBackend,
            B::Error: Display,
            S: Send + Sync,
            K: Key + StorageKey,
            K::Extractor: FromRequestParts<S> + Send,
            Q: QuotaSource<S>,
            LimitState<K, $policy, B>: FromRef<S>,
        {
            type Rejection = LimitRejection<
                <<K as Key>::Extractor as FromRequestParts<S>>::Rejection,
                Q::Rejection,
            >;

            fn from_request_parts(
                parts: &mut Parts,
                state: &S,
            ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send
            where
                S: Sync,
            {
                async move {
                    let key_extractor = match K::Extractor::from_request_parts(parts, state).await {
                        Ok(ke) => ke,
                        Err(rejection) => {
                            return Err(LimitRejection::KeyExtractionFailure(rejection));
                        }
                    };

                    let quota = match Q::resolve(parts, state).await {
                        Ok(quota) => quota,
                        Err(rejection) => {
                            return Err(LimitRejection::QuotaResolutionFailure(rejection));
                        }
                    };

                    let limit_state: LimitState<K, $policy, B> = FromRef::from_ref(state);
                    let key = K::from_extractor(&key_extractor);
                    let snapshot = limit_state
                        .check(key, quota)
                        .await
                        .map_err(|error| LimitRejection::Backend(error.to_string()))?;

                    if snapshot.allowed {
                        parts.extensions.insert(RateLimitInfo(snapshot));
                        Ok(Self(
                            key_extractor,
                            quota,
                            std::marker::PhantomData,
                        ))
                    } else {
                        Err(LimitRejection::RateLimitExceeded(snapshot))
                    }
                }
            }
        }
    };
}

define_dynamic_limit_extractor! {
    /// Token-bucket rate limit extractor with a runtime [`QuotaSource`].
    DynamicLimit => TokenBucketPolicy
}

define_dynamic_limit_extractor! {
    /// Fixed-window rate limit extractor with a runtime [`QuotaSource`].
    DynamicFixedWindowLimit => FixedWindowPolicy
}

define_dynamic_limit_extractor! {
    /// Sliding-window rate limit extractor with a runtime [`QuotaSource`].
    DynamicSlidingWindowLimit => SlidingWindowPolicy
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
pub enum LimitRejection<K, Q = Infallible> {
    /// Indicates a failure during key extraction, storing the underlying rejection reason.
    KeyExtractionFailure(K),

    /// Indicates that quota resolution failed.
    QuotaResolutionFailure(Q),

    /// Indicates that the rate limit has been exceeded.
    RateLimitExceeded(RateLimitSnapshot),

    /// Indicates that the storage backend failed while checking the limit.
    Backend(String),
}

impl<K: Display, Q: Display> Display for LimitRejection<K, Q> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LimitRejection::KeyExtractionFailure(r) => write!(f, "{r}"),
            LimitRejection::QuotaResolutionFailure(r) => write!(f, "{r}"),
            LimitRejection::RateLimitExceeded(_) => write!(f, "Rate limit exceeded."),
            LimitRejection::Backend(message) => write!(f, "Rate limit storage failure: {message}"),
        }
    }
}

impl<K: Error + 'static, Q: Error + 'static> Error for LimitRejection<K, Q> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            LimitRejection::KeyExtractionFailure(error) => Some(error),
            LimitRejection::QuotaResolutionFailure(error) => Some(error),
            LimitRejection::RateLimitExceeded(_) | LimitRejection::Backend(_) => None,
        }
    }
}

impl<K: IntoResponse, Q: IntoResponse> IntoResponse for LimitRejection<K, Q> {
    fn into_response(self) -> Response {
        match self {
            LimitRejection::KeyExtractionFailure(rejection) => rejection.into_response(),
            LimitRejection::QuotaResolutionFailure(rejection) => rejection.into_response(),
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
        async fn handler0(Limit(_uri, _): LimitPerSecond<1, Uri>) -> impl IntoResponse {}

        async fn handler1(Limit(_uri, _): LimitPerSecond<3, Uri>) -> impl IntoResponse {}

        let my_app = Router::new()
            .route(TEST_ROUTE0, get(handler0))
            .route(TEST_ROUTE1, get(handler1))
            .with_state(LimitState::default());

        let server = TestServer::new(my_app);

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

        let server = TestServer::new(app);

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

        let server = TestServer::new(app);

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

        let server = TestServer::new(app);

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

        let server = TestServer::new(app);

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

    #[derive(Clone)]
    struct ConfigState {
        limits: LimitState<Uri>,
        api_quota: Quota,
    }

    impl FromRef<ConfigState> for LimitState<Uri> {
        fn from_ref(state: &ConfigState) -> Self {
            state.limits.clone()
        }
    }

    impl FromRef<ConfigState> for Quota {
        fn from_ref(state: &ConfigState) -> Quota {
            state.api_quota
        }
    }

    #[derive(Clone, Copy)]
    struct ApiQuota(Quota);

    impl FromRef<ConfigState> for ApiQuota {
        fn from_ref(state: &ConfigState) -> Self {
            ApiQuota(state.api_quota)
        }
    }

    impl From<ApiQuota> for Quota {
        fn from(value: ApiQuota) -> Self {
            value.0
        }
    }

    #[tokio::test]
    async fn dynamic_limit_reads_quota_from_state() {
        const ROUTE: &str = "/dynamic";
        async fn handler(_: DynamicLimit<Uri, Quota>) -> impl IntoResponse {}

        let state = ConfigState {
            limits: LimitState::default(),
            api_quota: Quota::per_second(2),
        };

        let app = Router::new().route(ROUTE, get(handler)).with_state(state);

        let server = TestServer::new(app);

        assert_eq!(server.get(ROUTE).await.status_code(), StatusCode::OK);
        assert_eq!(server.get(ROUTE).await.status_code(), StatusCode::OK);
        assert_eq!(
            server.get(ROUTE).await.status_code(),
            StatusCode::TOO_MANY_REQUESTS
        );
    }

    #[tokio::test]
    async fn dynamic_limit_state_quota_newtype() {
        const ROUTE: &str = "/dynamic_newtype";
        async fn handler(_: DynamicLimit<Uri, ApiQuota>) -> impl IntoResponse {}

        let state = ConfigState {
            limits: LimitState::default(),
            api_quota: Quota::per_second(1),
        };

        let app = Router::new().route(ROUTE, get(handler)).with_state(state);

        let server = TestServer::new(app);

        assert_eq!(server.get(ROUTE).await.status_code(), StatusCode::OK);
        assert_eq!(
            server.get(ROUTE).await.status_code(),
            StatusCode::TOO_MANY_REQUESTS
        );
    }

    #[tokio::test]
    async fn different_keys_are_isolated() {
        let state = LimitState::<Uri>::default();
        let route_a: Uri = "/a".parse().expect("valid uri");
        let route_b: Uri = "/b".parse().expect("valid uri");
        let quota = Quota::per_second(1);

        assert!(
            state
                .check(route_a.clone(), quota)
                .await
                .expect("a")
                .allowed
        );
        assert!(!state.check(route_a, quota).await.expect("a").allowed);
        assert!(state.check(route_b, quota).await.expect("b").allowed);
    }

    #[test]
    fn rate_limit_headers_from_parts_reads_extension() {
        let mut parts = http::Request::new(()).into_parts().0;
        parts.extensions.insert(RateLimitInfo(RateLimitSnapshot {
            allowed: true,
            limit: 5,
            remaining: 4,
            reset_at_ms: 1_000,
        }));

        let headers = rate_limit_headers_from_parts(&parts).expect("headers");
        assert_eq!(
            headers
                .get("x-ratelimit-limit")
                .and_then(|value| value.to_str().ok()),
            Some("5")
        );
        assert_eq!(
            headers
                .get("x-ratelimit-remaining")
                .and_then(|value| value.to_str().ok()),
            Some("4")
        );
    }

    #[tokio::test]
    async fn dynamic_fixed_window_limit_reads_quota_from_state() {
        #[derive(Clone)]
        struct FixedConfigState {
            limits: LimitState<Uri, FixedWindowPolicy>,
            api_quota: Quota,
        }

        impl FromRef<FixedConfigState> for LimitState<Uri, FixedWindowPolicy> {
            fn from_ref(state: &FixedConfigState) -> Self {
                state.limits.clone()
            }
        }

        impl FromRef<FixedConfigState> for Quota {
            fn from_ref(state: &FixedConfigState) -> Quota {
                state.api_quota
            }
        }

        const ROUTE: &str = "/dynamic_fixed";
        async fn handler(_: DynamicFixedWindowLimit<Uri, Quota>) -> impl IntoResponse {}

        let state = FixedConfigState {
            limits: LimitState::<Uri, FixedWindowPolicy>::default(),
            api_quota: Quota::per_second(2),
        };

        let app = Router::new().route(ROUTE, get(handler)).with_state(state);
        let server = TestServer::new(app);

        assert_eq!(server.get(ROUTE).await.status_code(), StatusCode::OK);
        assert_eq!(server.get(ROUTE).await.status_code(), StatusCode::OK);
        assert_eq!(
            server.get(ROUTE).await.status_code(),
            StatusCode::TOO_MANY_REQUESTS
        );
    }

    #[tokio::test]
    async fn dynamic_sliding_window_limit_reads_quota_from_state() {
        #[derive(Clone)]
        struct SlidingConfigState {
            limits: LimitState<Uri, SlidingWindowPolicy>,
            api_quota: Quota,
        }

        impl FromRef<SlidingConfigState> for LimitState<Uri, SlidingWindowPolicy> {
            fn from_ref(state: &SlidingConfigState) -> Self {
                state.limits.clone()
            }
        }

        impl FromRef<SlidingConfigState> for Quota {
            fn from_ref(state: &SlidingConfigState) -> Quota {
                state.api_quota
            }
        }

        const ROUTE: &str = "/dynamic_sliding";
        async fn handler(_: DynamicSlidingWindowLimit<Uri, Quota>) -> impl IntoResponse {}

        let state = SlidingConfigState {
            limits: LimitState::<Uri, SlidingWindowPolicy>::default(),
            api_quota: Quota::per_second(1),
        };

        let app = Router::new().route(ROUTE, get(handler)).with_state(state);
        let server = TestServer::new(app);

        assert_eq!(server.get(ROUTE).await.status_code(), StatusCode::OK);
        assert_eq!(
            server.get(ROUTE).await.status_code(),
            StatusCode::TOO_MANY_REQUESTS
        );
    }

    #[derive(Clone)]
    struct FailingBackend;

    #[async_trait::async_trait]
    impl RateLimitBackend for FailingBackend {
        type Error = BackendError;

        fn namespace(&self) -> &str {
            "failing"
        }

        async fn transact<P>(
            &self,
            _storage_key: &str,
            _quota: Quota,
            _now_ms: u64,
        ) -> Result<RateLimitSnapshot, Self::Error>
        where
            P: RateLimitPolicy,
        {
            Err(BackendError::Contention)
        }
    }

    #[tokio::test]
    async fn backend_failure_returns_service_unavailable() {
        const ROUTE: &str = "/backend_fail";
        async fn handler(_: Limit<1, 1000, Uri, FailingBackend>) -> impl IntoResponse {}

        let app = Router::new()
            .route(ROUTE, get(handler))
            .with_state(LimitState::<Uri, TokenBucketPolicy, FailingBackend>::new(
                FailingBackend,
            ));

        let server = TestServer::new(app);
        assert_eq!(
            server.get(ROUTE).await.status_code(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
}
