use http::header::{HeaderName, HeaderValue, RETRY_AFTER};
use http::HeaderMap;
use std::time::{Duration, Instant};

/// Result of a rate limit check, including metadata for standard response headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimitSnapshot {
    /// Whether the request is allowed.
    pub allowed: bool,
    /// Configured maximum requests per period.
    pub limit: usize,
    /// Remaining requests in the current window or bucket.
    pub remaining: usize,
    /// When the limit fully resets or the next token becomes available.
    pub reset_at: Instant,
}

impl RateLimitSnapshot {
    /// Returns how long to wait before retrying from `now`.
    pub fn retry_after(&self, now: Instant) -> Duration {
        self.reset_at.saturating_duration_since(now)
    }

    /// Returns `Retry-After` in whole seconds, at least 1 when limited.
    pub fn retry_after_secs(&self, now: Instant) -> u64 {
        self.retry_after(now).as_secs().max(1)
    }

    /// Builds standard rate limit headers from this snapshot.
    pub fn to_headers(&self, now: Instant) -> HeaderMap {
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
            unix_timestamp_secs(self.reset_at),
        );

        if !self.allowed {
            insert_header(
                &mut headers,
                RETRY_AFTER,
                self.retry_after_secs(now),
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

fn unix_timestamp_secs(instant: Instant) -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
        .saturating_add(instant.saturating_duration_since(Instant::now()).as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_include_limit_metadata() {
        let now = Instant::now();
        let snapshot = RateLimitSnapshot {
            allowed: false,
            limit: 10,
            remaining: 0,
            reset_at: now + Duration::from_secs(30),
        };

        let headers = snapshot.to_headers(now);
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
