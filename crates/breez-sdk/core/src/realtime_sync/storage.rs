use std::{
    collections::HashMap,
    fmt::{Display, Formatter},
    str::FromStr,
    sync::Arc,
};

use breez_sdk_common::sync::{
    IncomingChange as CommonIncomingChange, NewRecordHandler,
    OutgoingChange as CommonOutgoingChange, RecordChangeRequest, RecordId, RecordOutcome,
    SchemaVersion, SyncService,
};
use serde_json::Value;
use tracing::{debug, error, warn};

use crate::{
    DepositInfo, EventEmitter, ListPaymentsRequest, Payment, PaymentDetails, PaymentMetadata,
    Storage, StorageError, UpdateDepositPayload,
    events::InternalSyncedEvent,
    sync_storage::{IncomingChange, OutgoingChange, Record, UnversionedRecordChange},
};
use tokio_with_wasm::alias as tokio;

const CURRENT_SCHEMA_VERSION: SchemaVersion = SchemaVersion::new(1, 0, 0);

const INITIAL_SYNC_CACHE_KEY: &str = "sync_initial_complete";

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
    event_emitter: Arc<EventEmitter>,
}

#[macros::async_trait]
impl NewRecordHandler for SyncedStorage {
    async fn on_incoming_change(
        &self,
        change: CommonIncomingChange,
    ) -> anyhow::Result<RecordOutcome> {
        self.handle_incoming_change(change).await
    }

    async fn on_replay_outgoing_change(&self, change: CommonOutgoingChange) -> anyhow::Result<()> {
        self.handle_outgoing_change(change).await
    }

    async fn on_sync_completed(
        &self,
        incoming_count: Option<u32>,
        outgoing_count: Option<u32>,
    ) -> anyhow::Result<()> {
        debug!(
            "real-time sync completed for {:?} incoming, {:?} outgoing records",
            incoming_count, outgoing_count
        );

        // No need to emit an event if no pull was done.
        if incoming_count.is_none() {
            return Ok(());
        }

        self.event_emitter
            .emit_synced(&InternalSyncedEvent {
                storage_incoming: incoming_count,
                ..Default::default()
            })
            .await;
        Ok(())
    }
}

impl SyncedStorage {
    pub fn new(
        inner: Arc<dyn Storage>,
        sync_service: Arc<SyncService>,
        event_emitter: Arc<EventEmitter>,
    ) -> Self {
        SyncedStorage {
            inner,
            sync_service,
            event_emitter,
        }
    }

    pub fn initial_setup(self: &Arc<Self>) {
        let clone = Arc::clone(self);
        tokio::spawn(async move {
            if let Err(e) = clone.feed_existing_payment_metadata().await {
                error!("Failed to feed existing payment metadata for sync: {}", e);
            }
        });
    }

    /// Feed existing payment metadata into sync storage. This is really only needed the first time sync is set up,
    /// but there doesn't seem to be a good way to detect that, so we just do it every time.
    async fn feed_existing_payment_metadata(&self) -> anyhow::Result<()> {
        if self
            .get_cached_item(INITIAL_SYNC_CACHE_KEY.to_string())
            .await?
            .is_some()
        {
            return Ok(());
        }

        let payments = self
            .inner
            .list_payments(ListPaymentsRequest::default())
            .await?;
        for payment in payments {
            let Some(details) = payment.details else {
                continue;
            };
            let (description, lnurl_pay_info, lnurl_withdraw_info, conversion_info) = match details
            {
                PaymentDetails::Lightning {
                    description,
                    lnurl_pay_info,
                    lnurl_withdraw_info,
                    ..
                } => (description, lnurl_pay_info, lnurl_withdraw_info, None),
                PaymentDetails::Spark {
                    conversion_info, ..
                }
                | PaymentDetails::Token {
                    conversion_info, ..
                } => (None, None, None, conversion_info),
                _ => continue,
            };

            if lnurl_pay_info.is_none()
                && lnurl_withdraw_info.is_none()
                && conversion_info.is_none()
            {
                continue;
            }

            let metadata = PaymentMetadata {
                lnurl_description: description,
                lnurl_pay_info,
                lnurl_withdraw_info,
                conversion_info,
                ..Default::default()
            };
            let record_id = RecordId::new(RecordType::PaymentMetadata.to_string(), &payment.id);
            let record_change_request = RecordChangeRequest {
                id: record_id,
                schema_version: CURRENT_SCHEMA_VERSION,
                updated_fields: serde_json::from_value(
                    serde_json::to_value(&metadata)
                        .map_err(|e| StorageError::Serialization(e.to_string()))?,
                )
                .map_err(|e| StorageError::Serialization(e.to_string()))?,
            };
            self.sync_service
                .set_outgoing_record(&record_change_request)
                .await?;
        }

        self.set_cached_item(INITIAL_SYNC_CACHE_KEY.to_string(), "true".to_string())
            .await?;
        Ok(())
    }

    async fn handle_incoming_change(
        &self,
        change: CommonIncomingChange,
    ) -> anyhow::Result<RecordOutcome> {
        // Domain-level applyability check: keep unsupported rows deferred for retry after upgrade.
        if !change
            .new_state
            .schema_version
            .is_supported_by(&CURRENT_SCHEMA_VERSION)
        {
            warn!(
                "Deferring incoming record type '{}' with unsupported schema version {} (supported up to major version {})",
                change.new_state.id.r#type,
                change.new_state.schema_version,
                CURRENT_SCHEMA_VERSION.major,
            );
            return Ok(RecordOutcome::Deferred);
        }

        let Ok(record_type) = RecordType::from_str(&change.new_state.id.r#type) else {
            warn!(
                "Deferring incoming record with unknown type '{}' at schema version {}",
                change.new_state.id.r#type, change.new_state.schema_version,
            );
            return Ok(RecordOutcome::Deferred);
        };
        match record_type {
            RecordType::PaymentMetadata => {
                self.handle_payment_metadata_update(
                    change.new_state.data,
                    change.new_state.id.data_id,
                )
                .await?;
                Ok(RecordOutcome::Completed)
            }
        }
    }

    /// Hook when an outgoing change is replayed, to ensure data consistency.
    async fn handle_outgoing_change(&self, change: CommonOutgoingChange) -> anyhow::Result<()> {
        let record_type =
            RecordType::from_str(&change.change.id.r#type).map_err(|e| anyhow::anyhow!(e))?;
        match record_type {
            RecordType::PaymentMetadata => {
                self.handle_payment_metadata_update(
                    change.change.updated_fields,
                    change.change.id.data_id,
                )
                .await
            }
        }
    }

    async fn handle_payment_metadata_update(
        &self,
        updated_fields: HashMap<String, Value>,
        data_id: String,
    ) -> anyhow::Result<()> {
        let metadata: PaymentMetadata = serde_json::from_value(
            serde_json::to_value(&updated_fields)
                .map_err(|e| StorageError::Serialization(e.to_string()))?,
        )
        .map_err(|e| StorageError::Serialization(e.to_string()))?;

        self.inner
            .insert_payment_metadata(data_id, metadata)
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
        request: ListPaymentsRequest,
    ) -> Result<Vec<Payment>, StorageError> {
        self.inner.list_payments(request).await
    }

    async fn insert_payment(&self, payment: Payment) -> Result<(), StorageError> {
        self.inner.insert_payment(payment).await
    }

    async fn insert_payment_metadata(
        &self,
        payment_id: String,
        metadata: PaymentMetadata,
    ) -> Result<(), StorageError> {
        // Set the outgoing record for sync before updating local storage.
        self.sync_service
            .set_outgoing_record(&RecordChangeRequest {
                id: RecordId::new(RecordType::PaymentMetadata.to_string(), &payment_id),
                schema_version: CURRENT_SCHEMA_VERSION,
                updated_fields: serde_json::from_value(
                    serde_json::to_value(&metadata)
                        .map_err(|e| StorageError::Serialization(e.to_string()))?,
                )
                .map_err(|e| StorageError::Serialization(e.to_string()))?,
            })
            .await
            .map_err(|e| StorageError::Implementation(e.to_string()))?;
        self.inner
            .insert_payment_metadata(payment_id, metadata)
            .await
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

    async fn get_payments_by_parent_ids(
        &self,
        parent_payment_ids: Vec<String>,
    ) -> Result<HashMap<String, Vec<Payment>>, StorageError> {
        self.inner
            .get_payments_by_parent_ids(parent_payment_ids)
            .await
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

    async fn set_lnurl_metadata(
        &self,
        metadata: Vec<crate::persist::SetLnurlMetadataItem>,
    ) -> Result<(), StorageError> {
        self.inner.set_lnurl_metadata(metadata).await
    }

    async fn add_outgoing_change(
        &self,
        record: UnversionedRecordChange,
    ) -> Result<u64, StorageError> {
        self.inner.add_outgoing_change(record).await
    }

    async fn complete_outgoing_sync(
        &self,
        record: Record,
        local_revision: u64,
    ) -> Result<(), StorageError> {
        self.inner
            .complete_outgoing_sync(record, local_revision)
            .await
    }

    async fn get_pending_outgoing_changes(
        &self,
        limit: u32,
    ) -> Result<Vec<OutgoingChange>, StorageError> {
        self.inner.get_pending_outgoing_changes(limit).await
    }

    async fn get_last_revision(&self) -> Result<u64, StorageError> {
        self.inner.get_last_revision().await
    }

    async fn insert_incoming_records(&self, records: Vec<Record>) -> Result<(), StorageError> {
        self.inner.insert_incoming_records(records).await
    }

    async fn delete_incoming_record(&self, record: Record) -> Result<(), StorageError> {
        self.inner.delete_incoming_record(record).await
    }

    async fn get_incoming_records(&self, limit: u32) -> Result<Vec<IncomingChange>, StorageError> {
        self.inner.get_incoming_records(limit).await
    }

    async fn get_latest_outgoing_change(&self) -> Result<Option<OutgoingChange>, StorageError> {
        self.inner.get_latest_outgoing_change().await
    }

    async fn update_record_from_incoming(&self, record: Record) -> Result<(), StorageError> {
        self.inner.update_record_from_incoming(record).await
    }
}
