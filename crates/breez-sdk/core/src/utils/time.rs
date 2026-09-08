use platform_utils::time::{SystemTime, UNIX_EPOCH};

use crate::error::SdkError;

/// Seconds since the Unix epoch, reading 0 on a clock set before it.
pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Milliseconds since the Unix epoch, reading 0 on a clock set before it.
pub(crate) fn now_ms() -> u128 {
    checked_now_ms().unwrap_or(0)
}

/// Milliseconds since the Unix epoch, `None` on a clock set before it, for
/// callers whose 0 reading would keep something alive that should expire.
pub(crate) fn checked_now_ms() -> Option<u128> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis())
}

/// Seconds since the Unix epoch, for callers that must refuse to act on a
/// clock set before it rather than read it as 0.
pub(crate) fn try_now_secs() -> Result<u64, SdkError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| SdkError::Generic("System clock is before the Unix epoch".to_string()))
}
