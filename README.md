# axum-limit

[![crates.io](https://img.shields.io/crates/v/axum-limit)](https://crates.io/crates/axum-limit)
[![crates.io download](https://img.shields.io/crates/d/axum-limit)](https://crates.io/crates/axum-limit)
[![LICENSE](https://img.shields.io/badge/license-MIT-blue)](https://github.com/gengteng/axum-limit/blob/main/LICENSE)
[![dependency status](https://deps.rs/repo/github/gengteng/axum-limit/status.svg)](https://deps.rs/repo/github/gengteng/axum-limit)
[![GitHub Workflow Status](https://img.shields.io/github/actions/workflow/status/gengteng/axum-limit/.github/workflows/main.yml?branch=main)](https://github.com/gengteng/axum-limit/actions/workflows/main.yml)

Production-oriented rate limiting for Axum with pluggable algorithms and storage backends.

## Features

- **Algorithms**: token bucket, fixed window, sliding window counter
- **Backends**: in-memory (single node), Redis (multi-node, `redis` feature), custom via [`RateLimitBackend`]
- **Extractor API**: declare limits in handler signatures
- **Standard headers**: `X-RateLimit-*` and `Retry-After`
- **Verified behavior**: deterministic tests + `proptest`

## Quick start (memory backend)

```rust,no_run
use http::Uri;
use axum_limit::{LimitPerSecond, LimitState};
use axum::{Router, routing::get, response::IntoResponse};

async fn handler(_: LimitPerSecond<5, Uri>) -> impl IntoResponse {}

#[tokio::main]
async fn main() {
    let app: Router<()> = Router::new()
        .route("/api", get(handler))
        .with_state(LimitState::<Uri>::default());

    // axum::serve(..., app).await
}
```

## Redis backend (multi-node)

Enable the `redis` feature, then use [`RedisBackend`] and specify it on the extractor:

```rust,no_run
# #[cfg(feature = "redis")]
# async fn example() -> Result<(), axum_limit::BackendError> {
use axum_limit::{LimitPerSecond, LimitState, RedisBackend, TokenBucketPolicy};
use http::Uri;

let backend = RedisBackend::connect("redis://127.0.0.1/").await?;
let state = LimitState::<Uri, TokenBucketPolicy, RedisBackend>::new(backend);

async fn handler(_: LimitPerSecond<5, Uri, RedisBackend>) -> impl axum::response::IntoResponse {}

let _app: axum::Router<LimitState<Uri, TokenBucketPolicy, RedisBackend>> = axum::Router::new()
    .route("/api", axum::routing::get(handler))
    .with_state(state);
# Ok(())
# }
```

## Custom backend

Implement [`RateLimitBackend`] and use [`apply_policy`] to run algorithms against your storage:

```rust,no_run
use axum_limit::{
    apply_policy, BackendError, RateLimitBackend, RateLimitPolicy, RateLimitSnapshot, Quota,
    TokenBucketPolicy,
};
use async_trait::async_trait;

#[derive(Clone)]
struct MyBackend;

async fn load(_key: &str) -> Result<Option<Vec<u8>>, BackendError> {
    Ok(None)
}

async fn save(_key: &str, _value: Vec<u8>) -> Result<(), BackendError> {
    Ok(())
}

#[async_trait]
impl RateLimitBackend for MyBackend {
    type Error = BackendError;

    fn namespace(&self) -> &str {
        "my-app"
    }

    async fn transact<P>(
        &self,
        storage_key: &str,
        quota: Quota,
        now_ms: u64,
    ) -> Result<RateLimitSnapshot, Self::Error>
    where
        P: RateLimitPolicy,
    {
        let payload = load(storage_key).await?;
        let (encoded, snapshot) = apply_policy::<P>(payload.as_deref(), quota, now_ms)?;
        save(storage_key, encoded).await?;
        Ok(snapshot)
    }
}
```

Custom keys must implement [`StorageKey`] in addition to [`Key`].

## Storage model

- Policy state is serialized as JSON under keys like `{namespace}:{policy}:{quota}:{subject}`
- UTC millisecond timestamps keep nodes consistent
- Different quotas on the same subject are isolated automatically

## Algorithms

| Algorithm | Extractor | Best for |
|-----------|-----------|----------|
| Token bucket | `Limit`, `LimitPerSecond` | Bursts with smooth sustained rate |
| Fixed window | `FixedWindowLimit` | Lowest overhead |
| Sliding window | `SlidingWindowLimit` | Fair limits without window spikes |

See the `examples/` directory for more.
