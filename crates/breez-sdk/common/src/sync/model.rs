use std::{collections::HashMap, fmt::Display};

use bitcoin::hashes::{Hash, sha256};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const CURRENT_SCHEMA_VERSION: Version = Version::new(0, 2, 6);
#[derive(Debug, Clone, Deserialize, Serialize)]
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

pub struct RecordChangeRequest {
    pub id: RecordId,
    pub updated_fields: HashMap<String, Value>,
}

impl From<&RecordChangeRequest> for UnversionedRecordChange {
    fn from(value: &RecordChangeRequest) -> Self {
        UnversionedRecordChange {
            id: value.id.clone(),
            schema_version: CURRENT_SCHEMA_VERSION,
            updated_fields: value.updated_fields.clone(),
        }
    }
}

pub struct UnversionedRecordChange {
    pub id: RecordId,
    pub schema_version: Version,
    pub updated_fields: HashMap<String, Value>,
}

pub struct OutgoingChange {
    pub change: RecordChange,
    pub parent: Option<Record>,
}

impl OutgoingChange {
    pub fn merge(self) -> Record {
        let mut record = Record {
            id: self.change.id.clone(),
            revision: self.change.revision,
            schema_version: self.change.schema_version.clone(),
            data: HashMap::new(),
        };

        if let Some(parent) = self.parent {
            for (k, v) in parent.data {
                record.data.insert(k, v);
            }
        }

        for (k, v) in &self.change.updated_fields {
            record.data.insert(k.clone(), v.clone());
        }

        record
    }
}

pub struct IncomingChange {
    /// The incoming record from remote.
    pub new_state: Record,

    /// The current already existing sync state for this record.
    pub old_state: Option<Record>,
    // Pending outgoing changes are changes that have been applied to the relational data store already, but not to the parent.
    // The incoming change will come _before_ these outgoing changes, so to do things perfectly these changes have to be rolled back
    // in reverse order, then the incoming change applied, then the outgoing changes reapplied in forward order.
    // commented out because unneeded at the moment.
    // pub pending_outgoing_changes: Vec<RecordChange>,
}

pub struct RecordChange {
    pub id: RecordId,
    pub schema_version: Version,
    pub updated_fields: HashMap<String, Value>,
    pub revision: u64,
}

#[derive(Deserialize, Serialize)]
struct SyncData {
    id: RecordId,
    data: HashMap<String, Value>,
}

impl SyncData {
    pub fn new(record: Record) -> Self {
        SyncData {
            id: record.id,
            data: record.data,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Record {
    pub id: RecordId,
    pub revision: u64,
    pub schema_version: Version,
    pub data: HashMap<String, Value>,
}

impl Record {
    #[must_use]
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

        for (k, v) in &self.data {
            record.data.insert(k.clone(), v.clone());
        }

        record
    }

    pub fn change_set(&self, parent: Option<Record>) -> OutgoingChange {
        let mut updated_fields = HashMap::new();

        if let Some(parent) = &parent {
            for (k, v) in &self.data {
                if let Some(parent_value) = parent.data.get(k) {
                    if parent_value != v {
                        updated_fields.insert(k.clone(), v.clone());
                    }
                } else {
                    updated_fields.insert(k.clone(), v.clone());
                }
            }
        } else {
            updated_fields.clone_from(&self.data);
        }

        OutgoingChange {
            change: RecordChange {
                id: self.id.clone(),
                schema_version: self.schema_version.clone(),
                updated_fields,
                revision: self.revision,
            },
            parent,
        }
    }
}

impl TryFrom<&Record> for crate::sync::proto::Record {
    type Error = anyhow::Error;

    fn try_from(record: &Record) -> Result<Self, Self::Error> {
        Ok(crate::sync::proto::Record {
            id: record.id.to_string(),
            revision: record.revision,
            schema_version: record.schema_version.to_string(),
            data: serde_json::to_vec(&SyncData::new(record.clone()))?,
        })
    }
}

impl TryFrom<crate::sync::proto::Record> for Record {
    type Error = anyhow::Error;

    fn try_from(record: crate::sync::proto::Record) -> Result<Self, Self::Error> {
        let sync_data: SyncData = serde_json::from_slice(&record.data)?;
        Ok(Record {
            id: sync_data.id,
            revision: record.revision,
            schema_version: Version::parse(&record.schema_version)?,
            data: sync_data.data,
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
