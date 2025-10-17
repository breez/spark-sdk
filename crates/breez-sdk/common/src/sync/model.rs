use std::{collections::HashMap, fmt::Display};

use bitcoin::hashes::{Hash, sha256};
use semver::Version;
use serde_json::Value;

const CURRENT_SCHEMA_VERSION: Version = Version::new(0, 2, 6);
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RecordId {
    pub r#type: String,
    pub data_id: String,
}

impl RecordId {
    pub fn new(r#type: impl Into<String>, data_id: impl Into<String>) -> Self {
        RecordId {
            r#type: r#type.into(),
            data_id: data_id.into(),
        }
    }
}

impl Display for RecordId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:x}",
            sha256::Hash::hash(format!("{}:{}", self.r#type, self.data_id).as_bytes())
        )
    }
}

pub struct OutgoingRecordRequest {
    pub id: RecordId,
    pub updated_fields: HashMap<String, Value>,
}

impl From<&OutgoingRecordRequest> for UnversionedOutgoingRecord {
    fn from(value: &OutgoingRecordRequest) -> Self {
        UnversionedOutgoingRecord {
            id: value.id.clone(),
            schema_version: CURRENT_SCHEMA_VERSION,
            updated_fields: value.updated_fields.clone(),
        }
    }
}

pub struct UnversionedOutgoingRecord {
    pub id: RecordId,
    pub schema_version: Version,
    pub updated_fields: HashMap<String, Value>,
}

pub struct OutgoingRecord {
    pub id: RecordId,
    pub schema_version: Version,
    pub updated_fields: HashMap<String, Value>,
    pub revision: u64,
}

impl OutgoingRecord {
    pub fn with_parent(&self, parent: Option<Record>) -> Record {
        let mut record = Record {
            id: self.id.clone(),
            revision: self.revision,
            schema_version: self.schema_version.clone(),
            data: HashMap::new(),
        };

        if let Some(parent) = parent {
            for (k, v) in parent.data {
                record.data.insert(k, v);
            }
        }

        for (k, v) in &self.updated_fields {
            record.data.insert(k.clone(), v.clone());
        }

        record
    }
}

pub struct Record {
    pub id: RecordId,
    pub revision: u64,
    pub schema_version: Version,
    pub data: HashMap<String, Value>,
}

impl TryFrom<&Record> for crate::sync::proto::Record {
    type Error = anyhow::Error;

    fn try_from(record: &Record) -> Result<Self, Self::Error> {
        Ok(crate::sync::proto::Record {
            id: record.id.to_string(),
            revision: record.revision,
            schema_version: record.schema_version.to_string(),
            data: serde_json::to_vec(&record.data)?,
        })
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
