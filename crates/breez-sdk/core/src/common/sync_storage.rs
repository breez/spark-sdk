use std::{collections::HashMap, sync::Arc};

use serde::{Deserialize, Serialize};

use crate::persist::{Storage, StorageError};

pub(crate) struct SyncStorageWrapper {
    pub inner: Arc<dyn Storage>,
}

impl SyncStorageWrapper {
    pub fn new(inner: Arc<dyn Storage>) -> Self {
        Self { inner }
    }
}

fn storage_to_sync_error(value: StorageError) -> breez_sdk_common::sync::storage::SyncStorageError {
    match value {
        StorageError::Connection(msg) | StorageError::Implementation(msg) => {
            breez_sdk_common::sync::storage::SyncStorageError::Implementation(msg)
        }
        StorageError::InitializationError(msg) => {
            breez_sdk_common::sync::storage::SyncStorageError::InitializationError(msg)
        }
        StorageError::Serialization(msg) => {
            breez_sdk_common::sync::storage::SyncStorageError::Serialization(msg)
        }
    }
}

#[macros::async_trait]
impl breez_sdk_common::sync::storage::SyncStorage for SyncStorageWrapper {
    async fn add_outgoing_change(
        &self,
        record: breez_sdk_common::sync::storage::UnversionedRecordChange,
    ) -> Result<u64, breez_sdk_common::sync::storage::SyncStorageError> {
        self.inner
            .add_outgoing_change(record.into())
            .await
            .map_err(storage_to_sync_error)
    }

    async fn complete_outgoing_sync(
        &self,
        record: breez_sdk_common::sync::storage::Record,
        local_revision: u64,
    ) -> Result<(), breez_sdk_common::sync::storage::SyncStorageError> {
        self.inner
            .complete_outgoing_sync(record.into(), local_revision)
            .await
            .map_err(storage_to_sync_error)
    }

    async fn get_pending_outgoing_changes(
        &self,
        limit: u32,
    ) -> Result<
        Vec<breez_sdk_common::sync::storage::OutgoingChange>,
        breez_sdk_common::sync::storage::SyncStorageError,
    > {
        let changes = self
            .inner
            .get_pending_outgoing_changes(limit)
            .await
            .map_err(storage_to_sync_error)?;
        Ok(changes.into_iter().map(From::from).collect())
    }

    async fn get_last_revision(
        &self,
    ) -> Result<u64, breez_sdk_common::sync::storage::SyncStorageError> {
        self.inner
            .get_last_revision()
            .await
            .map_err(storage_to_sync_error)
    }

    async fn insert_incoming_records(
        &self,
        records: Vec<breez_sdk_common::sync::storage::Record>,
    ) -> Result<(), breez_sdk_common::sync::storage::SyncStorageError> {
        let recs: Vec<Record> = records.into_iter().map(From::from).collect();
        self.inner
            .insert_incoming_records(recs)
            .await
            .map_err(storage_to_sync_error)
    }

    async fn delete_incoming_record(
        &self,
        record: breez_sdk_common::sync::storage::Record,
    ) -> Result<(), breez_sdk_common::sync::storage::SyncStorageError> {
        self.inner
            .delete_incoming_record(record.into())
            .await
            .map_err(storage_to_sync_error)
    }

    async fn get_incoming_records(
        &self,
        limit: u32,
    ) -> Result<
        Vec<breez_sdk_common::sync::storage::IncomingChange>,
        breez_sdk_common::sync::storage::SyncStorageError,
    > {
        let changes = self
            .inner
            .get_incoming_records(limit)
            .await
            .map_err(storage_to_sync_error)?;
        Ok(changes.into_iter().map(From::from).collect())
    }

    async fn get_latest_outgoing_change(
        &self,
    ) -> Result<
        Option<breez_sdk_common::sync::storage::OutgoingChange>,
        breez_sdk_common::sync::storage::SyncStorageError,
    > {
        let change = self
            .inner
            .get_latest_outgoing_change()
            .await
            .map_err(storage_to_sync_error)?;
        Ok(change.map(From::from))
    }

    async fn update_record_from_incoming(
        &self,
        record: breez_sdk_common::sync::storage::Record,
    ) -> Result<(), breez_sdk_common::sync::storage::SyncStorageError> {
        self.inner
            .update_record_from_incoming(record.into())
            .await
            .map_err(storage_to_sync_error)
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
    pub local_revision: u64,
}

impl From<breez_sdk_common::sync::storage::RecordChange> for RecordChange {
    fn from(value: breez_sdk_common::sync::storage::RecordChange) -> Self {
        RecordChange {
            id: value.id.into(),
            schema_version: value.schema_version,
            updated_fields: value.updated_fields,
            local_revision: value.local_revision,
        }
    }
}

impl From<RecordChange> for breez_sdk_common::sync::storage::RecordChange {
    fn from(value: RecordChange) -> Self {
        breez_sdk_common::sync::storage::RecordChange {
            id: value.id.into(),
            schema_version: value.schema_version,
            updated_fields: value.updated_fields,
            local_revision: value.local_revision,
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
