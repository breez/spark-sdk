use std::{
    collections::HashMap,
    fmt::{Display, Formatter},
    str::FromStr,
    sync::Arc,
};

use breez_sdk_common::sync::{
    IncomingChange as CommonIncomingChange, NewRecordHandler,
    OutgoingChange as CommonOutgoingChange, RecordChangeRequest, RecordId, SchemaVersion,
    SyncService,
};

const CURRENT_SCHEMA_VERSION: SchemaVersion = SchemaVersion::new(1, 0, 0);
use serde_json::Value;
use tracing::{debug, error, warn};

use crate::{
    DepositInfo, EventEmitter, ListPaymentsRequest, Payment, PaymentDetails, PaymentMetadata,
    Storage, StorageError, UpdateDepositPayload,
    events::InternalSyncedEvent,
    sync_storage::{IncomingChange, OutgoingChange, Record, UnversionedRecordChange},
};
use tokio_with_wasm::alias as tokio;

const INITIAL_SYNC_CACHE_KEY: &str = "sync_initial_complete";
const RECOVERY_APPLIED_SCHEMA_VERSION_KEY: &str = "recovery_applied_schema_version";

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
    async fn on_incoming_change(&self, change: CommonIncomingChange) -> anyhow::Result<()> {
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
            clone.try_apply_new_sync_state_records().await;
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

    async fn handle_incoming_change(&self, change: CommonIncomingChange) -> anyhow::Result<()> {
        if change.new_state.schema_version.major > CURRENT_SCHEMA_VERSION.major {
            warn!(
                "Skipping incoming record '{}:{}': newer schema version {}",
                change.new_state.id.r#type,
                change.new_state.id.data_id,
                change.new_state.schema_version
            );
            return Ok(());
        }

        let Ok(record_type) = RecordType::from_str(&change.new_state.id.r#type) else {
            error!(
                "Unknown record type '{}' with compatible schema version {}",
                change.new_state.id.r#type, change.new_state.schema_version
            );
            return Ok(());
        };

        let RecordType::PaymentMetadata = record_type;
        self.handle_payment_metadata_update(change.new_state.data, change.new_state.id.data_id)
            .await
    }

    /// Hook when an outgoing change is replayed, to ensure data consistency.
    async fn handle_outgoing_change(&self, change: CommonOutgoingChange) -> anyhow::Result<()> {
        if change.change.schema_version.major > CURRENT_SCHEMA_VERSION.major {
            warn!(
                "Skipping outgoing record '{}:{}': newer schema version {}",
                change.change.id.r#type, change.change.id.data_id, change.change.schema_version
            );
            return Ok(());
        }

        let Ok(record_type) = RecordType::from_str(&change.change.id.r#type) else {
            error!(
                "Unknown record type '{}' with compatible schema version {}",
                change.change.id.r#type, change.change.schema_version
            );
            return Ok(());
        };

        let RecordType::PaymentMetadata = record_type;
        self.handle_payment_metadata_update(change.change.updated_fields, change.change.id.data_id)
            .await
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

    /// Best-effort recovery: apply `sync_state` records to the relational DB that
    /// have a schema version newer than what the last recovery pass handled.
    /// This covers the case where a previous SDK version stored records in `sync_state`
    /// but couldn't apply them (e.g. unknown type at the time, now known after upgrade).
    async fn try_apply_new_sync_state_records(&self) {
        let current_version = CURRENT_SCHEMA_VERSION.to_string();
        let last_applied_version = match self
            .get_cached_item(RECOVERY_APPLIED_SCHEMA_VERSION_KEY.to_string())
            .await
        {
            Ok(Some(v)) if v == current_version => return,
            Ok(Some(v)) => match v.parse::<SchemaVersion>() {
                Ok(v) => Some(v),
                Err(e) => {
                    error!("Failed to parse last applied schema version '{v}': {e}");
                    return;
                }
            },
            Ok(None) => None,
            Err(e) => {
                error!("Failed to check recovery cache key: {e}");
                return;
            }
        };

        let mut records = match self.inner.get_sync_state_records().await {
            Ok(records) => records,
            Err(e) => {
                error!("Failed to get sync state records for recovery: {e}");
                return;
            }
        };

        // Sort by revision so records are replayed in the order the server received
        // them. This ensures correct behavior for record types that interact (e.g. a
        // contact creation at revision N is applied before its deletion at revision M>N).
        records.sort_by_key(|r| r.revision);

        for record in records {
            let schema_version = match record.schema_version.parse::<SchemaVersion>() {
                Ok(v) => v,
                Err(e) => {
                    error!(
                        "Failed to parse schema version '{}': {e}",
                        record.schema_version
                    );
                    continue;
                }
            };

            // Skip records with a newer major version (incompatible)
            if schema_version.major > CURRENT_SCHEMA_VERSION.major {
                continue;
            }

            // Skip records already processed in a previous recovery pass
            if let Some(ref last) = last_applied_version
                && schema_version <= *last
            {
                continue;
            }

            let Ok(record_type) = RecordType::from_str(&record.id.r#type) else {
                continue;
            };

            let data: HashMap<String, Value> = match record
                .data
                .into_iter()
                .map(|(k, v)| serde_json::from_str(&v).map(|parsed| (k, parsed)))
                .collect::<Result<_, _>>()
            {
                Ok(d) => d,
                Err(e) => {
                    error!(
                        "Failed to parse record data for '{}:{}': {e}",
                        record.id.r#type, record.id.data_id
                    );
                    continue;
                }
            };

            match record_type {
                RecordType::PaymentMetadata => {
                    if let Err(e) = self
                        .handle_payment_metadata_update(data, record.id.data_id)
                        .await
                    {
                        error!("Failed to apply sync state record: {e}");
                    }
                }
            }
        }

        if let Err(e) = self
            .set_cached_item(
                RECOVERY_APPLIED_SCHEMA_VERSION_KEY.to_string(),
                current_version,
            )
            .await
        {
            error!("Failed to set recovery cache key: {e}");
        }
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

    async fn rebase_pending_outgoing_records(&self, revision: u64) -> Result<(), StorageError> {
        self.inner.rebase_pending_outgoing_records(revision).await
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

    async fn get_sync_state_records(&self) -> Result<Vec<Record>, StorageError> {
        self.inner.get_sync_state_records().await
    }
}

#[cfg(all(test, not(all(target_family = "wasm", target_os = "unknown"))))]
mod tests {
    use super::*;
    use crate::persist::sqlite::SqliteStorage;
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
        SyncedStorage::new(storage, sync_service, event_emitter)
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
                revision: 1,
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
                payment_hash: "abc123".to_string(),
                destination_pubkey: "02def456".to_string(),
                description: None,
                preimage: None,
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
            SchemaVersion::new(2, 0, 0),
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
        let change =
            make_incoming_change("PaymentMetadata", "id1", SchemaVersion::new(2, 0, 0), data);
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
        let change =
            make_incoming_change("PaymentMetadata", "pay1", SchemaVersion::new(1, 1, 0), data);
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
            SchemaVersion::new(2, 0, 0),
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

    #[tokio::test]
    async fn test_recovery_applies_new_records_only() {
        let temp_dir = create_temp_dir("recovery_new_only");
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&temp_dir).unwrap());
        let synced = create_test_synced_storage(Arc::clone(&storage));

        storage
            .insert_payment(make_test_lightning_payment("pay1"))
            .await
            .unwrap();
        storage
            .insert_payment(make_test_lightning_payment("pay2"))
            .await
            .unwrap();

        // Simulate a previous recovery pass at version 0.9.0
        synced
            .set_cached_item(
                RECOVERY_APPLIED_SCHEMA_VERSION_KEY.to_string(),
                "0.9.0".to_string(),
            )
            .await
            .unwrap();

        // Insert a record at 0.9.0 (should be skipped — already handled)
        let mut old_data = HashMap::new();
        old_data.insert(
            "lnurl_pay_info".to_string(),
            serde_json::to_string(&serde_json::json!({"ln_address": "old@example.com"})).unwrap(),
        );
        let old_record = crate::sync_storage::Record {
            id: crate::sync_storage::RecordId::new(
                "PaymentMetadata".to_string(),
                "pay1".to_string(),
            ),
            revision: 1,
            schema_version: "0.9.0".to_string(),
            data: old_data,
        };
        storage
            .update_record_from_incoming(old_record)
            .await
            .unwrap();

        // Insert a record at 1.0.0 (newer than last applied — should be processed)
        let mut new_data = HashMap::new();
        new_data.insert(
            "lnurl_pay_info".to_string(),
            serde_json::to_string(&serde_json::json!({"ln_address": "new@example.com"})).unwrap(),
        );
        let new_record = crate::sync_storage::Record {
            id: crate::sync_storage::RecordId::new(
                "PaymentMetadata".to_string(),
                "pay2".to_string(),
            ),
            revision: 2,
            schema_version: "1.0.0".to_string(),
            data: new_data,
        };
        storage
            .update_record_from_incoming(new_record)
            .await
            .unwrap();

        synced.try_apply_new_sync_state_records().await;

        // pay1 (version 0.9.0) should NOT have been applied
        let payment1 = storage.get_payment_by_id("pay1".to_string()).await.unwrap();
        if let Some(crate::PaymentDetails::Lightning { lnurl_pay_info, .. }) = &payment1.details {
            assert!(
                lnurl_pay_info.is_none(),
                "pay1 metadata should not be applied (old schema version)"
            );
        } else {
            panic!("Expected Lightning payment details");
        }

        // pay2 (version 1.0.0) SHOULD have been applied
        let payment2 = storage.get_payment_by_id("pay2".to_string()).await.unwrap();
        if let Some(crate::PaymentDetails::Lightning { lnurl_pay_info, .. }) = &payment2.details {
            let info = lnurl_pay_info
                .as_ref()
                .expect("lnurl_pay_info should be set after recovery");
            assert_eq!(info.ln_address.as_deref(), Some("new@example.com"));
        } else {
            panic!("Expected Lightning payment details");
        }
    }

    #[tokio::test]
    async fn test_recovery_skips_unknown_and_newer_records() {
        let temp_dir = create_temp_dir("recovery_skip");
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&temp_dir).unwrap());
        let synced = create_test_synced_storage(Arc::clone(&storage));

        // Insert an unknown type record into sync_state
        let record_unknown = crate::sync_storage::Record {
            id: crate::sync_storage::RecordId::new("FutureType".to_string(), "id1".to_string()),
            revision: 1,
            schema_version: "1.0.0".to_string(),
            data: HashMap::new(),
        };
        storage
            .update_record_from_incoming(record_unknown)
            .await
            .unwrap();

        // Insert a record with newer major schema version
        let record_newer = crate::sync_storage::Record {
            id: crate::sync_storage::RecordId::new(
                "PaymentMetadata".to_string(),
                "id2".to_string(),
            ),
            revision: 2,
            schema_version: "2.0.0".to_string(),
            data: HashMap::new(),
        };
        storage
            .update_record_from_incoming(record_newer)
            .await
            .unwrap();

        // Recovery should not panic or error
        synced.try_apply_new_sync_state_records().await;

        // Verify no payment metadata was applied for either record
        assert!(storage.get_payment_by_id("id1".to_string()).await.is_err());
        assert!(storage.get_payment_by_id("id2".to_string()).await.is_err());
    }
}
