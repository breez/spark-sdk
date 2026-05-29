//! Cross-target conversions between `platform_utils::time::SystemTime`
//! and `chrono::DateTime<Utc>`.
//!
//! On native, `platform_utils::time::SystemTime` is `std::time::SystemTime`
//! and chrono provides `From<std::time::SystemTime>` for `DateTime<Utc>`,
//! so `.into()` Just Works.
//!
//! On wasm, `platform_utils::time` resolves to the `web_time` crate, and
//! chrono has no `From` impl for `web_time::SystemTime`. Rather than scatter
//! cfg branches at every conversion site, we route everything through these
//! two helpers — they go via the seconds-since-epoch representation, which
//! both `SystemTime` flavours expose identically.

use chrono::{DateTime, Utc};
use platform_utils::time::SystemTime;
use std::time::Duration;

/// Convert a `SystemTime` to a `chrono::DateTime<Utc>`. Returns
/// `UNIX_EPOCH` for any value at or before the epoch (matching the
/// behaviour of `DateTime::from_timestamp(0, 0)`).
pub(crate) fn system_time_to_datetime(t: SystemTime) -> DateTime<Utc> {
    let d = t.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
    // `from_timestamp` returns Option only because of i64-range overflow,
    // which can't happen for a SystemTime we just measured. Fall back to
    // epoch to keep the call site infallible.
    DateTime::<Utc>::from_timestamp(d.as_secs() as i64, d.subsec_nanos())
        .unwrap_or_else(|| DateTime::<Utc>::from_timestamp(0, 0).expect("epoch is valid"))
}

/// Convert a `chrono::DateTime<Utc>` to a `SystemTime`. Values before
/// `UNIX_EPOCH` clamp to the epoch.
pub(crate) fn datetime_to_system_time(dt: DateTime<Utc>) -> SystemTime {
    let secs = dt.timestamp();
    let nanos = dt.timestamp_subsec_nanos();
    if secs < 0 {
        return SystemTime::UNIX_EPOCH;
    }
    SystemTime::UNIX_EPOCH
        + Duration::from_secs(secs as u64)
        + Duration::from_nanos(u64::from(nanos))
}
