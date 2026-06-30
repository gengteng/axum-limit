# axum-limit

[![crates.io](https://img.shields.io/crates/v/axum-limit)](https://crates.io/crates/axum-limit)
[![crates.io download](https://img.shields.io/crates/d/axum-limit)](https://crates.io/crates/axum-limit)
[![LICENSE](https://img.shields.io/badge/license-MIT-blue)](https://github.com/gengteng/axum-limit/blob/main/LICENSE)
[![dependency status](https://deps.rs/repo/github/gengteng/axum-limit/status.svg)](https://deps.rs/repo/github/gengteng/axum-limit)
[![GitHub Workflow Status](https://img.shields.io/github/actions/workflow/status/gengteng/axum-limit/.github/workflows/main.yml?branch=main)](https://github.com/gengteng/axum-limit/actions/workflows/main.yml)

This crate provides production-oriented rate limiting for Axum applications using
extractor-based policies, standard response headers, and pluggable algorithms.

## Features

- **Multiple algorithms**: token bucket, fixed window, sliding window counter
- **Extractor-first API**: declare limits directly in handler signatures
- **Standard headers**: `X-RateLimit-*` and `Retry-After` on rejection
- **Per-key isolation**: limits by URI, method, custom keys, or tuples
- **Quota-safe state**: different quotas on the same key do not collide
- **Verified behavior**: deterministic unit tests plus `proptest` property tests

## Algorithms

| Algorithm | Extractor | Best for |
|-----------|-----------|----------|
| Token bucket | `Limit`, `LimitPerSecond` | Burst traffic with smooth sustained rate |
| Fixed window | `FixedWindowLimit`, `FixedWindowPerSecond` | Simple counting, lowest overhead |
| Sliding window | `SlidingWindowLimit`, `SlidingWindowPerSecond` | Fair limits without window-edge spikes |

## Example

```rust
use http::Uri;
use axum_limit::{Limit, LimitState, LimitPerSecond, SlidingWindowPerSecond, SlidingWindowPolicy};
use axum::{Router, routing::get, response::IntoResponse};

async fn token_bucket_handler(_: LimitPerSecond<5, Uri>) -> impl IntoResponse {}

async fn sliding_window_handler(_: SlidingWindowPerSecond<100, Uri>) -> impl IntoResponse {}

fn main() {
    let _app: Router<()> = Router::new()
        .route("/token-bucket", get(token_bucket_handler))
        .with_state(LimitState::<Uri>::default())
        .route("/sliding-window", get(sliding_window_handler))
        .with_state(LimitState::<Uri, SlidingWindowPolicy>::default());
}
```

## Response headers

When a request is rejected, the extractor returns `429 Too Many Requests` with:

- `X-RateLimit-Limit`
- `X-RateLimit-Remaining`
- `X-RateLimit-Reset`
- `Retry-After`

On successful checks, metadata is stored in request extensions. Use
`rate_limit_headers_from_parts` to attach headers to successful responses.

## Production notes

- State is in-process (`DashMap`). For multi-instance deployments, front the
  service with a shared limiter (API gateway, Redis-backed middleware).
- Choose **sliding window** when billing or abuse prevention must avoid fixed-window spikes.
- Choose **token bucket** when clients need short bursts but a clear sustained rate.
- Property tests validate invariants such as `remaining <= limit` across random request timelines.

For more examples, see the `examples/` directory.
