use std::time::SystemTime;

pub fn now() -> i64 {
    now_u64().try_into().expect("SystemTime overflow")
}

pub fn now_u64() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("Time went backwards")
        .as_secs()
}

pub fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("Time went backwards")
        .as_millis()
        .try_into()
        .expect("SystemTime overflow")
}
