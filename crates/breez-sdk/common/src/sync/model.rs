use std::fmt::Display;

use bitcoin::hashes::{sha256, Hash};
use semver::Version;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct RecordId {
    pub r#type: String,
    pub data_id: String,
}

impl RecordId {
    pub fn new(r#type: impl Into<String>, data_id: impl Into<String>) -> Self {
        RecordId { r#type: r#type.into(), data_id: data_id.into() }
    }
}

impl Display for RecordId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:x}", sha256::Hash::hash(format!("{}:{}", self.r#type, self.data_id).as_bytes()))
    }
}

pub struct OutgoingRecord {
    pub id: RecordId,
    pub schema_version: Version,
    pub updated_fields: Value,
}

pub struct Record {
    pub id: RecordId,
    pub revision: u64,
    pub schema_version: Version,
    pub data: Vec<u8>,
}

impl From<Record> for crate::sync::proto::Record {
    fn from(record: Record) -> Self {
        crate::sync::proto::Record {
            id: record.id.to_string(),
            revision: record.revision,
            schema_version: record.schema_version.to_string(),
            data: record.data,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_record_id_to_string() {
        let record_id = RecordId::new("test".to_string(), "123".to_string());
        assert_eq!(
            record_id.to_string(),
            "76734be06e3eaa967fa82746bac47e9621f291a5a18222d32016b7febacd4548"
        );
    }
}