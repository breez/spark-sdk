use platform_utils::time::{Duration, SystemTime, SystemTimeError, UNIX_EPOCH};
use prost_types::Timestamp;

pub fn web_time_to_prost_timestamp(system_time: &SystemTime) -> Result<Timestamp, SystemTimeError> {
    let duration = system_time.duration_since(UNIX_EPOCH)?;
    Ok(Timestamp {
        seconds: duration.as_secs() as i64,
        nanos: duration.subsec_nanos() as i32,
    })
}

/// Bounds of `google.protobuf.Timestamp`: up to 9999-12-31T23:59:59.999999999Z.
const MAX_TIMESTAMP_SECS: u64 = 253_402_300_799;
const NANOS_PER_SEC: u64 = 1_000_000_000;

pub fn prost_timestamp_to_web_time(timestamp: &Timestamp) -> Option<SystemTime> {
    let seconds = prost_timestamp_to_secs(timestamp)?;
    let nanos = u64::try_from(timestamp.nanos)
        .ok()
        .filter(|n| *n < NANOS_PER_SEC)?;
    UNIX_EPOCH
        .checked_add(Duration::from_secs(seconds))?
        .checked_add(Duration::from_nanos(nanos))
}

pub fn prost_timestamp_to_secs(timestamp: &Timestamp) -> Option<u64> {
    let seconds = u64::try_from(timestamp.seconds).ok()?;
    (seconds <= MAX_TIMESTAMP_SECS).then_some(seconds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use macros::test_all;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    fn ts(seconds: i64, nanos: i32) -> Timestamp {
        Timestamp { seconds, nanos }
    }

    #[test_all]
    fn converts_a_valid_timestamp() {
        let converted = prost_timestamp_to_web_time(&ts(1_700_000_000, 500)).unwrap();
        let since_epoch = converted.duration_since(UNIX_EPOCH).unwrap();
        assert_eq!(since_epoch.as_secs(), 1_700_000_000);
        assert_eq!(since_epoch.subsec_nanos(), 500);
    }

    #[test_all]
    fn rejects_negative_seconds() {
        assert!(prost_timestamp_to_web_time(&ts(-1, 0)).is_none());
        assert!(prost_timestamp_to_secs(&ts(-1, 0)).is_none());
    }

    #[test_all]
    fn rejects_negative_nanos() {
        assert!(prost_timestamp_to_web_time(&ts(0, -1)).is_none());
    }

    #[test_all]
    fn rejects_nanos_beyond_one_second() {
        assert!(prost_timestamp_to_web_time(&ts(0, 1_000_000_000)).is_none());
    }

    #[test_all]
    fn rejects_seconds_beyond_representable_time() {
        assert!(prost_timestamp_to_web_time(&ts(i64::MAX, 0)).is_none());
    }

    #[test_all]
    fn reads_seconds_from_a_valid_timestamp() {
        assert_eq!(
            prost_timestamp_to_secs(&ts(1_700_000_000, 0)),
            Some(1_700_000_000)
        );
    }
}
