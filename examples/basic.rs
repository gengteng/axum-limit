//! Demonstrates token bucket, fixed window, and sliding window rate limiting.
//!
//! Run with: `cargo run --example basic`

use axum::extract::Path;
use axum::routing::get;
use axum::Router;
use axum_core::extract::FromRef;
use axum_limit::{
    FixedWindowPerSecond, FixedWindowPolicy, Key, Limit, LimitPerDay, LimitPerHour, LimitPerSecond,
    LimitState, SlidingWindowPerSecond, SlidingWindowPolicy, StorageKey,
};
use http::{Method, Uri};
use serde::Deserialize;
use std::hash::Hash;
use std::net::{Ipv4Addr, SocketAddr};
use tokio::net::TcpListener;

#[derive(Clone)]
struct AppState {
    by_method: LimitState<Method>,
    by_uri: LimitState<Uri>,
    by_uri_fixed: LimitState<Uri, FixedWindowPolicy>,
    by_uri_sliding: LimitState<Uri, SlidingWindowPolicy>,
    by_uri_id: LimitState<(Uri, Id)>,
}

impl FromRef<AppState> for LimitState<Method> {
    fn from_ref(state: &AppState) -> Self {
        state.by_method.clone()
    }
}

impl FromRef<AppState> for LimitState<Uri> {
    fn from_ref(state: &AppState) -> Self {
        state.by_uri.clone()
    }
}

impl FromRef<AppState> for LimitState<Uri, FixedWindowPolicy> {
    fn from_ref(state: &AppState) -> Self {
        state.by_uri_fixed.clone()
    }
}

impl FromRef<AppState> for LimitState<Uri, SlidingWindowPolicy> {
    fn from_ref(state: &AppState) -> Self {
        state.by_uri_sliding.clone()
    }
}

impl FromRef<AppState> for LimitState<(Uri, Id)> {
    fn from_ref(state: &AppState) -> Self {
        state.by_uri_id.clone()
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 8080))).await?;

    let state = AppState {
        by_method: LimitState::default(),
        by_uri: LimitState::default(),
        by_uri_fixed: LimitState::default(),
        by_uri_sliding: LimitState::default(),
        by_uri_id: LimitState::default(),
    };

    let app = Router::new()
        .route(
            "/limit-2-per-500-ms-by-method",
            get(limit_2_per_500_ms_by_method),
        )
        .route("/limit-4-per-sec-by-uri", get(limit_4_per_sec_by_uri))
        .route(
            "/fixed-window-3-per-sec-by-uri",
            get(fixed_window_3_per_sec_by_uri),
        )
        .route(
            "/sliding-window-5-per-sec-by-uri",
            get(sliding_window_5_per_sec_by_uri),
        )
        .route(
            "/limit-100-per-hour-by-uri-id/:id/:name",
            get(limit_100_per_hour_by_id),
        )
        .route(
            "/limit-10000-per-day-by-uri-id/:id/:name",
            get(limit_10000_per_day_by_id),
        )
        .with_state(state);

    axum::serve(listener, app).await?;
    Ok(())
}

async fn limit_2_per_500_ms_by_method(_: Limit<2, 500, Method>) {}
async fn limit_4_per_sec_by_uri(_: LimitPerSecond<4, Uri>) {}
async fn fixed_window_3_per_sec_by_uri(_: FixedWindowPerSecond<3, Uri>) {}
async fn sliding_window_5_per_sec_by_uri(_: SlidingWindowPerSecond<5, Uri>) {}
async fn limit_100_per_hour_by_id(_: LimitPerHour<100, (Uri, Id)>) {}
async fn limit_10000_per_day_by_id(_: LimitPerDay<10000, (Uri, Id)>) {}

#[derive(Debug, Deserialize)]
struct Data {
    id: Id,
    _name: String,
}

#[derive(Deserialize, Clone, Copy, Hash, Ord, PartialOrd, Eq, PartialEq, Debug)]
struct Id(usize);

impl Key for Id {
    type Extractor = Path<Data>;

    fn from_extractor(extractor: &Self::Extractor) -> Self {
        extractor.id
    }
}

impl StorageKey for Id {
    fn storage_key(&self) -> String {
        self.0.to_string()
    }
}
