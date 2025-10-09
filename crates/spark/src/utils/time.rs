use prost_types::Timestamp;
use web_time::{SystemTime, SystemTimeError, UNIX_EPOCH};

pub fn web_time_to_prost_timestamp(system_time: &SystemTime) -> Result<Timestamp, SystemTimeError> {
    let duration = system_time.duration_since(UNIX_EPOCH)?;
    Ok(Timestamp {
        seconds: duration.as_secs() as i64,
        nanos: duration.subsec_nanos() as i32,
    })
}
