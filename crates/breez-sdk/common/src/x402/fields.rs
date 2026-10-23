use serde_json::Value;

pub(super) fn str_field<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field).and_then(Value::as_str)
}

/// A positive integer with no sign, leading zero, decimal point or exponent.
pub(super) fn parse_positive_decimal(s: &str) -> Option<u64> {
    let canonical = s.bytes().next().is_some_and(|b| (b'1'..=b'9').contains(&b))
        && s.bytes().all(|b| b.is_ascii_digit());
    canonical.then(|| s.parse().ok()).flatten()
}

pub(super) fn is_lower_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

pub(super) fn decode_lower_hex_32(s: &str) -> Option<[u8; 32]> {
    if !is_lower_hex(s, 64) {
        return None;
    }
    hex::decode(s).ok()?.try_into().ok()
}
