use std::{
    collections::HashMap,
    fmt::{Display, Formatter},
    str::FromStr,
    sync::Arc,
};

use breez_sdk_common::sync::{
    IncomingChange, NewRecordHandler, OutgoingChange, RecordChangeRequest, RecordId, SyncService,
};
use serde_json::Value;
use tracing::{debug, error};

use crate::{
    DepositInfo, EventEmitter, ListPaymentsRequest, Payment, PaymentDetails, PaymentMetadata,
    SdkEvent, Storage, StorageError, UpdateDepositPayload,
};
use tokio_with_wasm::alias as tokio;

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
    async fn on_incoming_change(&self, change: IncomingChange) -> anyhow::Result<()> {
        self.handle_incoming_change(change).await
    }

    async fn on_replay_outgoing_change(&self, change: OutgoingChange) -> anyhow::Result<()> {
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
        let did_pull_new_records = incoming_count.is_some();
        self.event_emitter
            .emit(&SdkEvent::DataSynced {
                did_pull_new_records,
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
            let PaymentDetails::Lightning {
                description,
                lnurl_pay_info,
                ..
            } = details
            else {
                continue;
            };
            let Some(lnurl_pay_info) = lnurl_pay_info else {
                continue;
            };
            let metadata = PaymentMetadata {
                lnurl_description: description,
                lnurl_pay_info: Some(lnurl_pay_info),
            };
            let record_id = RecordId::new(RecordType::PaymentMetadata.to_string(), &payment.id);
            let record_change_request = RecordChangeRequest {
                id: record_id,
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

    async fn handle_incoming_change(&self, change: IncomingChange) -> anyhow::Result<()> {
        let record_type =
            RecordType::from_str(&change.new_state.id.r#type).map_err(|e| anyhow::anyhow!(e))?;
        match record_type {
            RecordType::PaymentMetadata => {
                self.handle_payment_metadata_update(
                    change.new_state.data,
                    change.new_state.id.data_id,
                )
                .await
            }
        }
    }

    /// Hook when an outgoing change is replayed, to ensure data consistency.
    async fn handle_outgoing_change(&self, change: OutgoingChange) -> anyhow::Result<()> {
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

        self.inner.set_payment_metadata(data_id, metadata).await?;
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
}
