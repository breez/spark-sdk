use std::{
    collections::HashMap,
    fmt::{Display, Formatter},
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
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
    PaymentMetadata, SparkSettledBolt11Receive, SparkSettledBolt11Send, Storage, StorageError,
    UpdateDepositPayload,
    events::{InternalSyncedEvent, SdkEvent},
    lnurl::LnurlServerClient,
    persist::{
        LIGHTNING_ADDRESS_KEY, ObjectCacheRepository, StorageListPaymentsRequest,
        StoredCrossChainSwap, parse_cached_lightning_address,
    },
    sync_storage::{IncomingChange, OutgoingChange, Record, UnversionedRecordChange},
    utils::{payments::emit_payment_metadata_updated, time::now_secs},
};
use platform_utils::tokio;
use serde::{Deserialize, Serialize};

const INITIAL_SYNC_CACHE_KEY: &str = "sync_initial_complete";

enum RecordType {
    PaymentMetadata,
    Contact,
    LightningAddress,
    CrossChainSwap,
    /// A [`SparkSettledBolt11Send`], keyed by its payment id.
    SparkSettledBolt11Send,
    /// A [`SparkSettledBolt11Receive`], keyed by its id.
    SparkSettledBolt11Receive,
}

impl RecordType {
    #[allow(clippy::match_same_arms)] // Arms will diverge as types evolve independently.
    const fn schema_version(&self) -> SchemaVersion {
        match self {
            Self::PaymentMetadata => SchemaVersion::new(1, 0, 0),
            Self::Contact => SchemaVersion::new(1, 0, 0),
            Self::LightningAddress => SchemaVersion::new(1, 0, 0),
            Self::CrossChainSwap => SchemaVersion::new(1, 0, 0),
            Self::SparkSettledBolt11Send => SchemaVersion::new(1, 0, 0),
            Self::SparkSettledBolt11Receive => SchemaVersion::new(1, 0, 0),
        }
    }
}

impl Display for RecordType {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        let s = match self {
            RecordType::PaymentMetadata => "PaymentMetadata",
            RecordType::Contact => "Contact",
            RecordType::LightningAddress => "LightningAddress",
            RecordType::CrossChainSwap => "CrossChainSwap",
            RecordType::SparkSettledBolt11Send => "SparkSettledBolt11Send",
            RecordType::SparkSettledBolt11Receive => "SparkSettledBolt11Receive",
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
            "CrossChainSwap" => Ok(RecordType::CrossChainSwap),
            "SparkSettledBolt11Send" => Ok(RecordType::SparkSettledBolt11Send),
            "SparkSettledBolt11Receive" => Ok(RecordType::SparkSettledBolt11Receive),
            _ => Err(format!("Unknown record type: {s}")),
        }
    }
}

const LIGHTNING_ADDRESS_DATA_ID: &str = "current";

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

/// Storage wrapper that mirrors local writes into the real-time sync queue.
///
/// This is `sdk.storage`, which is reachable from the `EventEmitter` (handlers
/// and listeners registered on the emitter hold it), so it must not reference
/// the emitter or the SDK graph could never be dropped. Event emission for
/// sync outcomes lives in [`SyncedRecordHandler`] instead.
pub struct SyncedStorage {
    inner: Arc<dyn Storage>,
    sync_service: Arc<SyncService>,
}

/// Applies incoming and replayed sync records to local storage and reports
/// sync outcomes on the event emitter.
///
/// Owned by the `SyncProcessor`, whose tasks stop on the shutdown signal, so
/// the strong emitter reference is released on `disconnect()` and cannot form
/// a cycle (the emitter never owns the processor).
pub struct SyncedRecordHandler {
    storage: Arc<dyn Storage>,
    event_emitter: Arc<EventEmitter>,
    lnurl_server_client: Option<Arc<dyn LnurlServerClient>>,
    /// Set once the first pull finished. Metadata applied before that is the
    /// catch-up of everything missed while offline, and is not announced.
    initial_pull_done: AtomicBool,
}

#[macros::async_trait]
impl NewRecordHandler for SyncedRecordHandler {
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

        self.initial_pull_done.store(true, Ordering::Relaxed);

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
    pub fn new(inner: Arc<dyn Storage>, sync_service: Arc<SyncService>) -> Self {
        SyncedStorage {
            inner,
            sync_service,
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
                    conversion_info,
                    ..
                } => (
                    description,
                    lnurl_pay_info,
                    lnurl_withdraw_info,
                    conversion_info,
                ),
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

    async fn push_lightning_address_sync(&self) {
        if let Err(e) = self
            .sync_service
            .set_outgoing_record(&RecordChangeRequest {
                id: RecordId::new(
                    RecordType::LightningAddress.to_string(),
                    LIGHTNING_ADDRESS_DATA_ID,
                ),
                schema_version: RecordType::LightningAddress.schema_version(),
                updated_fields: HashMap::new(),
            })
            .await
        {
            error!("Failed to push lightning address sync signal: {e:?}");
        }
    }
}

impl SyncedRecordHandler {
    pub fn new(
        storage: Arc<dyn Storage>,
        event_emitter: Arc<EventEmitter>,
        lnurl_server_client: Option<Arc<dyn LnurlServerClient>>,
    ) -> Self {
        SyncedRecordHandler {
            storage,
            event_emitter,
            lnurl_server_client,
            initial_pull_done: AtomicBool::new(false),
        }
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
                return Ok(self.handle_lightning_address_change());
            }
            RecordType::CrossChainSwap => {
                self.handle_cross_chain_swap_change(change.new_state.data)
                    .await
            }
            RecordType::SparkSettledBolt11Send => {
                self.handle_spark_settled_bolt11_send_change(change.new_state.data)
                    .await
            }
            RecordType::SparkSettledBolt11Receive => {
                self.handle_spark_settled_bolt11_receive_change(change.new_state.data)
                    .await
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
            RecordType::CrossChainSwap => {
                self.handle_cross_chain_swap_change(change.change.updated_fields)
                    .await
            }
            RecordType::SparkSettledBolt11Send => {
                self.handle_spark_settled_bolt11_send_change(change.change.updated_fields)
                    .await
            }
            RecordType::SparkSettledBolt11Receive => {
                self.handle_spark_settled_bolt11_receive_change(change.change.updated_fields)
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

        self.storage
            .insert_payment_metadata(data_id.clone(), metadata)
            .await?;
        if self.initial_pull_done.load(Ordering::Relaxed) {
            emit_payment_metadata_updated(&self.storage, &self.event_emitter, &data_id).await;
        }
        Ok(())
    }

    /// Applies a [`SparkSettledBolt11Send`] another device recorded when it
    /// paid.
    ///
    /// Stored whether or not the transfer is here yet: the row is read when a
    /// payment is, so either arrival order reports the same thing.
    async fn handle_spark_settled_bolt11_send_change(
        &self,
        fields: HashMap<String, Value>,
    ) -> anyhow::Result<()> {
        let send: SparkSettledBolt11Send = serde_json::from_value(
            serde_json::to_value(&fields)
                .map_err(|e| StorageError::Serialization(e.to_string()))?,
        )
        .map_err(|e| StorageError::Serialization(e.to_string()))?;

        self.storage.set_spark_settled_bolt11_send(send).await?;
        Ok(())
    }

    /// Applies a [`SparkSettledBolt11Receive`] another device recorded when
    /// it created the Bolt11.
    ///
    /// Stored whether or not the transfer settling the invoice is here yet: the
    /// row is read when a payment is, so either arrival order reports the same
    /// thing.
    async fn handle_spark_settled_bolt11_receive_change(
        &self,
        fields: HashMap<String, Value>,
    ) -> anyhow::Result<()> {
        let receive: SparkSettledBolt11Receive = serde_json::from_value(
            serde_json::to_value(&fields)
                .map_err(|e| StorageError::Serialization(e.to_string()))?,
        )
        .map_err(|e| StorageError::Serialization(e.to_string()))?;

        self.storage
            .set_spark_settled_bolt11_receive(receive)
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
            let _ = self.storage.delete_contact(data_id).await;
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
        self.storage.insert_contact(contact).await?;

        Ok(())
    }

    async fn handle_cross_chain_swap_change(
        &self,
        fields: HashMap<String, Value>,
    ) -> anyhow::Result<()> {
        let swap: StoredCrossChainSwap = serde_json::from_value(
            serde_json::to_value(&fields)
                .map_err(|e| StorageError::Serialization(e.to_string()))?,
        )
        .map_err(|e| StorageError::Serialization(e.to_string()))?;
        self.storage.set_cross_chain_swap(swap).await?;
        Ok(())
    }

    fn handle_lightning_address_change(&self) -> RecordOutcome {
        let Some(client) = &self.lnurl_server_client else {
            return RecordOutcome::Completed;
        };

        let client = Arc::clone(client);
        let storage = Arc::clone(&self.storage);
        let event_emitter = Arc::clone(&self.event_emitter);
        let span = tracing::Span::current();

        tokio::spawn(
            async move {
                let cache = ObjectCacheRepository::new(Arc::clone(&storage));
                let old = cache
                    .fetch_lightning_address()
                    .await
                    .ok()
                    .flatten()
                    .flatten();

                let resp = match client.recover_lightning_address().await {
                    Ok(resp) => resp,
                    Err(e) => {
                        warn!("Failed to recover lightning address after sync trigger: {e:?}");
                        if let Err(e) = storage
                            .delete_cached_item(LIGHTNING_ADDRESS_KEY.to_string())
                            .await
                        {
                            error!("Failed to reset lightning address cache: {e:?}");
                        }
                        return;
                    }
                };

                let new = if let Some(resp) = resp {
                    let address_info = resp.into();
                    if let Err(e) = cache.save_lightning_address(&address_info, true).await {
                        error!("Failed to save recovered lightning address: {e:?}");
                        if let Err(e) = storage
                            .delete_cached_item(LIGHTNING_ADDRESS_KEY.to_string())
                            .await
                        {
                            error!("Failed to reset lightning address cache: {e:?}");
                        }
                        return;
                    }
                    Some(address_info)
                } else {
                    if let Err(e) = cache.delete_lightning_address(true).await {
                        error!("Failed to delete lightning address from cache: {e:?}");
                        if let Err(e) = storage
                            .delete_cached_item(LIGHTNING_ADDRESS_KEY.to_string())
                            .await
                        {
                            error!("Failed to reset lightning address cache: {e:?}");
                        }
                        return;
                    }
                    None
                };

                if old != new {
                    event_emitter
                        .emit(&SdkEvent::LightningAddressChanged {
                            lightning_address: new,
                        })
                        .await;
                }
            }
            .instrument(span),
        );

        RecordOutcome::Completed
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
        if key == LIGHTNING_ADDRESS_KEY
            && let Ok(cached) = parse_cached_lightning_address(&value)
            && !cached.recovered
        {
            self.push_lightning_address_sync().await;
        }
        self.inner.set_cached_item(key, value).await
    }
    async fn list_payments(
        &self,
        request: StorageListPaymentsRequest,
    ) -> Result<Vec<Payment>, StorageError> {
        self.inner.list_payments(request).await
    }

    async fn apply_payment_update(&self, payment: Payment) -> Result<bool, StorageError> {
        self.inner.apply_payment_update(payment).await
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
        is_mature: bool,
    ) -> Result<(), StorageError> {
        self.inner
            .add_deposit(txid, vout, amount_sats, is_mature)
            .await
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

    // Local-only: the watch list is a per-instance chain-polling optimization,
    // so it is not replicated the way the deposits themselves are.
    async fn list_watched_deposit_addresses(
        &self,
    ) -> Result<Vec<crate::persist::WatchedDepositAddress>, StorageError> {
        self.inner.list_watched_deposit_addresses().await
    }

    async fn update_watched_deposit_address(
        &self,
        address: String,
        payload: crate::persist::UpdateWatchedAddressPayload,
    ) -> Result<(), StorageError> {
        self.inner
            .update_watched_deposit_address(address, payload)
            .await
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
        let now = now_secs();
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

    async fn set_cross_chain_swap(&self, swap: StoredCrossChainSwap) -> Result<(), StorageError> {
        let data_id = format!("{}:{}", swap.provider, swap.id);
        self.sync_service
            .set_outgoing_record(&RecordChangeRequest {
                id: RecordId::new(RecordType::CrossChainSwap.to_string(), &data_id),
                schema_version: RecordType::CrossChainSwap.schema_version(),
                updated_fields: serde_json::from_value(
                    serde_json::to_value(&swap)
                        .map_err(|e| StorageError::Serialization(e.to_string()))?,
                )
                .map_err(|e| StorageError::Serialization(e.to_string()))?,
            })
            .await
            .map_err(|e| StorageError::Implementation(e.to_string()))?;
        self.inner.set_cross_chain_swap(swap).await
    }

    async fn get_cross_chain_swap(
        &self,
        provider: String,
        id: String,
    ) -> Result<Option<StoredCrossChainSwap>, StorageError> {
        self.inner.get_cross_chain_swap(provider, id).await
    }

    async fn list_active_cross_chain_swaps(
        &self,
        provider: String,
    ) -> Result<Vec<StoredCrossChainSwap>, StorageError> {
        self.inner.list_active_cross_chain_swaps(provider).await
    }

    /// The payer is the only one who knows which Bolt11 a transfer settled,
    /// and only the device that sent it. Sharing the row is what lets the
    /// account's other devices report the send as that invoice too.
    async fn set_spark_settled_bolt11_send(
        &self,
        send: SparkSettledBolt11Send,
    ) -> Result<(), StorageError> {
        self.sync_service
            .set_outgoing_record(&RecordChangeRequest {
                id: RecordId::new(
                    RecordType::SparkSettledBolt11Send.to_string(),
                    &send.payment_id,
                ),
                schema_version: RecordType::SparkSettledBolt11Send.schema_version(),
                updated_fields: serde_json::from_value(
                    serde_json::to_value(&send)
                        .map_err(|e| StorageError::Serialization(e.to_string()))?,
                )
                .map_err(|e| StorageError::Serialization(e.to_string()))?,
            })
            .await
            .map_err(|e| StorageError::Implementation(e.to_string()))?;
        self.inner.set_spark_settled_bolt11_send(send).await
    }

    /// The receiver's half: only the device that created the Bolt11 knows which
    /// Spark invoice it embedded, and a transfer settling that invoice can be
    /// claimed on any of the account's devices.
    async fn set_spark_settled_bolt11_receive(
        &self,
        receive: SparkSettledBolt11Receive,
    ) -> Result<(), StorageError> {
        self.sync_service
            .set_outgoing_record(&RecordChangeRequest {
                id: RecordId::new(
                    RecordType::SparkSettledBolt11Receive.to_string(),
                    &receive.id,
                ),
                schema_version: RecordType::SparkSettledBolt11Receive.schema_version(),
                updated_fields: serde_json::from_value(
                    serde_json::to_value(&receive)
                        .map_err(|e| StorageError::Serialization(e.to_string()))?,
                )
                .map_err(|e| StorageError::Serialization(e.to_string()))?,
            })
            .await
            .map_err(|e| StorageError::Implementation(e.to_string()))?;
        self.inner.set_spark_settled_bolt11_receive(receive).await
    }

    async fn delete_expired_spark_settled_bolt11_receives(
        &self,
        before: u64,
    ) -> Result<(), StorageError> {
        self.inner
            .delete_expired_spark_settled_bolt11_receives(before)
            .await
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

#[cfg(all(test, feature = "sqlite"))]
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
        SyncedStorage::new(storage, sync_service)
    }

    fn create_test_record_handler(storage: Arc<dyn Storage>) -> SyncedRecordHandler {
        let event_emitter = Arc::new(EventEmitter::new(true));
        SyncedRecordHandler::new(storage, event_emitter, None)
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
                htlc_details: Some(SparkHtlcDetails {
                    payment_hash: "abc123".to_string(),
                    preimage: None,
                    expiry_time: 0,
                    status: crate::SparkHtlcStatus::WaitingForPreimage,
                }),
                lnurl_pay_info: None,
                lnurl_withdraw_info: None,
                lnurl_receive_metadata: None,
                conversion_info: None,
            }),
            conversion_details: None,
        }
    }

    #[tokio::test]
    async fn test_incoming_unknown_type_newer_schema() {
        let temp_dir = create_temp_dir("incoming_unknown_newer");
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&temp_dir).unwrap());
        let handler = create_test_record_handler(Arc::clone(&storage));

        let change = make_incoming_change(
            "FutureType",
            "id1",
            SchemaVersion::new(99, 0, 0),
            HashMap::new(),
        );
        let result = handler.handle_incoming_change(change).await;
        assert!(result.is_ok());

        // Verify no payment was created as a side effect
        assert!(storage.get_payment_by_id("id1".to_string()).await.is_err());
    }

    #[tokio::test]
    async fn test_incoming_unknown_type_compatible_schema() {
        let temp_dir = create_temp_dir("incoming_unknown_compat");
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&temp_dir).unwrap());
        let handler = create_test_record_handler(Arc::clone(&storage));

        let change = make_incoming_change(
            "UnknownType",
            "id1",
            SchemaVersion::new(1, 0, 0),
            HashMap::new(),
        );
        let result = handler.handle_incoming_change(change).await;
        assert!(result.is_ok());

        // Verify no payment metadata was written
        assert!(storage.get_payment_by_id("id1".to_string()).await.is_err());
    }

    #[tokio::test]
    async fn test_incoming_known_type_newer_major_version() {
        let temp_dir = create_temp_dir("incoming_known_newer_major");
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&temp_dir).unwrap());
        let handler = create_test_record_handler(Arc::clone(&storage));

        // Insert a payment so we can verify metadata was NOT written
        storage
            .apply_payment_update(make_test_lightning_payment("id1"))
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
        let result = handler.handle_incoming_change(change).await;
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
        let handler = create_test_record_handler(Arc::clone(&storage));

        storage
            .apply_payment_update(make_test_lightning_payment("pay1"))
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
        let result = handler.handle_incoming_change(change).await;
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

    /// Sync record carrying an untyped (pre-migration) `conversion_info` JSON
    /// is upgraded to a tagged `ConversionInfo::Amm` by the lenient
    /// deserializer, then persisted by the strict re-serialize on insert.
    /// Future direct reads see the tagged row.
    #[tokio::test]
    async fn test_incoming_payment_metadata_upgrades_pre_migration_conversion_info() {
        let temp_dir = create_temp_dir("incoming_pm_pre_migration");
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&temp_dir).unwrap());
        let handler = create_test_record_handler(Arc::clone(&storage));

        storage
            .apply_payment_update(make_test_lightning_payment("pm-upgrade"))
            .await
            .unwrap();

        // Pre-migration shape: no `"type"` tag on the conversion_info object.
        let mut data = HashMap::new();
        data.insert(
            "conversion_info".to_string(),
            serde_json::json!({
                "pool_id": "pool-1",
                "conversion_id": "conv-1",
                "status": "Pending",
                "fee": "100",
                "purpose": null,
            }),
        );
        let pm_version = RecordType::PaymentMetadata.schema_version();
        let change = make_incoming_change("PaymentMetadata", "pm-upgrade", pm_version, data);
        let _ = handler.handle_incoming_change(change).await.unwrap();

        let payment = storage
            .get_payment_by_id("pm-upgrade".to_string())
            .await
            .unwrap();
        let Some(crate::PaymentDetails::Lightning {
            conversion_info, ..
        }) = &payment.details
        else {
            panic!("Expected Lightning payment details");
        };
        match conversion_info
            .as_ref()
            .expect("conversion_info should be set")
        {
            crate::ConversionInfo::Amm {
                pool_id,
                conversion_id,
                ..
            } => {
                assert_eq!(pool_id, "pool-1");
                assert_eq!(conversion_id, "conv-1");
            }
            other => panic!("Expected ConversionInfo::Amm, got {other:?}"),
        }
    }

    /// Sync record with an already-tagged `conversion_info` passes through
    /// unchanged (sanity check that the lenient deserializer doesn't corrupt
    /// modern records).
    #[tokio::test]
    async fn test_incoming_payment_metadata_preserves_tagged_conversion_info() {
        let temp_dir = create_temp_dir("incoming_pm_tagged");
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&temp_dir).unwrap());
        let handler = create_test_record_handler(Arc::clone(&storage));

        storage
            .apply_payment_update(make_test_lightning_payment("pm-tagged"))
            .await
            .unwrap();

        let mut data = HashMap::new();
        data.insert(
            "conversion_info".to_string(),
            serde_json::json!({
                "type": "amm",
                "pool_id": "pool-2",
                "conversion_id": "conv-2",
                "status": "Completed",
                "fee": "200",
                "purpose": null,
            }),
        );
        let pm_version = RecordType::PaymentMetadata.schema_version();
        let change = make_incoming_change("PaymentMetadata", "pm-tagged", pm_version, data);
        let _ = handler.handle_incoming_change(change).await.unwrap();

        let payment = storage
            .get_payment_by_id("pm-tagged".to_string())
            .await
            .unwrap();
        let Some(crate::PaymentDetails::Lightning {
            conversion_info, ..
        }) = &payment.details
        else {
            panic!("Expected Lightning payment details");
        };
        assert!(matches!(
            conversion_info,
            Some(crate::ConversionInfo::Amm { .. })
        ));
    }

    #[tokio::test]
    async fn test_outgoing_unknown_type_newer_schema() {
        let temp_dir = create_temp_dir("outgoing_unknown_newer");
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&temp_dir).unwrap());
        let handler = create_test_record_handler(Arc::clone(&storage));

        let change = make_outgoing_change(
            "FutureType",
            "id1",
            SchemaVersion::new(99, 0, 0),
            HashMap::new(),
        );
        let result = handler.handle_outgoing_change(change).await;
        assert!(result.is_ok());

        // Verify no payment metadata side effect
        assert!(storage.get_payment_by_id("id1".to_string()).await.is_err());
    }

    #[tokio::test]
    async fn test_outgoing_unknown_type_compatible_schema() {
        let temp_dir = create_temp_dir("outgoing_unknown_compat");
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&temp_dir).unwrap());
        let handler = create_test_record_handler(Arc::clone(&storage));

        let change = make_outgoing_change(
            "UnknownType",
            "id1",
            SchemaVersion::new(1, 0, 0),
            HashMap::new(),
        );
        let result = handler.handle_outgoing_change(change).await;
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
        let handler = create_test_record_handler(Arc::clone(&storage));

        let data = make_contact_data("Alice", "alice@example.com");
        let change =
            make_incoming_change("Contact", "c1", RecordType::Contact.schema_version(), data);
        let result = handler.handle_incoming_change(change).await;
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
        let handler = create_test_record_handler(Arc::clone(&storage));

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
        let result = handler.handle_incoming_change(change).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), RecordOutcome::Completed);

        assert!(storage.get_contact("c1".to_string()).await.is_err());
    }

    #[tokio::test]
    async fn test_outgoing_replay_contact_without_deleted_at_upserts() {
        let temp_dir = create_temp_dir("outgoing_contact_upsert");
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&temp_dir).unwrap());
        let handler = create_test_record_handler(Arc::clone(&storage));

        let data = make_contact_data("Bob", "bob@example.com");
        let change =
            make_outgoing_change("Contact", "c2", RecordType::Contact.schema_version(), data);
        let result = handler.handle_outgoing_change(change).await;
        assert!(result.is_ok());

        let contact = storage.get_contact("c2".to_string()).await.unwrap();
        assert_eq!(contact.name, "Bob");
    }

    #[tokio::test]
    async fn test_outgoing_replay_contact_with_deleted_at_deletes() {
        let temp_dir = create_temp_dir("outgoing_contact_delete");
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&temp_dir).unwrap());
        let handler = create_test_record_handler(Arc::clone(&storage));

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
        let result = handler.handle_outgoing_change(change).await;
        assert!(result.is_ok());

        assert!(storage.get_contact("c3".to_string()).await.is_err());
    }

    #[tokio::test]
    async fn test_cross_chain_swap_sync_round_trip_boltz() {
        run_cross_chain_swap_round_trip("boltz", "swap-1").await;
    }

    #[tokio::test]
    async fn test_cross_chain_swap_sync_round_trip_orchestra() {
        run_cross_chain_swap_round_trip("orchestra", "quote-1").await;
    }

    async fn run_cross_chain_swap_round_trip(provider: &str, id: &str) {
        // Instance A: set_cross_chain_swap emits an outgoing record and writes
        // locally.
        let writer_dir = create_temp_dir(&format!("cross_chain_sync_writer_{provider}"));
        let writer_inner: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&writer_dir).unwrap());
        let synced = create_test_synced_storage(Arc::clone(&writer_inner));

        let stored = StoredCrossChainSwap {
            provider: provider.to_string(),
            id: id.to_string(),
            is_terminal: false,
            updated_at: 1700,
            data: format!(r#"{{"id":"{id}","status":"Created"}}"#),
            secrets: "c2VjcmV0".to_string(),
        };
        synced.set_cross_chain_swap(stored.clone()).await.unwrap();

        let expected_data_id = format!("{provider}:{id}");

        // Exactly the fields the writer emitted are replayed (not re-serialized
        // here), so the test catches any emit/apply field mismatch.
        let emitted = writer_inner
            .get_pending_outgoing_changes(100)
            .await
            .unwrap()
            .into_iter()
            .find(|c| {
                c.change.id.r#type == RecordType::CrossChainSwap.to_string()
                    && c.change.id.data_id == expected_data_id
            })
            .expect("set_cross_chain_swap must queue a CrossChainSwap outgoing record");
        let fields: HashMap<String, Value> = emitted
            .change
            .updated_fields
            .into_iter()
            .map(|(k, v)| (k, serde_json::from_str(&v).unwrap()))
            .collect();

        // Instance B: apply the emitted record as an incoming change.
        let reader_dir = create_temp_dir(&format!("cross_chain_sync_reader_{provider}"));
        let reader_inner: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&reader_dir).unwrap());
        let handler = create_test_record_handler(Arc::clone(&reader_inner));

        let change = make_incoming_change(
            &RecordType::CrossChainSwap.to_string(),
            &expected_data_id,
            RecordType::CrossChainSwap.schema_version(),
            fields,
        );
        let outcome = handler.handle_incoming_change(change).await.unwrap();
        assert_eq!(outcome, RecordOutcome::Completed);

        // Instance B's local store now holds an identical row.
        let fetched = reader_inner
            .get_cross_chain_swap(provider.to_string(), id.to_string())
            .await
            .unwrap()
            .expect("swap applied to reader store");
        assert_eq!(fetched.provider, stored.provider);
        assert_eq!(fetched.id, stored.id);
        assert_eq!(fetched.is_terminal, stored.is_terminal);
        assert_eq!(fetched.updated_at, stored.updated_at);
        assert_eq!(fetched.data, stored.data);
        assert_eq!(fetched.secrets, stored.secrets);
    }

    /// Helper: returns the number of pending outgoing sync changes with record type
    /// `LightningAddress`.
    async fn lightning_address_outgoing_count(storage: &Arc<dyn Storage>) -> usize {
        storage
            .get_pending_outgoing_changes(100)
            .await
            .unwrap()
            .iter()
            .filter(|c| c.change.id.r#type == RecordType::LightningAddress.to_string())
            .count()
    }

    #[tokio::test]
    async fn test_set_cached_lightning_address_with_recovered_false_triggers_sync() {
        let temp_dir = create_temp_dir("la_sync_not_recovered");
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&temp_dir).unwrap());
        let synced = create_test_synced_storage(Arc::clone(&storage));

        assert_eq!(lightning_address_outgoing_count(&storage).await, 0);

        // A client-initiated save (recovered: false) should trigger a sync push
        let cache = ObjectCacheRepository::new(Arc::new(synced) as Arc<dyn Storage>);
        let address = crate::LightningAddressInfo {
            lightning_address: "test@example.com".to_string(),
            username: "test".to_string(),
            description: "Test".to_string(),
            lnurl: crate::LnurlInfo::new("https://example.com/.well-known/lnurlp/test".to_string()),
        };
        cache.save_lightning_address(&address, false).await.unwrap();

        assert_eq!(lightning_address_outgoing_count(&storage).await, 1);
    }

    #[tokio::test]
    async fn test_set_cached_lightning_address_with_recovered_true_does_not_trigger_sync() {
        let temp_dir = create_temp_dir("la_sync_recovered");
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&temp_dir).unwrap());
        let synced = create_test_synced_storage(Arc::clone(&storage));

        assert_eq!(lightning_address_outgoing_count(&storage).await, 0);

        // A recovery save (recovered: true) should NOT trigger a sync push
        let cache = ObjectCacheRepository::new(Arc::new(synced) as Arc<dyn Storage>);
        let address = crate::LightningAddressInfo {
            lightning_address: "test@example.com".to_string(),
            username: "test".to_string(),
            description: "Test".to_string(),
            lnurl: crate::LnurlInfo::new("https://example.com/.well-known/lnurlp/test".to_string()),
        };
        cache.save_lightning_address(&address, true).await.unwrap();

        assert_eq!(lightning_address_outgoing_count(&storage).await, 0);
    }

    #[tokio::test]
    async fn test_delete_cached_lightning_address_with_recovered_false_triggers_sync() {
        let temp_dir = create_temp_dir("la_delete_sync_not_recovered");
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&temp_dir).unwrap());
        let synced = create_test_synced_storage(Arc::clone(&storage));

        let cache = ObjectCacheRepository::new(Arc::new(synced) as Arc<dyn Storage>);
        // A client-initiated delete (recovered: false) should trigger a sync push
        cache.delete_lightning_address(false).await.unwrap();

        assert_eq!(lightning_address_outgoing_count(&storage).await, 1);
    }

    #[tokio::test]
    async fn test_delete_cached_lightning_address_with_recovered_true_does_not_trigger_sync() {
        let temp_dir = create_temp_dir("la_delete_sync_recovered");
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&temp_dir).unwrap());
        let synced = create_test_synced_storage(Arc::clone(&storage));

        let cache = ObjectCacheRepository::new(Arc::new(synced) as Arc<dyn Storage>);
        // A recovery delete (recovered: true) should NOT trigger a sync push
        cache.delete_lightning_address(true).await.unwrap();

        assert_eq!(lightning_address_outgoing_count(&storage).await, 0);
    }

    // A regtest Bolt11 and a regtest Spark invoice, both real so they parse.
    const TEST_BOLT11: &str = "lnbcrt10u1p42j5khpp57zyscpf43g90q9de4za4ptj6xpyp9snztynac9k7q2vet7kuknpssp58tfaf7uq5z6vgaphxg64zv9z69kd399llwvu0d760w8z48sd282qxqyz5vqnp4qtlyk6hxw5h4hrdfdkd4nh2rv0mwyyqvdtakr3dv6m4vvsmfshvg6cqzpudqq9qyyssq6gnvg355jjmqtw73pfevvkpf788j4xsftv2h7xhfyj4jvzkljxurmh7dydt2dyex7te49hfstkfg950vaepejaxf8gugft9fvequ7qsqu49at4";
    const TEST_SPARK_INVOICE: &str = "sparkrt1pgss8cf4gru7ece2ryn8ym3vm3yz8leeend2589m7svq2mgv0xncfyx8zf8ssqgjzqqe5pmwfwyh9u4u6wgrepzk7j6j5prdv4kk7v3pqdur4y4c5nlcyr7lksm4mhrhdzakas9yt8gz4levtnfe49sgkqknywstpzxd8hk8qcgvp7x22q3qxz8gqudyp7rmuglc2axjqnlzz7d047gndmxff6ud02fvdgasdsq2en2aah6g52rq4qq7peler4s4d85s7prhm6sqzqj7gvc9nlzucy4yfh206fyqpk9zez";

    /// A Spark payment, carrying `spark_invoice` when it settled one.
    fn make_spark_payment(id: &str, spark_invoice: Option<&str>) -> crate::Payment {
        crate::Payment {
            id: id.to_string(),
            payment_type: crate::PaymentType::Receive,
            status: crate::PaymentStatus::Completed,
            amount: 1000,
            fees: 0,
            timestamp: now_secs(),
            method: crate::PaymentMethod::Spark,
            details: Some(crate::PaymentDetails::Spark {
                invoice_details: spark_invoice.map(|invoice| crate::SparkInvoicePaymentDetails {
                    description: None,
                    invoice: invoice.to_string(),
                }),
                htlc_details: None,
                conversion_info: None,
            }),
            conversion_details: None,
        }
    }

    fn test_receive(spark_invoice: &str, bolt11: &str) -> SparkSettledBolt11Receive {
        SparkSettledBolt11Receive {
            description: None,
            destination_pubkey: String::new(),
            id: crate::persist::spark_invoice_digest(spark_invoice),
            spark_invoice: spark_invoice.to_string(),
            bolt11: bolt11.to_string(),
            expires_at: crate::persist::spark_invoice_expiry_secs(spark_invoice),
        }
    }

    fn record_fields<T: Serialize>(record: &T) -> HashMap<String, Value> {
        serde_json::from_value(serde_json::to_value(record).unwrap()).unwrap()
    }

    /// Stores a Spark payment and returns the Bolt11 it reports, which is the
    /// only way a settled row is observable.
    async fn settled_bolt11(
        storage: &Arc<dyn Storage>,
        payment_id: &str,
        spark_invoice: Option<&str>,
    ) -> Option<String> {
        storage
            .apply_payment_update(make_spark_payment(payment_id, spark_invoice))
            .await
            .unwrap();
        match storage
            .get_payment_by_id(payment_id.to_string())
            .await
            .unwrap()
            .details
        {
            Some(crate::PaymentDetails::Lightning { invoice, .. }) => Some(invoice),
            _ => None,
        }
    }

    async fn outgoing_count(storage: &Arc<dyn Storage>, record_type: &RecordType) -> usize {
        storage
            .get_pending_outgoing_changes(100)
            .await
            .unwrap()
            .iter()
            .filter(|c| c.change.id.r#type == record_type.to_string())
            .count()
    }

    #[tokio::test]
    async fn test_set_spark_settled_bolt11_receive_triggers_sync() {
        let temp_dir = create_temp_dir("spark_settled_bolt11_receive_sync_push");
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&temp_dir).unwrap());
        let synced = create_test_synced_storage(Arc::clone(&storage));

        let receive = test_receive(TEST_SPARK_INVOICE, TEST_BOLT11);
        synced
            .set_spark_settled_bolt11_receive(receive.clone())
            .await
            .unwrap();

        assert_eq!(
            outgoing_count(&storage, &RecordType::SparkSettledBolt11Receive).await,
            1
        );
        let change = storage.get_latest_outgoing_change().await.unwrap().unwrap();
        assert_eq!(change.change.id.data_id, receive.id);
        // Queued field values are stored JSON-encoded.
        assert_eq!(
            change.change.updated_fields.get("sparkInvoice"),
            Some(&serde_json::to_string(TEST_SPARK_INVOICE).unwrap())
        );
        assert_eq!(
            change.change.updated_fields.get("bolt11"),
            Some(&serde_json::to_string(TEST_BOLT11).unwrap())
        );
        assert_eq!(
            settled_bolt11(&storage, "settled", Some(TEST_SPARK_INVOICE)).await,
            Some(TEST_BOLT11.to_string())
        );
    }

    #[tokio::test]
    async fn test_set_spark_settled_bolt11_send_triggers_sync() {
        let temp_dir = create_temp_dir("spark_settled_bolt11_send_sync_push");
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&temp_dir).unwrap());
        let synced = create_test_synced_storage(Arc::clone(&storage));

        synced
            .set_spark_settled_bolt11_send(SparkSettledBolt11Send {
                payment_id: "transfer-1".to_string(),
                bolt11: TEST_BOLT11.to_string(),
                description: None,
                destination_pubkey: String::new(),
            })
            .await
            .unwrap();

        assert_eq!(
            outgoing_count(&storage, &RecordType::SparkSettledBolt11Send).await,
            1
        );
        let change = storage.get_latest_outgoing_change().await.unwrap().unwrap();
        assert_eq!(change.change.id.data_id, "transfer-1");
        assert_eq!(
            settled_bolt11(&storage, "transfer-1", None).await,
            Some(TEST_BOLT11.to_string())
        );
    }

    #[tokio::test]
    async fn test_incoming_spark_settled_bolt11_receive_is_stored_for_a_later_transfer() {
        let temp_dir = create_temp_dir("spark_settled_bolt11_receive_incoming_first");
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&temp_dir).unwrap());
        let handler = create_test_record_handler(Arc::clone(&storage));

        let receive = test_receive(TEST_SPARK_INVOICE, TEST_BOLT11);
        let change = make_incoming_change(
            "SparkSettledBolt11Receive",
            &receive.id,
            RecordType::SparkSettledBolt11Receive.schema_version(),
            record_fields(&receive),
        );
        assert_eq!(
            handler.handle_incoming_change(change).await.unwrap(),
            RecordOutcome::Completed
        );

        // The transfer lands after the row, and still reports as the Bolt11.
        assert_eq!(
            settled_bolt11(&storage, "settled", Some(TEST_SPARK_INVOICE)).await,
            Some(TEST_BOLT11.to_string())
        );
    }

    #[tokio::test]
    async fn test_incoming_spark_settled_bolt11_receive_attributes_the_stored_payment() {
        let temp_dir = create_temp_dir("spark_settled_bolt11_receive_incoming_second");
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&temp_dir).unwrap());
        let handler = create_test_record_handler(Arc::clone(&storage));

        storage
            .apply_payment_update(make_spark_payment("settled", Some(TEST_SPARK_INVOICE)))
            .await
            .unwrap();
        storage
            .apply_payment_update(make_spark_payment("other", Some("sparkrt1other")))
            .await
            .unwrap();

        let receive = test_receive(TEST_SPARK_INVOICE, TEST_BOLT11);
        let change = make_incoming_change(
            "SparkSettledBolt11Receive",
            &receive.id,
            RecordType::SparkSettledBolt11Receive.schema_version(),
            record_fields(&receive),
        );
        assert_eq!(
            handler.handle_incoming_change(change).await.unwrap(),
            RecordOutcome::Completed
        );

        let settled = storage
            .get_payment_by_id("settled".to_string())
            .await
            .unwrap();
        assert_eq!(settled.method, crate::PaymentMethod::Lightning);
        let Some(crate::PaymentDetails::Lightning {
            invoice,
            htlc_details,
            ..
        }) = settled.details
        else {
            panic!("expected Lightning details, got {:?}", settled.details);
        };
        assert_eq!(invoice, TEST_BOLT11);
        assert!(htlc_details.is_none());

        let other = storage
            .get_payment_by_id("other".to_string())
            .await
            .unwrap();
        assert_eq!(other.method, crate::PaymentMethod::Spark);
    }
}
