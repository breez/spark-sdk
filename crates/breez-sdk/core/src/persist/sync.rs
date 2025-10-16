use std::{fmt::{Display, Formatter}, str::FromStr, sync::Arc};

use breez_sdk_common::sync::{OutgoingRecordRequest, RecordId, SyncService};
use semver::Version;

use crate::{DepositInfo, Payment, PaymentMetadata, Storage, StorageError, UpdateDepositPayload};

enum RecordType {
    PaymentMetadata
}

impl Display for RecordType {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        let s = match self {
            RecordType::PaymentMetadata => "PaymentMetadata",
        };
        write!(f, "{}", s)
    }
}

impl FromStr for RecordType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "PaymentMetadata" => Ok(RecordType::PaymentMetadata),
            _ => Err(format!("Unknown record type: {}", s)),
        }
    }
}

pub struct SyncedStorage {
    inner: Arc<dyn Storage>,
    schema_version: Version,
    sync_service: Arc<SyncService>,
}

impl SyncedStorage {
    pub fn new(inner: Arc<dyn Storage>, sync_service: Arc<SyncService>) -> Self {
        SyncedStorage { inner, sync_service, schema_version: "0.2.6".parse().expect("Invalid sync schema version") }
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
        self.sync_service.set_outgoing_record(&OutgoingRecordRequest {
            id: RecordId::new(RecordType::PaymentMetadata.to_string(), &payment_id),
            updated_fields: serde_json::to_value(&metadata).map_err(|e| StorageError::Implementation(e.to_string()))?,
        }).await.map_err(|e| StorageError::Implementation(e.to_string()))?;
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