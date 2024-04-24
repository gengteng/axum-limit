use axum::extract::Path;
use axum::routing::get;
use axum::Router;
use axum_limit::{Key, Limit, LimitPerDay, LimitPerHour, LimitPerSecond, LimitState};
use http::{Method, Uri};
use serde::Deserialize;
use std::hash::Hash;
use std::net::{Ipv4Addr, SocketAddr};
use tokio::net::TcpListener;

/// Main entry point for an Axum application that demonstrates various rate limiting strategies.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Set up the TCP listener on port 8080 of all available network interfaces.
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 8080))).await?;

    // Build the router with specific routes and their associated rate limits.
    let app = Router::new()
        // Route to limit requests based on the HTTP method. The rate limit allows 2 requests per 500 milliseconds.
        .route(
            "/limit-2-per-500-ms-by-method",
            get(limit_2_per_500_ms_by_method),
        )
        .with_state(LimitState::<Method>::default()) // Initialize state for rate limiting based on HTTP method.
        // Route to limit requests based on the request URI. The rate limit allows 4 requests per second.
        .route("/limit-4-per-sec-by-uri", get(limit_4_per_sec_by_uri))
        .with_state(LimitState::<Uri>::default()) // Initialize state for rate limiting based on URI.
        // Route to limit requests based on a custom ID extracted from the request. The rate limit allows 100 requests per hour.
        .route(
            "/limit-100-per-hour-by-uri-id",
            get(limit_100_per_hour_by_id),
        )
        // Another route using the same ID-based rate limiting logic, but allowing 10,000 requests per day.
        .route(
            "/limit-10000-per-day-by-uri-id",
            get(limit_10000_per_day_by_id),
        )
        .with_state(LimitState::<(Uri, Id)>::default()); // Initialize state for rate limiting based on custom ID, shared by two routes.
    axum::serve(listener, app).await?;
    Ok(())
}

async fn limit_2_per_500_ms_by_method(_: Limit<2, 500, Method>) {}
async fn limit_4_per_sec_by_uri(_: LimitPerSecond<4, Uri>) {}
async fn limit_100_per_hour_by_id(
    Limit((uri, Path(Data { name, .. }))): LimitPerHour<100, (Uri, Id)>,
) {
    println!("{uri}, {name}");
}
async fn limit_10000_per_day_by_id(_: LimitPerDay<10000, (Uri, Id)>) {}

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
