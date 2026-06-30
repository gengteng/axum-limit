use axum::extract::Path;
use axum::routing::get;
use axum::Router;
use axum_limit::{
    FixedWindowPerSecond, FixedWindowPolicy, Key, Limit, LimitPerDay, LimitPerHour,
    LimitPerSecond, LimitState, SlidingWindowPerSecond, SlidingWindowPolicy,
};
use http::{Method, Uri};
use serde::Deserialize;
use std::hash::Hash;
use std::net::{Ipv4Addr, SocketAddr};
use tokio::net::TcpListener;

/// Demonstrates token bucket, fixed window, and sliding window rate limiting.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 8080))).await?;

    let app = Router::new()
        .route(
            "/limit-2-per-500-ms-by-method",
            get(limit_2_per_500_ms_by_method),
        )
        .with_state(LimitState::<Method>::default())
        .route("/limit-4-per-sec-by-uri", get(limit_4_per_sec_by_uri))
        .with_state(LimitState::<Uri>::default())
        .route(
            "/fixed-window-3-per-sec-by-uri",
            get(fixed_window_3_per_sec_by_uri),
        )
        .with_state(LimitState::<Uri, FixedWindowPolicy>::default())
        .route(
            "/sliding-window-5-per-sec-by-uri",
            get(sliding_window_5_per_sec_by_uri),
        )
        .with_state(LimitState::<Uri, SlidingWindowPolicy>::default())
        .route(
            "/limit-100-per-hour-by-uri-id/:id/:name",
            get(limit_100_per_hour_by_id),
        )
        .route(
            "/limit-10000-per-day-by-uri-id/:id/:name",
            get(limit_10000_per_day_by_id),
        )
        .with_state(LimitState::<(Uri, Id)>::default());

    axum::serve(listener, app).await?;
    Ok(())
}

async fn limit_2_per_500_ms_by_method(_: Limit<2, 500, Method>) {}
async fn limit_4_per_sec_by_uri(_: LimitPerSecond<4, Uri>) {}
async fn fixed_window_3_per_sec_by_uri(_: FixedWindowPerSecond<3, Uri>) {}
async fn sliding_window_5_per_sec_by_uri(_: SlidingWindowPerSecond<5, Uri>) {}
async fn limit_100_per_hour_by_id(
    Limit((uri, Path(Data { name, .. }))): LimitPerHour<100, (Uri, Id)>,
) {
    println!("{uri}, {name}");
}
async fn limit_10000_per_day_by_id(
    Limit((_uri, Path(Data { id, .. }))): LimitPerDay<10000, (Uri, Id)>,
) {
    println!("id: {id:?}");
}

#[derive(Debug, Deserialize)]
struct Data {
    id: Id,
    name: String,
}

#[derive(Deserialize, Clone, Copy, Hash, Ord, PartialOrd, Eq, PartialEq, Debug)]
struct Id(usize);

impl Key for Id {
    type Extractor = Path<Data>;

    fn from_extractor(extractor: &Self::Extractor) -> Self {
        extractor.id
    }
}
