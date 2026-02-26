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
use tracing::{Instrument, debug, error, warn};

use crate::{
    Contact, DepositInfo, EventEmitter, ListContactsRequest, Payment, PaymentDetails,
    PaymentMetadata, Storage, StorageError, UpdateDepositPayload,
    events::InternalSyncedEvent,
    persist::StorageListPaymentsRequest,
    sync_storage::{IncomingChange, OutgoingChange, Record, UnversionedRecordChange},
};
use serde::{Deserialize, Serialize};
use tokio_with_wasm::alias as tokio;

const INITIAL_SYNC_CACHE_KEY: &str = "sync_initial_complete";

pub(crate) enum RecordType {
    PaymentMetadata,
    Contact,
    LightningAddress,
}

impl RecordType {
    #[allow(clippy::match_same_arms)] // Arms will diverge as types evolve independently.
    pub(crate) const fn schema_version(&self) -> SchemaVersion {
        match self {
            Self::PaymentMetadata => SchemaVersion::new(1, 0, 0),
            Self::Contact => SchemaVersion::new(1, 0, 0),
            Self::LightningAddress => SchemaVersion::new(1, 0, 0),
        }
    }
}

impl Display for RecordType {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        let s = match self {
            RecordType::PaymentMetadata => "PaymentMetadata",
            RecordType::Contact => "Contact",
            RecordType::LightningAddress => "LightningAddress",
        };
        write!(f, "{s}")
    }
}

impl FromStr for RecordType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "PaymentMetadata" => Ok(RecordType::PaymentMetadata),
            "Contact" => Ok(RecordType::Contact),
            "LightningAddress" => Ok(RecordType::LightningAddress),
            _ => Err(format!("Unknown record type: {s}")),
        }
    }
}

pub(crate) const LIGHTNING_ADDRESS_DATA_ID: &str = "current";
const DELETED_AT_FIELD: &str = "deleted_at";

/// Internal sync model for contacts
#[derive(Serialize, Deserialize)]
struct ContactSyncData {
    pub id: String,
    pub name: String,
    pub payment_identifier: String,
    pub created_at: u64,
    pub updated_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    pub deleted_at: Option<u64>,
}

pub struct SyncedStorage {
    inner: Arc<dyn Storage>,
    sync_service: Arc<SyncService>,
    event_emitter: Arc<EventEmitter>,
    lightning_address_trigger: tokio::sync::broadcast::Sender<()>,
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

    async fn on_sync_failed(&self) {
        self.event_emitter.notify_rtsync_failed().await;
    }
}

impl SyncedStorage {
    pub fn new(
        inner: Arc<dyn Storage>,
        sync_service: Arc<SyncService>,
        event_emitter: Arc<EventEmitter>,
        lightning_address_trigger: tokio::sync::broadcast::Sender<()>,
    ) -> Self {
        SyncedStorage {
            inner,
            sync_service,
            event_emitter,
            lightning_address_trigger,
        }
    }

    pub fn initial_setup(self: &Arc<Self>) {
        let clone = Arc::clone(self);
        let span = tracing::Span::current();
        tokio::spawn(
            async move {
                if let Err(e) = clone.feed_existing_payment_metadata().await {
                    error!("Failed to feed existing payment metadata for sync: {}", e);
                }
            }
            .instrument(span),
        );
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
            .list_payments(StorageListPaymentsRequest::default())
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
                schema_version: RecordType::PaymentMetadata.schema_version(),
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
        let Ok(record_type) = RecordType::from_str(&change.new_state.id.r#type) else {
            warn!(
                "Deferring incoming record with unknown type '{}' at schema version {}",
                change.new_state.id.r#type, change.new_state.schema_version,
            );
            return Ok(RecordOutcome::Deferred);
        };

        // Domain-level applyability check: keep unsupported rows deferred for retry after upgrade.
        let type_version = record_type.schema_version();
        if !change
            .new_state
            .schema_version
            .is_supported_by(&type_version)
        {
            warn!(
                "Deferring incoming record type '{}' with unsupported schema version {} (supported up to major version {})",
                change.new_state.id.r#type, change.new_state.schema_version, type_version.major,
            );
            return Ok(RecordOutcome::Deferred);
        }

        match record_type {
            RecordType::PaymentMetadata => {
                self.handle_payment_metadata_update(
                    change.new_state.data,
                    change.new_state.id.data_id,
                )
                .await
            }
            RecordType::Contact => {
                self.handle_contact_change(change.new_state.data, change.new_state.id.data_id)
                    .await
            }
            RecordType::LightningAddress => {
                let _ = self.lightning_address_trigger.send(());
                Ok(())
            }
        }?;
        Ok(RecordOutcome::Completed)
    }

    /// Hook when an outgoing change is replayed, to ensure data consistency.
    async fn handle_outgoing_change(&self, change: CommonOutgoingChange) -> anyhow::Result<()> {
        let Ok(record_type) = RecordType::from_str(&change.change.id.r#type) else {
            error!(
                "Unknown record type '{}' with schema version {}",
                change.change.id.r#type, change.change.schema_version
            );
            return Ok(());
        };

        if change.change.schema_version.major > record_type.schema_version().major {
            warn!(
                "Skipping outgoing record '{}:{}': newer schema version {}",
                change.change.id.r#type, change.change.id.data_id, change.change.schema_version
            );
            return Ok(());
        }

        match record_type {
            RecordType::PaymentMetadata => {
                self.handle_payment_metadata_update(
                    change.change.updated_fields,
                    change.change.id.data_id,
                )
                .await
            }
            RecordType::Contact => {
                self.handle_contact_change(change.change.updated_fields, change.change.id.data_id)
                    .await
            }
            RecordType::LightningAddress => Ok(()),
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

    async fn handle_contact_change(
        &self,
        fields: HashMap<String, Value>,
        data_id: String,
    ) -> anyhow::Result<()> {
        if fields.contains_key(DELETED_AT_FIELD) {
            // Ignore not-found errors when deleting
            let _ = self.inner.delete_contact(data_id).await;
            return Ok(());
        }

        let sync_data: ContactSyncData = serde_json::from_value(
            serde_json::to_value(&fields)
                .map_err(|e| StorageError::Serialization(e.to_string()))?,
        )
        .map_err(|e| StorageError::Serialization(e.to_string()))?;

        let contact = Contact {
            id: data_id,
            name: sync_data.name,
            payment_identifier: sync_data.payment_identifier,
            created_at: sync_data.created_at,
            updated_at: sync_data.updated_at,
        };
        self.inner.insert_contact(contact).await?;

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
        request: StorageListPaymentsRequest,
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
                schema_version: RecordType::PaymentMetadata.schema_version(),
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

    async fn list_contacts(
        &self,
        request: ListContactsRequest,
    ) -> Result<Vec<Contact>, StorageError> {
        self.inner.list_contacts(request).await
    }

    async fn get_contact(&self, id: String) -> Result<Contact, StorageError> {
        self.inner.get_contact(id).await
    }

    async fn insert_contact(&self, contact: Contact) -> Result<(), StorageError> {
        let sync_data = ContactSyncData {
            id: contact.id.clone(),
            name: contact.name.clone(),
            payment_identifier: contact.payment_identifier.clone(),
            created_at: contact.created_at,
            updated_at: contact.updated_at,
            deleted_at: None,
        };
        self.sync_service
            .set_outgoing_record(&RecordChangeRequest {
                id: RecordId::new(RecordType::Contact.to_string(), &contact.id),
                schema_version: RecordType::Contact.schema_version(),
                updated_fields: serde_json::from_value(
                    serde_json::to_value(&sync_data)
                        .map_err(|e| StorageError::Serialization(e.to_string()))?,
                )
                .map_err(|e| StorageError::Serialization(e.to_string()))?,
            })
            .await
            .map_err(|e| StorageError::Implementation(e.to_string()))?;
        self.inner.insert_contact(contact).await
    }

    async fn delete_contact(&self, id: String) -> Result<(), StorageError> {
        let now = web_time::SystemTime::now()
            .duration_since(web_time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut updated_fields = HashMap::new();
        updated_fields.insert(DELETED_AT_FIELD.to_string(), serde_json::json!(now));
        self.sync_service
            .set_outgoing_record(&RecordChangeRequest {
                id: RecordId::new(RecordType::Contact.to_string(), &id),
                schema_version: RecordType::Contact.schema_version(),
                updated_fields,
            })
            .await
            .map_err(|e| StorageError::Implementation(e.to_string()))?;
        self.inner.delete_contact(id).await
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

#[cfg(all(test, not(all(target_family = "wasm", target_os = "unknown"))))]
mod tests {
    use super::*;
    use crate::{SparkHtlcDetails, persist::sqlite::SqliteStorage};
    use breez_sdk_common::sync::{
        Record as ModelRecord, RecordChange as ModelRecordChange, RecordId as ModelRecordId,
    };
    use std::path::PathBuf;

    fn create_temp_dir(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("breez-test-{}-{}", name, uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn create_test_synced_storage(storage: Arc<dyn Storage>) -> SyncedStorage {
        let sync_storage: Arc<dyn breez_sdk_common::sync::storage::SyncStorage> = Arc::new(
            crate::sync_storage::SyncStorageWrapper::new(Arc::clone(&storage)),
        );
        let sync_service = Arc::new(SyncService::new(sync_storage));
        let event_emitter = Arc::new(EventEmitter::new(true));
        let (la_trigger, _) = tokio::sync::broadcast::channel(16);
        SyncedStorage::new(storage, sync_service, event_emitter, la_trigger)
    }

    fn make_incoming_change(
        record_type: &str,
        data_id: &str,
        schema_version: SchemaVersion,
        data: HashMap<String, Value>,
    ) -> CommonIncomingChange {
        CommonIncomingChange {
            new_state: ModelRecord {
                id: ModelRecordId::new(record_type, data_id),
                revision: 1,
                schema_version,
                data,
            },
            old_state: None,
        }
    }

    fn make_outgoing_change(
        record_type: &str,
        data_id: &str,
        schema_version: SchemaVersion,
        updated_fields: HashMap<String, Value>,
    ) -> CommonOutgoingChange {
        CommonOutgoingChange {
            change: ModelRecordChange {
                id: ModelRecordId::new(record_type, data_id),
                schema_version,
                updated_fields,
                local_revision: 1,
            },
            parent: None,
        }
    }

    fn make_test_lightning_payment(id: &str) -> crate::Payment {
        crate::Payment {
            id: id.to_string(),
            payment_type: crate::PaymentType::Send,
            status: crate::PaymentStatus::Completed,
            amount: 100,
            fees: 0,
            timestamp: 1000,
            method: crate::PaymentMethod::Lightning,
            details: Some(crate::PaymentDetails::Lightning {
                invoice: "lnbc1test".to_string(),
                destination_pubkey: "02def456".to_string(),
                description: None,
                htlc_details: SparkHtlcDetails {
                    payment_hash: "abc123".to_string(),
                    preimage: None,
                    expiry_time: 0,
                    status: crate::SparkHtlcStatus::WaitingForPreimage,
                },
                lnurl_pay_info: None,
                lnurl_withdraw_info: None,
                lnurl_receive_metadata: None,
            }),
            conversion_details: None,
        }
    }

    #[tokio::test]
    async fn test_incoming_unknown_type_newer_schema() {
        let temp_dir = create_temp_dir("incoming_unknown_newer");
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&temp_dir).unwrap());
        let synced = create_test_synced_storage(Arc::clone(&storage));

        let change = make_incoming_change(
            "FutureType",
            "id1",
            SchemaVersion::new(99, 0, 0),
            HashMap::new(),
        );
        let result = synced.handle_incoming_change(change).await;
        assert!(result.is_ok());

        // Verify no payment was created as a side effect
        assert!(storage.get_payment_by_id("id1".to_string()).await.is_err());
    }

    #[tokio::test]
    async fn test_incoming_unknown_type_compatible_schema() {
        let temp_dir = create_temp_dir("incoming_unknown_compat");
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&temp_dir).unwrap());
        let synced = create_test_synced_storage(Arc::clone(&storage));

        let change = make_incoming_change(
            "UnknownType",
            "id1",
            SchemaVersion::new(1, 0, 0),
            HashMap::new(),
        );
        let result = synced.handle_incoming_change(change).await;
        assert!(result.is_ok());

        // Verify no payment metadata was written
        assert!(storage.get_payment_by_id("id1".to_string()).await.is_err());
    }

    #[tokio::test]
    async fn test_incoming_known_type_newer_major_version() {
        let temp_dir = create_temp_dir("incoming_known_newer_major");
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&temp_dir).unwrap());
        let synced = create_test_synced_storage(Arc::clone(&storage));

        // Insert a payment so we can verify metadata was NOT written
        storage
            .insert_payment(make_test_lightning_payment("id1"))
            .await
            .unwrap();

        let mut data = HashMap::new();
        data.insert(
            "lnurl_pay_info".to_string(),
            serde_json::json!({"ln_address": "test@example.com"}),
        );
        let pm_version = RecordType::PaymentMetadata.schema_version();
        let change = make_incoming_change(
            "PaymentMetadata",
            "id1",
            SchemaVersion::new(pm_version.major + 1, 0, 0),
            data,
        );
        let result = synced.handle_incoming_change(change).await;
        assert!(result.is_ok());

        // Verify metadata was NOT applied despite known type (newer major version)
        let payment = storage.get_payment_by_id("id1".to_string()).await.unwrap();
        if let Some(crate::PaymentDetails::Lightning { lnurl_pay_info, .. }) = &payment.details {
            assert!(
                lnurl_pay_info.is_none(),
                "lnurl_pay_info should not be set for newer major version"
            );
        } else {
            panic!("Expected Lightning payment details");
        }
    }

    #[tokio::test]
    async fn test_incoming_known_type_newer_minor_version_applied() {
        let temp_dir = create_temp_dir("incoming_known_newer_minor");
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&temp_dir).unwrap());
        let synced = create_test_synced_storage(Arc::clone(&storage));

        storage
            .insert_payment(make_test_lightning_payment("pay1"))
            .await
            .unwrap();

        let mut data = HashMap::new();
        data.insert(
            "lnurl_pay_info".to_string(),
            serde_json::json!({"ln_address": "test@example.com"}),
        );
        let pm_version = RecordType::PaymentMetadata.schema_version();
        let change = make_incoming_change(
            "PaymentMetadata",
            "pay1",
            SchemaVersion::new(pm_version.major, pm_version.minor + 1, 0),
            data,
        );
        let result = synced.handle_incoming_change(change).await;
        assert!(result.is_ok());

        // Verify metadata WAS applied (compatible minor version bump)
        let payment = storage.get_payment_by_id("pay1".to_string()).await.unwrap();
        if let Some(crate::PaymentDetails::Lightning { lnurl_pay_info, .. }) = &payment.details {
            let info = lnurl_pay_info
                .as_ref()
                .expect("lnurl_pay_info should be set");
            assert_eq!(info.ln_address.as_deref(), Some("test@example.com"));
        } else {
            panic!("Expected Lightning payment details");
        }
    }

    #[tokio::test]
    async fn test_outgoing_unknown_type_newer_schema() {
        let temp_dir = create_temp_dir("outgoing_unknown_newer");
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&temp_dir).unwrap());
        let synced = create_test_synced_storage(Arc::clone(&storage));

        let change = make_outgoing_change(
            "FutureType",
            "id1",
            SchemaVersion::new(99, 0, 0),
            HashMap::new(),
        );
        let result = synced.handle_outgoing_change(change).await;
        assert!(result.is_ok());

        // Verify no payment metadata side effect
        assert!(storage.get_payment_by_id("id1".to_string()).await.is_err());
    }

    #[tokio::test]
    async fn test_outgoing_unknown_type_compatible_schema() {
        let temp_dir = create_temp_dir("outgoing_unknown_compat");
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&temp_dir).unwrap());
        let synced = create_test_synced_storage(Arc::clone(&storage));

        let change = make_outgoing_change(
            "UnknownType",
            "id1",
            SchemaVersion::new(1, 0, 0),
            HashMap::new(),
        );
        let result = synced.handle_outgoing_change(change).await;
        assert!(result.is_ok());

        // Verify no payment metadata side effect
        assert!(storage.get_payment_by_id("id1".to_string()).await.is_err());
    }

    fn make_contact_data(name: &str, payment_id: &str) -> HashMap<String, Value> {
        let mut data = HashMap::new();
        data.insert("id".to_string(), serde_json::json!("c1"));
        data.insert("name".to_string(), serde_json::json!(name));
        data.insert(
            "payment_identifier".to_string(),
            serde_json::json!(payment_id),
        );
        data.insert("created_at".to_string(), serde_json::json!(1000));
        data.insert("updated_at".to_string(), serde_json::json!(1000));
        data
    }

    #[tokio::test]
    async fn test_incoming_contact_without_deleted_at_upserts() {
        let temp_dir = create_temp_dir("incoming_contact_upsert");
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&temp_dir).unwrap());
        let synced = create_test_synced_storage(Arc::clone(&storage));

        let data = make_contact_data("Alice", "alice@example.com");
        let change =
            make_incoming_change("Contact", "c1", RecordType::Contact.schema_version(), data);
        let result = synced.handle_incoming_change(change).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), RecordOutcome::Completed);

        let contact = storage.get_contact("c1".to_string()).await.unwrap();
        assert_eq!(contact.name, "Alice");
        assert_eq!(contact.payment_identifier, "alice@example.com");
    }

    #[tokio::test]
    async fn test_incoming_contact_with_deleted_at_deletes() {
        let temp_dir = create_temp_dir("incoming_contact_delete");
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&temp_dir).unwrap());
        let synced = create_test_synced_storage(Arc::clone(&storage));

        // First insert a contact
        storage
            .insert_contact(Contact {
                id: "c1".to_string(),
                name: "Alice".to_string(),
                payment_identifier: "alice@example.com".to_string(),
                created_at: 1000,
                updated_at: 1000,
            })
            .await
            .unwrap();

        // Incoming change with deleted_at should delete the contact
        let mut data = make_contact_data("Alice", "alice@example.com");
        data.insert("deleted_at".to_string(), serde_json::json!(2000));
        let change =
            make_incoming_change("Contact", "c1", RecordType::Contact.schema_version(), data);
        let result = synced.handle_incoming_change(change).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), RecordOutcome::Completed);

        assert!(storage.get_contact("c1".to_string()).await.is_err());
    }

    #[tokio::test]
    async fn test_outgoing_replay_contact_without_deleted_at_upserts() {
        let temp_dir = create_temp_dir("outgoing_contact_upsert");
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&temp_dir).unwrap());
        let synced = create_test_synced_storage(Arc::clone(&storage));

        let data = make_contact_data("Bob", "bob@example.com");
        let change =
            make_outgoing_change("Contact", "c2", RecordType::Contact.schema_version(), data);
        let result = synced.handle_outgoing_change(change).await;
        assert!(result.is_ok());

        let contact = storage.get_contact("c2".to_string()).await.unwrap();
        assert_eq!(contact.name, "Bob");
    }

    #[tokio::test]
    async fn test_outgoing_replay_contact_with_deleted_at_deletes() {
        let temp_dir = create_temp_dir("outgoing_contact_delete");
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&temp_dir).unwrap());
        let synced = create_test_synced_storage(Arc::clone(&storage));

        // First insert a contact
        storage
            .insert_contact(Contact {
                id: "c3".to_string(),
                name: "Charlie".to_string(),
                payment_identifier: "charlie@example.com".to_string(),
                created_at: 1000,
                updated_at: 1000,
            })
            .await
            .unwrap();

        // Outgoing replay with deleted_at should delete the contact
        let mut data = HashMap::new();
        data.insert("deleted_at".to_string(), serde_json::json!(2000));
        let change =
            make_outgoing_change("Contact", "c3", RecordType::Contact.schema_version(), data);
        let result = synced.handle_outgoing_change(change).await;
        assert!(result.is_ok());

        assert!(storage.get_contact("c3".to_string()).await.is_err());
    }
}
