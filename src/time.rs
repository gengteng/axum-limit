use std::time::{SystemTime, UNIX_EPOCH};

/// Returns the current UTC time in milliseconds since the Unix epoch.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// Converts milliseconds since the Unix epoch to whole seconds for HTTP headers.
pub const fn millis_to_secs(ms: u64) -> u64 {
    ms / 1000
}

/// Saturating difference between two millisecond timestamps.
pub const fn saturating_sub_ms(later_ms: u64, earlier_ms: u64) -> u64 {
    later_ms.saturating_sub(earlier_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_ms_is_monotonicish() {
        let first = now_ms();
        let second = now_ms();
        assert!(second >= first);
    }
}
