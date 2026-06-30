use crate::quota::Quota;
use axum_core::extract::FromRef;
use axum_core::response::IntoResponse;
use http::request::Parts;
use std::convert::Infallible;

/// Resolves a [`Quota`] at request time.
///
/// Types that implement [`FromRef`] and [`Into<Quota>`] receive a blanket implementation.
pub trait QuotaSource<S>: Send + Sync + Sized + 'static {
    /// Error returned when quota resolution fails.
    type Rejection: IntoResponse;

    /// Resolves the quota for the current request.
    fn resolve(
        parts: &Parts,
        state: &S,
    ) -> impl std::future::Future<Output = Result<Quota, Self::Rejection>> + Send;
}

impl<S, Q> QuotaSource<S> for Q
where
    Q: FromRef<S> + Into<Quota> + Send + Sync + 'static,
    S: Sync,
{
    type Rejection = Infallible;

    fn resolve(
        _parts: &Parts,
        state: &S,
    ) -> impl std::future::Future<Output = Result<Quota, Self::Rejection>> + Send {
        let quota = Q::from_ref(state).into();
        async move { Ok(quota) }
    }
}

/// Resolves a fixed [`Quota`] stored in application state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedQuota(pub Quota);

impl From<FixedQuota> for Quota {
    fn from(value: FixedQuota) -> Self {
        value.0
    }
}
