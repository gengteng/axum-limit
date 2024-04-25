# axum-limit

[![crates.io](https://img.shields.io/crates/v/axum-limit)](https://crates.io/crates/axum-limit)
[![crates.io download](https://img.shields.io/crates/d/axum-limit)](https://crates.io/crates/axum-limit)
[![LICENSE](https://img.shields.io/badge/license-MIT-blue)](https://github.com/gengteng/axum-limit/blob/main/LICENSE)
[![dependency status](https://deps.rs/repo/github/gengteng/axum-limit/status.svg)](https://deps.rs/repo/github/gengteng/axum-limit)
[![GitHub Workflow Status](https://img.shields.io/github/actions/workflow/status/gengteng/axum-limit/.github/workflows/main.yml?branch=main)](https://github.com/gengteng/axum-limit/actions/workflows/ci.yml)
This crate provides an efficient rate limiting mechanism using token buckets, specifically designed for asynchronous web
applications with a strong focus on extractor-based rate limits.

## Features

- Configurable rate limits using extractors, allowing for flexible limit strategies per route.
- Supports various time granularities for rate limits (per second, per minute, per hour, and per day).
- Easily integrates with Axum, using extractors to apply rate limits seamlessly within your application routes.
- Utilizes `DashMap` for concurrent state management across asynchronous tasks.

## Example

Here is a basic example showing how to use the crate with Axum routes:

```rust
use http::Uri;
use axum_limit::{Limit, LimitState, LimitPerSecond};
use axum::{Router, routing::get, response::IntoResponse};

async fn route_handler(_: LimitPerSecond<5, Uri>) -> impl IntoResponse {
    // Handler logic here, automatically enforcing the rate limit
}

fn main() {
    let _app: Router<()> = Router::new()
        .route("/your_route", get(route_handler))
        .with_state(LimitState::<Uri>::default());
}
```

This example demonstrates setting up a rate limit of 5 requests per second on a specific route. The `Limit` extractor
automatically enforces these limits based on the incoming requests.

For more comprehensive examples, please check the `examples` directory in this
repository.
