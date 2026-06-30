use crate::time::{millis_to_secs, saturating_sub_ms};
use http::header::{HeaderName, HeaderValue, RETRY_AFTER};
use http::HeaderMap;

/// Result of a rate limit check, including metadata for standard response headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimitSnapshot {
    /// Whether the request is allowed.
    pub allowed: bool,
    /// Configured maximum requests per period.
    pub limit: usize,
    /// Remaining requests in the current window or bucket.
    pub remaining: usize,
    /// UTC timestamp in milliseconds when the limit fully resets.
    pub reset_at_ms: u64,
}

impl RateLimitSnapshot {
    /// Returns how long to wait before retrying from `now_ms`.
    pub fn retry_after_ms(&self, now_ms: u64) -> u64 {
        saturating_sub_ms(self.reset_at_ms, now_ms)
    }

    /// Returns `Retry-After` in whole seconds, at least 1 when limited.
    pub fn retry_after_secs(&self, now_ms: u64) -> u64 {
        self.retry_after_ms(now_ms).div_ceil(1000).max(1)
    }

    /// Builds standard rate limit headers from this snapshot.
    pub fn to_headers(&self, now_ms: u64) -> HeaderMap {
        let mut headers = HeaderMap::new();
        insert_header(
            &mut headers,
            HeaderName::from_static("x-ratelimit-limit"),
            self.limit,
        );
        insert_header(
            &mut headers,
            HeaderName::from_static("x-ratelimit-remaining"),
            self.remaining,
        );
        insert_header(
            &mut headers,
            HeaderName::from_static("x-ratelimit-reset"),
            millis_to_secs(self.reset_at_ms),
        );

        if !self.allowed {
            insert_header(
                &mut headers,
                RETRY_AFTER,
                self.retry_after_secs(now_ms),
            );
        }

        headers
    }
}

/// Extension type stored in request parts after a successful limit check.
#[derive(Debug, Clone, Copy)]
pub struct RateLimitInfo(pub RateLimitSnapshot);

fn insert_header(headers: &mut HeaderMap, name: HeaderName, value: impl ToString) {
    if let Ok(value) = HeaderValue::from_str(&value.to_string()) {
        headers.insert(name, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::time::now_ms;

    #[test]
    fn headers_include_limit_metadata() {
        let now_ms = now_ms();
        let snapshot = RateLimitSnapshot {
            allowed: false,
            limit: 10,
            remaining: 0,
            reset_at_ms: now_ms + 30_000,
        };

        let headers = snapshot.to_headers(now_ms);
        assert_eq!(
            headers
                .get("x-ratelimit-limit")
                .and_then(|v| v.to_str().ok()),
            Some("10")
        );
        assert_eq!(
            headers
                .get("x-ratelimit-remaining")
                .and_then(|v| v.to_str().ok()),
            Some("0")
        );
        assert!(headers.get(RETRY_AFTER).is_some());
    }
}
