use std::collections::HashMap;

use thiserror::Error;

use crate::sync::RecordId;

/// Errors that can occur during storage operations
#[derive(Debug, Error, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Error))]
pub enum StorageError {
    #[error("Underline implementation error: {0}")]
    Implementation(String),

    /// Database initialization error
    #[error("Failed to initialize database: {0}")]
    InitializationError(String),

    #[error("Failed to serialize/deserialize data: {0}")]
    Serialization(String),
}

impl From<semver::Error> for StorageError {
    fn from(e: semver::Error) -> Self {
        StorageError::Serialization(e.to_string())
    }
}

impl From<serde_json::Error> for StorageError {
    fn from(e: serde_json::Error) -> Self {
        StorageError::Serialization(e.to_string())
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
#[macros::async_trait]
pub trait SyncStorage: Send + Sync {
    async fn add_outgoing_change(
        &self,
        record: UnversionedRecordChange,
    ) -> Result<u64, StorageError>;
    async fn complete_outgoing_sync(&self, record: Record) -> Result<(), StorageError>;
    async fn get_pending_outgoing_changes(
        &self,
        limit: u32,
    ) -> Result<Vec<OutgoingChange>, StorageError>;

    /// Get the revision number of the last synchronized record
    async fn get_last_revision(&self) -> Result<u64, StorageError>;

    /// Insert incoming records from remote sync
    async fn insert_incoming_records(&self, records: Vec<Record>) -> Result<(), StorageError>;

    /// Delete an incoming record after it has been processed
    async fn delete_incoming_record(&self, record: Record) -> Result<(), StorageError>;

    /// Update revision numbers of pending outgoing records to be higher than the given revision
    async fn rebase_pending_outgoing_records(&self, revision: u64) -> Result<(), StorageError>;

    /// Get incoming records that need to be processed, up to the specified limit
    async fn get_incoming_records(&self, limit: u32) -> Result<Vec<IncomingChange>, StorageError>;

    /// Get the latest outgoing record if any exists
    async fn get_latest_outgoing_change(&self) -> Result<Option<OutgoingChange>, StorageError>;

    /// Update the sync state record from an incoming record
    async fn update_record_from_incoming(&self, record: Record) -> Result<(), StorageError>;
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct IncomingChange {
    pub new_state: Record,
    pub old_state: Option<Record>,
    // pub pending_outgoing_changes: Vec<RecordChange>,
}

impl TryFrom<&IncomingChange> for crate::sync::model::IncomingChange {
    type Error = StorageError;

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

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OutgoingChange {
    pub change: RecordChange,
    pub parent: Option<Record>,
}

impl TryFrom<OutgoingChange> for crate::sync::model::OutgoingChange {
    type Error = StorageError;

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

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct UnversionedRecordChange {
    pub id: RecordId,
    pub schema_version: String,
    pub updated_fields: HashMap<String, String>,
}

impl TryFrom<UnversionedRecordChange> for crate::sync::model::UnversionedRecordChange {
    type Error = StorageError;

    fn try_from(value: UnversionedRecordChange) -> Result<Self, Self::Error> {
        Ok(crate::sync::model::UnversionedRecordChange {
            id: value.id,
            schema_version: value.schema_version.parse()?,
            updated_fields: value
                .updated_fields
                .into_iter()
                .map(|(k, v)| Ok((k, serde_json::from_str(&v)?)))
                .collect::<Result<HashMap<String, serde_json::Value>, StorageError>>()?,
        })
    }
}

impl TryFrom<crate::sync::model::UnversionedRecordChange> for UnversionedRecordChange {
    type Error = StorageError;

    fn try_from(value: crate::sync::model::UnversionedRecordChange) -> Result<Self, Self::Error> {
        Ok(UnversionedRecordChange {
            id: value.id,
            schema_version: value.schema_version.to_string(),
            updated_fields: value
                .updated_fields
                .into_iter()
                .map(|(k, v)| Ok((k, serde_json::to_string(&v)?)))
                .collect::<Result<HashMap<String, String>, StorageError>>()?,
        })
    }
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RecordChange {
    pub id: RecordId,
    pub schema_version: String,
    pub updated_fields: HashMap<String, String>,
    pub revision: u64,
}

impl TryFrom<RecordChange> for crate::sync::model::RecordChange {
    type Error = StorageError;

    fn try_from(value: RecordChange) -> Result<Self, Self::Error> {
        Ok(crate::sync::model::RecordChange {
            id: value.id,
            schema_version: value.schema_version.parse()?,
            updated_fields: value
                .updated_fields
                .into_iter()
                .map(|(k, v)| Ok((k, serde_json::from_str(&v)?)))
                .collect::<Result<HashMap<String, serde_json::Value>, StorageError>>()?,
            revision: value.revision,
        })
    }
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Clone)]
pub struct Record {
    pub id: RecordId,
    pub revision: u64,
    pub schema_version: String,
    pub data: HashMap<String, String>,
}

impl TryFrom<&Record> for crate::sync::model::Record {
    type Error = StorageError;

    fn try_from(value: &Record) -> Result<Self, Self::Error> {
        Ok(crate::sync::model::Record {
            id: value.id.clone(),
            schema_version: value.schema_version.parse()?,
            data: value
                .data
                .iter()
                .map(|(k, v)| Ok((k.clone(), serde_json::from_str(v)?)))
                .collect::<Result<HashMap<String, serde_json::Value>, StorageError>>()?,
            revision: value.revision,
        })
    }
}

impl TryFrom<&crate::sync::model::Record> for Record {
    type Error = StorageError;

    fn try_from(value: &crate::sync::model::Record) -> Result<Self, Self::Error> {
        Ok(Record {
            id: value.id.clone(),
            schema_version: value.schema_version.to_string(),
            data: value
                .data
                .iter()
                .map(|(k, v)| Ok((k.clone(), serde_json::to_string(&v)?)))
                .collect::<Result<HashMap<String, String>, StorageError>>()?,
            revision: value.revision,
        })
    }
}
