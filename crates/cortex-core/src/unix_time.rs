//! Clock times as the store and the protocol hold them: UTC milliseconds since 1970.

use std::time::{SystemTime, UNIX_EPOCH};

/// `time` in UTC milliseconds since 1970. A clock set before 1970 reads as 0, and a time past what
/// 64 bits of milliseconds hold as the largest.
#[must_use]
pub fn unix_millis(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH).map_or(0, |since| {
        i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// 1,700,000,000,123 ms after 1970, and a quarter of a millisecond later, both read as that
    /// millisecond; a second before 1970 reads as 0.
    #[test]
    fn times_are_whole_milliseconds_since_1970() {
        let time = UNIX_EPOCH + Duration::from_millis(1_700_000_000_123);
        assert_eq!(unix_millis(time), 1_700_000_000_123);
        let later = time + Duration::from_micros(250);
        assert_eq!(unix_millis(later), 1_700_000_000_123);
        assert_eq!(unix_millis(UNIX_EPOCH - Duration::from_secs(1)), 0);
    }
}
