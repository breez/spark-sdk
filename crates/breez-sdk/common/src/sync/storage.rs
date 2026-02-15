use std::collections::HashMap;

use thiserror::Error;

pub use crate::sync::RecordId;

/// Errors that can occur during storage operations
#[derive(Debug, Error, Clone)]
pub enum SyncStorageError {
    #[error("Underline implementation error: {0}")]
    Implementation(String),

    /// Database initialization error
    #[error("Failed to initialize database: {0}")]
    InitializationError(String),

    #[error("Failed to serialize/deserialize data: {0}")]
    Serialization(String),
}

impl From<serde_json::Error> for SyncStorageError {
    fn from(e: serde_json::Error) -> Self {
        SyncStorageError::Serialization(e.to_string())
    }
}

impl From<String> for SyncStorageError {
    fn from(e: String) -> Self {
        SyncStorageError::Serialization(e)
    }
}

#[cfg_attr(test, mockall::automock)]
#[macros::async_trait]
pub trait SyncStorage: Send + Sync {
    async fn add_outgoing_change(
        &self,
        record: UnversionedRecordChange,
    ) -> Result<u64, SyncStorageError>;
    async fn complete_outgoing_sync(
        &self,
        record: Record,
        local_revision: u64,
    ) -> Result<(), SyncStorageError>;
    async fn get_pending_outgoing_changes(
        &self,
        limit: u32,
    ) -> Result<Vec<OutgoingChange>, SyncStorageError>;

    /// Get the revision number of the last synchronized record
    async fn get_last_revision(&self) -> Result<u64, SyncStorageError>;

    /// Insert incoming records from remote sync
    async fn insert_incoming_records(&self, records: Vec<Record>) -> Result<(), SyncStorageError>;

    /// Delete an incoming record after it has been processed
    async fn delete_incoming_record(&self, record: Record) -> Result<(), SyncStorageError>;

    /// Get incoming records that need to be processed, up to the specified limit
    async fn get_incoming_records(
        &self,
        limit: u32,
    ) -> Result<Vec<IncomingChange>, SyncStorageError>;

    /// Get the latest outgoing record if any exists
    async fn get_latest_outgoing_change(&self) -> Result<Option<OutgoingChange>, SyncStorageError>;

    /// Update the sync state record from an incoming record
    async fn update_record_from_incoming(&self, record: Record) -> Result<(), SyncStorageError>;
}

#[derive(Clone, Debug)]
pub struct IncomingChange {
    pub new_state: Record,
    pub old_state: Option<Record>,
    // pub pending_outgoing_changes: Vec<RecordChange>,
}

impl TryFrom<&IncomingChange> for crate::sync::model::IncomingChange {
    type Error = SyncStorageError;

    fn try_from(value: &IncomingChange) -> Result<Self, Self::Error> {
        Ok(crate::sync::model::IncomingChange {
            new_state: (&value.new_state).try_into()?,
            old_state: match &value.old_state {
                Some(old_state) => Some(old_state.try_into()?),
                None => None,
            },
        })
    }
}

#[derive(Clone, Debug)]
pub struct OutgoingChange {
    pub change: RecordChange,
    pub parent: Option<Record>,
}

impl TryFrom<OutgoingChange> for crate::sync::model::OutgoingChange {
    type Error = SyncStorageError;

    fn try_from(value: OutgoingChange) -> Result<Self, Self::Error> {
        Ok(crate::sync::model::OutgoingChange {
            change: value.change.try_into()?,
            parent: match value.parent {
                Some(parent) => Some((&parent).try_into()?),
                None => None,
            },
        })
    }
}

#[derive(Clone, Debug)]
pub struct UnversionedRecordChange {
    pub id: RecordId,
    pub schema_version: String,
    pub updated_fields: HashMap<String, String>,
}

impl TryFrom<UnversionedRecordChange> for crate::sync::model::UnversionedRecordChange {
    type Error = SyncStorageError;

    fn try_from(value: UnversionedRecordChange) -> Result<Self, Self::Error> {
        Ok(crate::sync::model::UnversionedRecordChange {
            id: value.id,
            schema_version: value.schema_version.parse()?,
            updated_fields: value
                .updated_fields
                .into_iter()
                .map(|(k, v)| Ok((k, serde_json::from_str(&v)?)))
                .collect::<Result<HashMap<String, serde_json::Value>, SyncStorageError>>()?,
        })
    }
}

impl TryFrom<crate::sync::model::UnversionedRecordChange> for UnversionedRecordChange {
    type Error = SyncStorageError;

    fn try_from(value: crate::sync::model::UnversionedRecordChange) -> Result<Self, Self::Error> {
        Ok(UnversionedRecordChange {
            id: value.id,
            schema_version: value.schema_version.to_string(),
            updated_fields: value
                .updated_fields
                .into_iter()
                .map(|(k, v)| Ok((k, serde_json::to_string(&v)?)))
                .collect::<Result<HashMap<String, String>, SyncStorageError>>()?,
        })
    }
}

#[derive(Clone, Debug)]
pub struct RecordChange {
    pub id: RecordId,
    pub schema_version: String,
    pub updated_fields: HashMap<String, String>,
    /// Local queue id used to keep pending outgoing ordering stable.
    pub local_revision: u64,
}

impl TryFrom<RecordChange> for crate::sync::model::RecordChange {
    type Error = SyncStorageError;

    fn try_from(value: RecordChange) -> Result<Self, Self::Error> {
        Ok(crate::sync::model::RecordChange {
            id: value.id,
            schema_version: value.schema_version.parse()?,
            updated_fields: value
                .updated_fields
                .into_iter()
                .map(|(k, v)| Ok((k, serde_json::from_str(&v)?)))
                .collect::<Result<HashMap<String, serde_json::Value>, SyncStorageError>>()?,
            local_revision: value.local_revision,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Record {
    pub id: RecordId,
    pub revision: u64,
    pub schema_version: String,
    pub data: HashMap<String, String>,
}

impl TryFrom<&Record> for crate::sync::model::Record {
    type Error = SyncStorageError;

    fn try_from(value: &Record) -> Result<Self, Self::Error> {
        Ok(crate::sync::model::Record {
            id: value.id.clone(),
            schema_version: value.schema_version.parse()?,
            data: value
                .data
                .iter()
                .map(|(k, v)| Ok((k.clone(), serde_json::from_str(v)?)))
                .collect::<Result<HashMap<String, serde_json::Value>, SyncStorageError>>()?,
            revision: value.revision,
        })
    }
}

impl TryFrom<&crate::sync::model::Record> for Record {
    type Error = SyncStorageError;

    fn try_from(value: &crate::sync::model::Record) -> Result<Self, Self::Error> {
        Ok(Record {
            id: value.id.clone(),
            schema_version: value.schema_version.to_string(),
            data: value
                .data
                .iter()
                .map(|(k, v)| Ok((k.clone(), serde_json::to_string(&v)?)))
                .collect::<Result<HashMap<String, String>, SyncStorageError>>()?,
            revision: value.revision,
        })
    }
}
