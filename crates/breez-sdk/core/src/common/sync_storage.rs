use std::{collections::HashMap, sync::Arc};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors that can occur during storage operations
#[derive(Debug, Error, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Error))]
pub enum SyncStorageError {
    #[error("Underline implementation error: {0}")]
    Implementation(String),

    /// Database initialization error
    #[error("Failed to initialize database: {0}")]
    InitializationError(String),

    #[error("Failed to serialize/deserialize data: {0}")]
    Serialization(String),
}

impl From<SyncStorageError> for breez_sdk_common::sync::storage::SyncStorageError {
    fn from(value: SyncStorageError) -> Self {
        match value {
            SyncStorageError::Implementation(msg) => {
                breez_sdk_common::sync::storage::SyncStorageError::Implementation(msg)
            }
            SyncStorageError::InitializationError(msg) => {
                breez_sdk_common::sync::storage::SyncStorageError::InitializationError(msg)
            }
            SyncStorageError::Serialization(msg) => {
                breez_sdk_common::sync::storage::SyncStorageError::Serialization(msg)
            }
        }
    }
}

impl From<serde_json::Error> for SyncStorageError {
    fn from(e: serde_json::Error) -> Self {
        SyncStorageError::Serialization(e.to_string())
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
#[macros::async_trait]
pub trait SyncStorage: Send + Sync {
    async fn add_outgoing_change(
        &self,
        record: UnversionedRecordChange,
    ) -> Result<u64, SyncStorageError>;
    async fn complete_outgoing_sync(&self, record: Record) -> Result<(), SyncStorageError>;
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

    /// Update revision numbers of pending outgoing records to be higher than the given revision
    async fn rebase_pending_outgoing_records(&self, revision: u64) -> Result<(), SyncStorageError>;

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

pub(crate) struct SyncStorageWrapper {
    pub inner: Arc<dyn SyncStorage>,
}

impl SyncStorageWrapper {
    pub fn new(inner: Arc<dyn SyncStorage>) -> Self {
        Self { inner }
    }
}

#[macros::async_trait]
impl breez_sdk_common::sync::storage::SyncStorage for SyncStorageWrapper {
    async fn add_outgoing_change(
        &self,
        record: breez_sdk_common::sync::storage::UnversionedRecordChange,
    ) -> Result<u64, breez_sdk_common::sync::storage::SyncStorageError> {
        Ok(self.inner.add_outgoing_change(record.into()).await?)
    }

    async fn complete_outgoing_sync(
        &self,
        record: breez_sdk_common::sync::storage::Record,
    ) -> Result<(), breez_sdk_common::sync::storage::SyncStorageError> {
        Ok(self.inner.complete_outgoing_sync(record.into()).await?)
    }

    async fn get_pending_outgoing_changes(
        &self,
        limit: u32,
    ) -> Result<
        Vec<breez_sdk_common::sync::storage::OutgoingChange>,
        breez_sdk_common::sync::storage::SyncStorageError,
    > {
        let changes = self.inner.get_pending_outgoing_changes(limit).await?;
        Ok(changes.into_iter().map(From::from).collect())
    }

    async fn get_last_revision(
        &self,
    ) -> Result<u64, breez_sdk_common::sync::storage::SyncStorageError> {
        Ok(self.inner.get_last_revision().await?)
    }

    async fn insert_incoming_records(
        &self,
        records: Vec<breez_sdk_common::sync::storage::Record>,
    ) -> Result<(), breez_sdk_common::sync::storage::SyncStorageError> {
        let recs: Vec<Record> = records.into_iter().map(From::from).collect();
        Ok(self.inner.insert_incoming_records(recs).await?)
    }

    async fn delete_incoming_record(
        &self,
        record: breez_sdk_common::sync::storage::Record,
    ) -> Result<(), breez_sdk_common::sync::storage::SyncStorageError> {
        Ok(self.inner.delete_incoming_record(record.into()).await?)
    }

    async fn rebase_pending_outgoing_records(
        &self,
        revision: u64,
    ) -> Result<(), breez_sdk_common::sync::storage::SyncStorageError> {
        Ok(self.inner.rebase_pending_outgoing_records(revision).await?)
    }

    async fn get_incoming_records(
        &self,
        limit: u32,
    ) -> Result<
        Vec<breez_sdk_common::sync::storage::IncomingChange>,
        breez_sdk_common::sync::storage::SyncStorageError,
    > {
        let changes = self.inner.get_incoming_records(limit).await?;
        Ok(changes.into_iter().map(From::from).collect())
    }

    async fn get_latest_outgoing_change(
        &self,
    ) -> Result<
        Option<breez_sdk_common::sync::storage::OutgoingChange>,
        breez_sdk_common::sync::storage::SyncStorageError,
    > {
        let change = self.inner.get_latest_outgoing_change().await?;
        Ok(change.map(From::from))
    }

    async fn update_record_from_incoming(
        &self,
        record: breez_sdk_common::sync::storage::Record,
    ) -> Result<(), breez_sdk_common::sync::storage::SyncStorageError> {
        Ok(self
            .inner
            .update_record_from_incoming(record.into())
            .await?)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RecordId {
    pub r#type: String,
    pub data_id: String,
}

impl RecordId {
    pub fn new(r#type: String, data_id: String) -> Self {
        RecordId { r#type, data_id }
    }
}

impl From<breez_sdk_common::sync::storage::RecordId> for RecordId {
    fn from(value: breez_sdk_common::sync::storage::RecordId) -> Self {
        RecordId {
            r#type: value.r#type,
            data_id: value.data_id,
        }
    }
}

impl From<RecordId> for breez_sdk_common::sync::storage::RecordId {
    fn from(value: RecordId) -> Self {
        breez_sdk_common::sync::storage::RecordId {
            r#type: value.r#type,
            data_id: value.data_id,
        }
    }
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Clone, Debug)]
pub struct IncomingChange {
    pub new_state: Record,
    pub old_state: Option<Record>,
}

impl From<breez_sdk_common::sync::storage::IncomingChange> for IncomingChange {
    fn from(value: breez_sdk_common::sync::storage::IncomingChange) -> Self {
        IncomingChange {
            new_state: value.new_state.into(),
            old_state: value.old_state.map(From::from),
        }
    }
}

impl From<IncomingChange> for breez_sdk_common::sync::storage::IncomingChange {
    fn from(value: IncomingChange) -> Self {
        breez_sdk_common::sync::storage::IncomingChange {
            new_state: value.new_state.into(),
            old_state: value.old_state.map(From::from),
        }
    }
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Clone, Debug)]
pub struct OutgoingChange {
    pub change: RecordChange,
    pub parent: Option<Record>,
}

impl From<breez_sdk_common::sync::storage::OutgoingChange> for OutgoingChange {
    fn from(value: breez_sdk_common::sync::storage::OutgoingChange) -> Self {
        OutgoingChange {
            change: value.change.into(),
            parent: value.parent.map(From::from),
        }
    }
}

impl From<OutgoingChange> for breez_sdk_common::sync::storage::OutgoingChange {
    fn from(value: OutgoingChange) -> Self {
        breez_sdk_common::sync::storage::OutgoingChange {
            change: value.change.into(),
            parent: value.parent.map(From::from),
        }
    }
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Clone, Debug)]
pub struct UnversionedRecordChange {
    pub id: RecordId,
    pub schema_version: String,
    pub updated_fields: HashMap<String, String>,
}

impl From<breez_sdk_common::sync::storage::UnversionedRecordChange> for UnversionedRecordChange {
    fn from(value: breez_sdk_common::sync::storage::UnversionedRecordChange) -> Self {
        UnversionedRecordChange {
            id: value.id.into(),
            schema_version: value.schema_version,
            updated_fields: value.updated_fields,
        }
    }
}

impl From<UnversionedRecordChange> for breez_sdk_common::sync::storage::UnversionedRecordChange {
    fn from(value: UnversionedRecordChange) -> Self {
        breez_sdk_common::sync::storage::UnversionedRecordChange {
            id: value.id.into(),
            schema_version: value.schema_version,
            updated_fields: value.updated_fields,
        }
    }
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Clone, Debug)]
pub struct RecordChange {
    pub id: RecordId,
    pub schema_version: String,
    pub updated_fields: HashMap<String, String>,
    pub revision: u64,
}

impl From<breez_sdk_common::sync::storage::RecordChange> for RecordChange {
    fn from(value: breez_sdk_common::sync::storage::RecordChange) -> Self {
        RecordChange {
            id: value.id.into(),
            schema_version: value.schema_version,
            updated_fields: value.updated_fields,
            revision: value.revision,
        }
    }
}

impl From<RecordChange> for breez_sdk_common::sync::storage::RecordChange {
    fn from(value: RecordChange) -> Self {
        breez_sdk_common::sync::storage::RecordChange {
            id: value.id.into(),
            schema_version: value.schema_version,
            updated_fields: value.updated_fields,
            revision: value.revision,
        }
    }
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Clone, Debug)]
pub struct Record {
    pub id: RecordId,
    pub revision: u64,
    pub schema_version: String,
    pub data: HashMap<String, String>,
}

impl From<breez_sdk_common::sync::storage::Record> for Record {
    fn from(value: breez_sdk_common::sync::storage::Record) -> Self {
        Record {
            id: value.id.into(),
            revision: value.revision,
            schema_version: value.schema_version,
            data: value.data,
        }
    }
}

impl From<Record> for breez_sdk_common::sync::storage::Record {
    fn from(value: Record) -> Self {
        breez_sdk_common::sync::storage::Record {
            id: value.id.into(),
            revision: value.revision,
            schema_version: value.schema_version,
            data: value.data,
        }
    }
}
