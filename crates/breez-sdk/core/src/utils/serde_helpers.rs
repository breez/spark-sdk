//! Shared serde helpers for `u128` fields that need to survive JSON
//! round-trips through `serde_json::Value` (which doesn't support `u128`).
//!
//! These are used wherever `u128` appears inside an internally-tagged enum
//! (`#[serde(tag = "...")]`), where serde deserializes via an intermediate
//! `Value`. They serialize as JSON strings and accept both strings and numbers
//! on the way in for backwards compatibility with data stored before the enum
//! migration.

/// Serialize/deserialize `u128` as a JSON string. Accepts both `"123"` and
/// `123` when deserializing.
pub mod serde_u128_as_string {
    use serde::{self, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &u128, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&value.to_string())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<u128, D::Error>
    where
        D: Deserializer<'de>,
    {
        let v = serde_json::Value::deserialize(deserializer)?;
        match v {
            serde_json::Value::String(s) => s.parse::<u128>().map_err(serde::de::Error::custom),
            serde_json::Value::Number(n) => n.as_u64().map(u128::from).ok_or_else(|| {
                serde::de::Error::custom(format!("number {n} cannot be represented as u128"))
            }),
            other => Err(serde::de::Error::custom(format!(
                "expected string or number for u128, got {other}"
            ))),
        }
    }
}

/// Serialize/deserialize `Option<u128>` as an optional JSON string. Accepts
/// `"123"`, `123`, or `null` when deserializing.
pub mod serde_option_u128_as_string {
    use serde::{self, Deserialize, Deserializer, Serializer};

    #[allow(clippy::ref_option)]
    pub fn serialize<S>(value: &Option<u128>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(v) => serializer.serialize_some(&v.to_string()),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<u128>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let opt: Option<serde_json::Value> = Option::deserialize(deserializer)?;
        match opt {
            None | Some(serde_json::Value::Null) => Ok(None),
            Some(serde_json::Value::String(s)) => s
                .parse::<u128>()
                .map(Some)
                .map_err(serde::de::Error::custom),
            Some(serde_json::Value::Number(n)) => {
                if let Some(v) = n.as_u64() {
                    Ok(Some(u128::from(v)))
                } else {
                    Err(serde::de::Error::custom(format!(
                        "number {n} cannot be represented as u128"
                    )))
                }
            }
            Some(other) => Err(serde::de::Error::custom(format!(
                "expected string or number for u128, got {other}"
            ))),
        }
    }
}
