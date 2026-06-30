use std::hash::Hash;
use std::time::Duration;

/// Describes how many requests are allowed within a time period.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Quota {
    /// Maximum sustained requests allowed per [`Self::per`](Quota::per).
    pub max: usize,
    /// Time period in milliseconds.
    pub per_ms: u64,
    /// Optional burst capacity for token-bucket style policies.
    ///
    /// When `None`, policies that support bursting use [`Self::max`](Quota::max) as capacity.
    pub burst: Option<usize>,
}

impl Quota {
    /// Creates a quota allowing `max` requests per `per_ms` milliseconds.
    pub const fn new(max: usize, per_ms: u64) -> Self {
        Self {
            max,
            per_ms,
            burst: None,
        }
    }

    /// Creates a quota with an explicit burst capacity.
    pub const fn with_burst(max: usize, per_ms: u64, burst: usize) -> Self {
        Self {
            max,
            per_ms,
            burst: Some(burst),
        }
    }

    /// Creates a quota allowing `max` requests per second.
    pub const fn per_second(max: usize) -> Self {
        Self::new(max, 1000)
    }

    /// Creates a quota allowing `max` requests per minute.
    pub const fn per_minute(max: usize) -> Self {
        Self::new(max, 60_000)
    }

    /// Creates a quota allowing `max` requests per hour.
    pub const fn per_hour(max: usize) -> Self {
        Self::new(max, 3_600_000)
    }

    /// Creates a quota allowing `max` requests per day.
    pub const fn per_day(max: usize) -> Self {
        Self::new(max, 86_400_000)
    }

    /// Returns the configured period as a [`Duration`].
    pub const fn per(&self) -> Duration {
        Duration::from_millis(self.per_ms)
    }

    /// Returns the burst capacity, defaulting to [`Self::max`](Quota::max).
    pub const fn burst(&self) -> usize {
        match self.burst {
            Some(burst) => burst,
            None => self.max,
        }
    }

    /// Returns a fingerprint used to isolate state for different quotas on the same key.
    pub const fn fingerprint(self) -> QuotaFingerprint {
        QuotaFingerprint {
            max: self.max,
            per_ms: self.per_ms,
            burst: self.burst,
        }
    }
}

/// Identifies a unique quota configuration for per-key state lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QuotaFingerprint {
    /// Maximum sustained requests allowed per period.
    pub max: usize,
    /// Time period in milliseconds.
    pub per_ms: u64,
    /// Optional burst capacity.
    pub burst: Option<usize>,
}
