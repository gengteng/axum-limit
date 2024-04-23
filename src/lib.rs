mod key;

use axum_core::extract::{FromRef, FromRequestParts};
use axum_core::response::{IntoResponse, Response};
use dashmap::DashMap;
use http::request::Parts;
use http::StatusCode;
use std::hash::Hash;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct Limit<const COUNT: usize, const PER: u64, E>(pub E);

impl<const COUNT: usize, const PER: u64, E> AsRef<E> for Limit<COUNT, PER, E> {
    fn as_ref(&self) -> &E {
        &self.0
    }
}

impl<const COUNT: usize, const PER: u64, E> AsMut<E> for Limit<COUNT, PER, E> {
    fn as_mut(&mut self) -> &mut E {
        &mut self.0
    }
}

impl<const COUNT: usize, const PER: u64, E> Limit<COUNT, PER, E> {
    pub const fn count() -> usize {
        COUNT
    }

    pub const fn per() -> u64 {
        PER
    }

    pub fn check<S>(&self, state: &LimitState<E::Key>, key: &E::Key) -> bool
    where
        E: KeyExtractor<S> + Send + Sync,
        E::Key: Clone,
    {
        let mut bucket = state
            .rate_limits
            .entry(key.clone())
            .or_insert_with(|| TokenBucket::new(Self::count(), Self::per()));
        bucket.try_acquire()
    }

    pub fn into_inner(self) -> E {
        self.0
    }
}

#[async_trait::async_trait]
pub trait KeyExtractor<S>: FromRequestParts<S> {
    type Key: Eq + Hash + Send + Sync + Clone;
    fn get_key(&self) -> &Self::Key;
}

struct TokenBucket {
    tokens: usize,
    last_refill_time: Instant,
    refill_duration: Duration,
}

impl TokenBucket {
    fn new(tokens: impl Into<usize>, per: impl Into<u64>) -> Self {
        Self {
            tokens: tokens.into(),
            last_refill_time: Instant::now(),
            refill_duration: Duration::from_secs(per.into()),
        }
    }

    fn try_acquire(&mut self) -> bool {
        self.refill();
        if self.tokens > 0 {
            self.tokens -= 1;
            true
        } else {
            false
        }
    }

    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill_time);
        if elapsed >= self.refill_duration {
            let new_tokens = (elapsed.as_secs() / self.refill_duration.as_secs()) as usize;
            self.tokens += new_tokens;
            self.last_refill_time =
                now - Duration::from_secs(elapsed.as_secs() % self.refill_duration.as_secs());
        }
    }
}

#[derive(Clone, Default)]
pub struct LimitState<K>
where
    K: Eq + Hash + Send + Sync + Clone,
{
    rate_limits: Arc<DashMap<K, TokenBucket>>,
}

#[async_trait::async_trait]
impl<const C: usize, const P: u64, E: KeyExtractor<S>, S> FromRequestParts<S> for Limit<C, P, E>
where
    LimitState<E::Key>: FromRef<S>,
    S: Send + Sync,
    E: KeyExtractor<S> + Send + Sync,
{
    type Rejection = LimitRejection<E::Rejection>;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let key_extractor = match E::from_request_parts(parts, state).await {
            Ok(ke) => ke,
            Err(rejection) => return Err(LimitRejection::KeyExtractionFailure(rejection)),
        };

        let limit_state: LimitState<E::Key> = FromRef::from_ref(state);
        let limit = Self(key_extractor);
        if limit.check(&limit_state, limit.as_ref().get_key()) {
            Ok(limit)
        } else {
            Err(LimitRejection::RateLimitExceeded)
        }
    }
}

pub enum LimitRejection<R> {
    KeyExtractionFailure(R),
    RateLimitExceeded,
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
}
