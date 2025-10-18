use std::{
    fmt::{Display, Formatter},
    str::FromStr,
    sync::Arc,
};

use breez_sdk_common::sync::model::{RecordChangeRequest, RecordChangeSet, RecordId};

use crate::{
    persist::Record,
    sync::{CallbackReceiver, SyncService},
};

use crate::{DepositInfo, Payment, PaymentMetadata, Storage, StorageError, UpdateDepositPayload};
use tokio_with_wasm::alias as tokio;

enum RecordType {
    PaymentMetadata,
}

impl Display for RecordType {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        let s = match self {
            RecordType::PaymentMetadata => "PaymentMetadata",
        };
        write!(f, "{s}")
    }
}

impl FromStr for RecordType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "PaymentMetadata" => Ok(RecordType::PaymentMetadata),
            _ => Err(format!("Unknown record type: {s}")),
        }
    }
}

pub struct SyncedStorage {
    inner: Arc<dyn Storage>,
    sync_service: Arc<SyncService>,
}

impl SyncedStorage {
    pub fn new(inner: Arc<dyn Storage>, sync_service: Arc<SyncService>) -> Self {
        SyncedStorage {
            inner,
            sync_service,
        }
    }

    pub fn listen(
        self: &Arc<Self>,
        incoming_callback: CallbackReceiver<RecordChangeSet>,
        outgoing_callback: CallbackReceiver<RecordChangeSet>,
    ) {
        let clone = Arc::clone(self);
        tokio::spawn(async move {
            clone
                .listen_inner(incoming_callback, outgoing_callback)
                .await;
        });
    }

    async fn listen_inner(
        self: Arc<Self>,
        mut incoming_callback: CallbackReceiver<RecordChangeSet>,
        mut outgoing_callback: CallbackReceiver<RecordChangeSet>,
    ) {
        loop {
            tokio::select! {
                incoming = incoming_callback.recv() => {
                    let Some(callback) = incoming else {
                        break;
                    };
                    let result = self.handle_incoming_change(callback.args).await;
                    let _ = callback.responder.send(result);
                }

                outgoing = outgoing_callback.recv() => {
                    let Some(callback) = outgoing else {
                        break;
                    };
                    let result = self.handle_outgoing_change(callback.args).await;
                    let _ = callback.responder.send(result);
                }
            }
        }
    }

    async fn handle_incoming_change(&self, change: RecordChangeSet) -> anyhow::Result<()> {
        // Incoming and outgoing records are handled the same way at this level.
        self.handle_change(change).await
    }

    async fn handle_outgoing_change(&self, change: RecordChangeSet) -> anyhow::Result<()> {
        // Incoming and outgoing records are handled the same way at this level.
        self.handle_change(change).await
    }

    async fn handle_change(&self, change: RecordChangeSet) -> anyhow::Result<()> {
        let record_type =
            RecordType::from_str(&change.change.id.r#type).map_err(|e| anyhow::anyhow!(e))?;
        match record_type {
            RecordType::PaymentMetadata => {
                self.handle_payment_metadata_update(change).await?;
            }
        }
        Ok(())
    }

    async fn handle_payment_metadata_update(&self, change: RecordChangeSet) -> anyhow::Result<()> {
        let metadata: PaymentMetadata = serde_json::from_value(
            serde_json::to_value(&change.change.updated_fields)
                .map_err(|e| StorageError::Serialization(e.to_string()))?,
        )
        .map_err(|e| StorageError::Serialization(e.to_string()))?;

        self.inner
            .set_payment_metadata(change.change.id.data_id, metadata)
            .await?;
        Ok(())
    }
}

#[macros::async_trait]
impl Storage for SyncedStorage {
    async fn delete_cached_item(&self, key: String) -> Result<(), StorageError> {
        self.inner.delete_cached_item(key).await
    }
    async fn get_cached_item(&self, key: String) -> Result<Option<String>, StorageError> {
        self.inner.get_cached_item(key).await
    }
    async fn set_cached_item(&self, key: String, value: String) -> Result<(), StorageError> {
        self.inner.set_cached_item(key, value).await
    }
    async fn list_payments(
        &self,
        offset: Option<u32>,
        limit: Option<u32>,
    ) -> Result<Vec<Payment>, StorageError> {
        self.inner.list_payments(offset, limit).await
    }

    async fn insert_payment(&self, payment: Payment) -> Result<(), StorageError> {
        self.inner.insert_payment(payment).await
    }

    async fn set_payment_metadata(
        &self,
        payment_id: String,
        metadata: PaymentMetadata,
    ) -> Result<(), StorageError> {
        // Set the outgoing record for sync before updating local storage.
        self.sync_service
            .set_outgoing_record(&RecordChangeRequest {
                id: RecordId::new(RecordType::PaymentMetadata.to_string(), &payment_id),
                updated_fields: serde_json::from_value(
                    serde_json::to_value(&metadata)
                        .map_err(|e| StorageError::Serialization(e.to_string()))?,
                )
                .map_err(|e| StorageError::Serialization(e.to_string()))?,
            })
            .await
            .map_err(|e| StorageError::Implementation(e.to_string()))?;
        self.inner.set_payment_metadata(payment_id, metadata).await
    }

    async fn get_payment_by_id(&self, id: String) -> Result<Payment, StorageError> {
        self.inner.get_payment_by_id(id).await
    }

    async fn get_payment_by_invoice(
        &self,
        invoice: String,
    ) -> Result<Option<Payment>, StorageError> {
        self.inner.get_payment_by_invoice(invoice).await
    }

    async fn add_deposit(
        &self,
        txid: String,
        vout: u32,
        amount_sats: u64,
    ) -> Result<(), StorageError> {
        self.inner.add_deposit(txid, vout, amount_sats).await
    }

    async fn delete_deposit(&self, txid: String, vout: u32) -> Result<(), StorageError> {
        self.inner.delete_deposit(txid, vout).await
    }

    async fn list_deposits(&self) -> Result<Vec<DepositInfo>, StorageError> {
        self.inner.list_deposits().await
    }

    async fn update_deposit(
        &self,
        txid: String,
        vout: u32,
        payload: UpdateDepositPayload,
    ) -> Result<(), StorageError> {
        self.inner.update_deposit(txid, vout, payload).await
    }

    async fn sync_add_outgoing_change(
        &self,
        record: crate::persist::UnversionedRecordChange,
    ) -> Result<u64, StorageError> {
        self.inner.sync_add_outgoing_change(record).await
    }
    async fn sync_complete_outgoing_sync(
        &self,
        record: crate::persist::Record,
    ) -> Result<(), StorageError> {
        self.inner.sync_complete_outgoing_sync(record).await
    }
    async fn sync_get_pending_outgoing_changes(
        &self,
        limit: u32,
    ) -> Result<Vec<crate::persist::RecordChangeSet>, StorageError> {
        self.inner.sync_get_pending_outgoing_changes(limit).await
    }

    /// Get the revision number of the last synchronized record
    async fn sync_get_last_revision(&self) -> Result<u64, StorageError> {
        self.inner.sync_get_last_revision().await
    }

    /// Insert incoming records from remote sync
    async fn sync_insert_incoming_records(&self, records: Vec<Record>) -> Result<(), StorageError> {
        self.inner.sync_insert_incoming_records(records).await
    }

    /// Delete an incoming record after it has been processed
    async fn sync_delete_incoming_record(&self, record: Record) -> Result<(), StorageError> {
        self.inner.sync_delete_incoming_record(record).await
    }

    /// Update revision numbers of pending outgoing records to be higher than the given revision
    async fn sync_rebase_pending_outgoing_records(
        &self,
        revision: u64,
    ) -> Result<(), StorageError> {
        self.inner
            .sync_rebase_pending_outgoing_records(revision)
            .await
    }

    /// Get incoming records that need to be processed, up to the specified limit
    async fn sync_get_incoming_records(
        &self,
        limit: u32,
    ) -> Result<Vec<crate::persist::RecordContext>, StorageError> {
        self.inner.sync_get_incoming_records(limit).await
    }

    /// Get the latest outgoing record if any exists
    async fn sync_get_latest_outgoing_change(
        &self,
    ) -> Result<Option<crate::persist::RecordChangeSet>, StorageError> {
        self.inner.sync_get_latest_outgoing_change().await
    }

    /// Update the sync state record from an incoming record
    async fn sync_update_record_from_incoming(&self, record: Record) -> Result<(), StorageError> {
        self.inner.sync_update_record_from_incoming(record).await
    }
}
