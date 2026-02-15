pub(crate) mod path;
#[cfg(all(
    feature = "postgres",
    not(all(target_family = "wasm", target_os = "unknown"))
))]
pub mod postgres;
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub(crate) mod sqlite;

use std::{collections::HashMap, sync::Arc};

use macros::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    ConversionInfo, DepositClaimError, DepositInfo, LightningAddressInfo, ListPaymentsRequest,
    LnurlPayInfo, LnurlWithdrawInfo, TokenBalance, TokenMetadata,
    models::Payment,
    sync_storage::{IncomingChange, OutgoingChange, Record, UnversionedRecordChange},
};

const ACCOUNT_INFO_KEY: &str = "account_info";
const LAST_SYNC_TIME_KEY: &str = "last_sync_time";
const LIGHTNING_ADDRESS_KEY: &str = "lightning_address";
const LNURL_METADATA_UPDATED_AFTER_KEY: &str = "lnurl_metadata_updated_after";
const SYNC_OFFSET_KEY: &str = "sync_offset";
const TX_CACHE_KEY: &str = "tx_cache";
const STATIC_DEPOSIT_ADDRESS_CACHE_KEY: &str = "static_deposit_address";
const TOKEN_METADATA_KEY_PREFIX: &str = "token_metadata_";
const PAYMENT_METADATA_KEY_PREFIX: &str = "payment_metadata";
const SPARK_PRIVATE_MODE_INITIALIZED_KEY: &str = "spark_private_mode_initialized";

#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum UpdateDepositPayload {
    ClaimError {
        error: DepositClaimError,
    },
    Refund {
        refund_txid: String,
        refund_tx: String,
    },
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SetLnurlMetadataItem {
    pub payment_hash: String,
    pub sender_comment: Option<String>,
    pub nostr_zap_request: Option<String>,
    pub nostr_zap_receipt: Option<String>,
}

impl From<lnurl_models::ListMetadataMetadata> for SetLnurlMetadataItem {
    fn from(value: lnurl_models::ListMetadataMetadata) -> Self {
        SetLnurlMetadataItem {
            payment_hash: value.payment_hash,
            sender_comment: value.sender_comment,
            nostr_zap_request: value.nostr_zap_request,
            nostr_zap_receipt: value.nostr_zap_receipt,
        }
    }
}

/// Errors that can occur during storage operations
#[derive(Debug, Error, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Error))]
pub enum StorageError {
    /// Connection-related errors (pool exhaustion, timeouts, connection refused).
    /// These are often transient and may be retried.
    #[error("Connection error: {0}")]
    Connection(String),

    #[error("Underlying implementation error: {0}")]
    Implementation(String),

    /// Database initialization error
    #[error("Failed to initialize database: {0}")]
    InitializationError(String),

    #[error("Failed to serialize/deserialize data: {0}")]
    Serialization(String),
}

impl From<serde_json::Error> for StorageError {
    fn from(e: serde_json::Error) -> Self {
        StorageError::Serialization(e.to_string())
    }
}

impl From<std::num::TryFromIntError> for StorageError {
    fn from(e: std::num::TryFromIntError) -> Self {
        StorageError::Implementation(format!("integer overflow: {e}"))
    }
}

/// Metadata associated with a payment that cannot be extracted from the Spark operator.
#[derive(Clone, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PaymentMetadata {
    pub parent_payment_id: Option<String>,
    pub lnurl_pay_info: Option<LnurlPayInfo>,
    pub lnurl_withdraw_info: Option<LnurlWithdrawInfo>,
    pub lnurl_description: Option<String>,
    pub conversion_info: Option<ConversionInfo>,
}

/// Trait for persistent storage
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
#[async_trait]
pub trait Storage: Send + Sync {
    async fn delete_cached_item(&self, key: String) -> Result<(), StorageError>;
    async fn get_cached_item(&self, key: String) -> Result<Option<String>, StorageError>;
    async fn set_cached_item(&self, key: String, value: String) -> Result<(), StorageError>;
    /// Lists payments with optional filters and pagination
    ///
    /// # Arguments
    ///
    /// * `list_payments_request` - The request to list payments
    ///
    /// # Returns
    ///
    /// A vector of payments or a `StorageError`
    async fn list_payments(
        &self,
        request: ListPaymentsRequest,
    ) -> Result<Vec<Payment>, StorageError>;

    /// Inserts a payment into storage
    ///
    /// # Arguments
    ///
    /// * `payment` - The payment to insert
    ///
    /// # Returns
    ///
    /// Success or a `StorageError`
    async fn insert_payment(&self, payment: Payment) -> Result<(), StorageError>;

    /// Inserts payment metadata into storage
    ///
    /// # Arguments
    ///
    /// * `payment_id` - The ID of the payment
    /// * `metadata` - The metadata to insert
    ///
    /// # Returns
    ///
    /// Success or a `StorageError`
    async fn insert_payment_metadata(
        &self,
        payment_id: String,
        metadata: PaymentMetadata,
    ) -> Result<(), StorageError>;

    /// Gets a payment by its ID
    /// # Arguments
    ///
    /// * `id` - The ID of the payment to retrieve
    ///
    /// # Returns
    ///
    /// The payment if found or None if not found
    async fn get_payment_by_id(&self, id: String) -> Result<Payment, StorageError>;

    /// Gets a payment by its invoice
    /// # Arguments
    ///
    /// * `invoice` - The invoice of the payment to retrieve
    /// # Returns
    ///
    /// The payment if found or None if not found
    async fn get_payment_by_invoice(
        &self,
        invoice: String,
    ) -> Result<Option<Payment>, StorageError>;

    /// Gets payments that have any of the specified parent payment IDs.
    /// Used to load related payments for a set of parent payments.
    ///
    /// # Arguments
    ///
    /// * `parent_payment_ids` - The IDs of the parent payments
    ///
    /// # Returns
    ///
    /// A map of `parent_payment_id` -> Vec<Payment> or a `StorageError`
    async fn get_payments_by_parent_ids(
        &self,
        parent_payment_ids: Vec<String>,
    ) -> Result<HashMap<String, Vec<Payment>>, StorageError>;

    /// Add a deposit to storage
    /// # Arguments
    ///
    /// * `txid` - The transaction ID of the deposit
    /// * `vout` - The output index of the deposit
    /// * `amount_sats` - The amount of the deposit in sats
    ///
    /// # Returns
    ///
    /// Success or a `StorageError`
    async fn add_deposit(
        &self,
        txid: String,
        vout: u32,
        amount_sats: u64,
    ) -> Result<(), StorageError>;

    /// Removes an unclaimed deposit from storage
    /// # Arguments
    ///
    /// * `txid` - The transaction ID of the deposit
    /// * `vout` - The output index of the deposit
    ///
    /// # Returns
    ///
    /// Success or a `StorageError`
    async fn delete_deposit(&self, txid: String, vout: u32) -> Result<(), StorageError>;

    /// Lists all unclaimed deposits from storage
    /// # Returns
    ///
    /// A vector of `DepositInfo` or a `StorageError`
    async fn list_deposits(&self) -> Result<Vec<DepositInfo>, StorageError>;

    /// Updates or inserts unclaimed deposit details
    /// # Arguments
    ///
    /// * `txid` - The transaction ID of the deposit
    /// * `vout` - The output index of the deposit
    /// * `payload` - The payload for the update
    ///
    /// # Returns
    ///
    /// Success or a `StorageError`
    async fn update_deposit(
        &self,
        txid: String,
        vout: u32,
        payload: UpdateDepositPayload,
    ) -> Result<(), StorageError>;

    async fn set_lnurl_metadata(
        &self,
        metadata: Vec<SetLnurlMetadataItem>,
    ) -> Result<(), StorageError>;

    // Sync storage methods
    async fn add_outgoing_change(
        &self,
        record: UnversionedRecordChange,
    ) -> Result<u64, StorageError>;
    async fn complete_outgoing_sync(
        &self,
        record: Record,
        local_revision: u64,
    ) -> Result<(), StorageError>;
    async fn get_pending_outgoing_changes(
        &self,
        limit: u32,
    ) -> Result<Vec<OutgoingChange>, StorageError>;

    /// Get the last committed sync revision.
    ///
    /// The `sync_revision` table tracks the highest revision that has been committed
    /// (i.e. acknowledged by the server or received from it). It does NOT include
    /// pending outgoing queue ids. This value is used by the sync protocol to
    /// request changes from the server.
    async fn get_last_revision(&self) -> Result<u64, StorageError>;

    /// Insert incoming records from remote sync
    async fn insert_incoming_records(&self, records: Vec<Record>) -> Result<(), StorageError>;

    /// Delete an incoming record after it has been processed
    async fn delete_incoming_record(&self, record: Record) -> Result<(), StorageError>;

    /// Get incoming records that need to be processed, up to the specified limit
    async fn get_incoming_records(&self, limit: u32) -> Result<Vec<IncomingChange>, StorageError>;

    /// Get the latest outgoing record if any exists
    async fn get_latest_outgoing_change(&self) -> Result<Option<OutgoingChange>, StorageError>;

    /// Update the sync state record from an incoming record
    async fn update_record_from_incoming(&self, record: Record) -> Result<(), StorageError>;
}

pub(crate) struct ObjectCacheRepository {
    storage: Arc<dyn Storage>,
}

impl ObjectCacheRepository {
    pub(crate) fn new(storage: Arc<dyn Storage>) -> Self {
        ObjectCacheRepository { storage }
    }

    pub(crate) async fn save_account_info(
        &self,
        value: &CachedAccountInfo,
    ) -> Result<(), StorageError> {
        self.storage
            .set_cached_item(ACCOUNT_INFO_KEY.to_string(), serde_json::to_string(value)?)
            .await?;
        Ok(())
    }

    pub(crate) async fn fetch_account_info(
        &self,
    ) -> Result<Option<CachedAccountInfo>, StorageError> {
        let value = self
            .storage
            .get_cached_item(ACCOUNT_INFO_KEY.to_string())
            .await?;
        match value {
            Some(value) => Ok(Some(serde_json::from_str(&value)?)),
            None => Ok(None),
        }
    }

    pub(crate) async fn save_sync_info(&self, value: &CachedSyncInfo) -> Result<(), StorageError> {
        self.storage
            .set_cached_item(SYNC_OFFSET_KEY.to_string(), serde_json::to_string(value)?)
            .await?;
        Ok(())
    }

    pub(crate) async fn fetch_sync_info(&self) -> Result<Option<CachedSyncInfo>, StorageError> {
        let value = self
            .storage
            .get_cached_item(SYNC_OFFSET_KEY.to_string())
            .await?;
        match value {
            Some(value) => Ok(Some(serde_json::from_str(&value)?)),
            None => Ok(None),
        }
    }

    pub(crate) async fn save_tx(&self, txid: &str, value: &CachedTx) -> Result<(), StorageError> {
        self.storage
            .set_cached_item(
                format!("{TX_CACHE_KEY}-{txid}"),
                serde_json::to_string(value)?,
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn fetch_tx(&self, txid: &str) -> Result<Option<CachedTx>, StorageError> {
        let value = self
            .storage
            .get_cached_item(format!("{TX_CACHE_KEY}-{txid}"))
            .await?;
        match value {
            Some(value) => Ok(Some(serde_json::from_str(&value)?)),
            None => Ok(None),
        }
    }

    pub(crate) async fn save_static_deposit_address(
        &self,
        value: &StaticDepositAddress,
    ) -> Result<(), StorageError> {
        self.storage
            .set_cached_item(
                STATIC_DEPOSIT_ADDRESS_CACHE_KEY.to_string(),
                serde_json::to_string(value)?,
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn fetch_static_deposit_address(
        &self,
    ) -> Result<Option<StaticDepositAddress>, StorageError> {
        let value = self
            .storage
            .get_cached_item(STATIC_DEPOSIT_ADDRESS_CACHE_KEY.to_string())
            .await?;
        match value {
            Some(value) => Ok(Some(serde_json::from_str(&value)?)),
            None => Ok(None),
        }
    }

    pub(crate) async fn save_lightning_address(
        &self,
        value: &LightningAddressInfo,
    ) -> Result<(), StorageError> {
        self.storage
            .set_cached_item(
                LIGHTNING_ADDRESS_KEY.to_string(),
                serde_json::to_string(value)?,
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn delete_lightning_address(&self) -> Result<(), StorageError> {
        self.storage
            .delete_cached_item(LIGHTNING_ADDRESS_KEY.to_string())
            .await?;
        Ok(())
    }

    pub(crate) async fn fetch_lightning_address(
        &self,
    ) -> Result<Option<LightningAddressInfo>, StorageError> {
        let value = self
            .storage
            .get_cached_item(LIGHTNING_ADDRESS_KEY.to_string())
            .await?;
        match value {
            Some(value) => Ok(Some(serde_json::from_str(&value)?)),
            None => Ok(None),
        }
    }

    pub(crate) async fn save_token_metadata(
        &self,
        value: &TokenMetadata,
    ) -> Result<(), StorageError> {
        self.storage
            .set_cached_item(
                format!("{TOKEN_METADATA_KEY_PREFIX}{}", value.identifier),
                serde_json::to_string(value)?,
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn fetch_token_metadata(
        &self,
        identifier: &str,
    ) -> Result<Option<TokenMetadata>, StorageError> {
        let value = self
            .storage
            .get_cached_item(format!("{TOKEN_METADATA_KEY_PREFIX}{identifier}"))
            .await?;
        match value {
            Some(value) => Ok(Some(serde_json::from_str(&value)?)),
            None => Ok(None),
        }
    }

    pub(crate) async fn save_payment_metadata(
        &self,
        identifier: &str,
        value: &PaymentMetadata,
    ) -> Result<(), StorageError> {
        self.storage
            .set_cached_item(
                format!("{PAYMENT_METADATA_KEY_PREFIX}-{identifier}"),
                serde_json::to_string(value)?,
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn fetch_payment_metadata(
        &self,
        identifier: &str,
    ) -> Result<Option<PaymentMetadata>, StorageError> {
        let value = self
            .storage
            .get_cached_item(format!("{PAYMENT_METADATA_KEY_PREFIX}-{identifier}",))
            .await?;
        match value {
            Some(value) => Ok(Some(serde_json::from_str(&value)?)),
            None => Ok(None),
        }
    }

    pub(crate) async fn delete_payment_metadata(
        &self,
        identifier: &str,
    ) -> Result<(), StorageError> {
        self.storage
            .delete_cached_item(format!("{PAYMENT_METADATA_KEY_PREFIX}-{identifier}",))
            .await?;
        Ok(())
    }

    pub(crate) async fn save_spark_private_mode_initialized(&self) -> Result<(), StorageError> {
        self.storage
            .set_cached_item(
                SPARK_PRIVATE_MODE_INITIALIZED_KEY.to_string(),
                "true".to_string(),
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn fetch_spark_private_mode_initialized(&self) -> Result<bool, StorageError> {
        let value = self
            .storage
            .get_cached_item(SPARK_PRIVATE_MODE_INITIALIZED_KEY.to_string())
            .await?;
        match value {
            Some(value) => Ok(value == "true"),
            None => Ok(false),
        }
    }

    pub(crate) async fn save_lnurl_metadata_updated_after(
        &self,
        offset: i64,
    ) -> Result<(), StorageError> {
        self.storage
            .set_cached_item(
                LNURL_METADATA_UPDATED_AFTER_KEY.to_string(),
                offset.to_string(),
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn fetch_lnurl_metadata_updated_after(&self) -> Result<i64, StorageError> {
        let value = self
            .storage
            .get_cached_item(LNURL_METADATA_UPDATED_AFTER_KEY.to_string())
            .await?;
        match value {
            Some(value) => Ok(value.parse().map_err(|_| {
                StorageError::Serialization("invalid lnurl_metadata_updated_after".to_string())
            })?),
            None => Ok(0),
        }
    }

    pub(crate) async fn get_last_sync_time(&self) -> Result<Option<u64>, StorageError> {
        let value = self
            .storage
            .get_cached_item(LAST_SYNC_TIME_KEY.to_string())
            .await?;
        match value {
            Some(v) => Ok(Some(v.parse().map_err(|_| {
                StorageError::Serialization("invalid last_sync_time".to_string())
            })?)),
            None => Ok(None),
        }
    }

    pub(crate) async fn set_last_sync_time(&self, time: u64) -> Result<(), StorageError> {
        self.storage
            .set_cached_item(LAST_SYNC_TIME_KEY.to_string(), time.to_string())
            .await
    }
}

#[derive(Serialize, Deserialize, Default)]
pub(crate) struct CachedAccountInfo {
    pub(crate) balance_sats: u64,
    #[serde(default)]
    pub(crate) token_balances: HashMap<String, TokenBalance>,
}

#[derive(Serialize, Deserialize, Default)]
pub(crate) struct CachedSyncInfo {
    pub(crate) offset: u64,
    pub(crate) last_synced_final_token_payment_id: Option<String>,
}

#[derive(Serialize, Deserialize, Default)]
pub(crate) struct CachedTx {
    pub(crate) raw_tx: String,
}

#[derive(Serialize, Deserialize, Default)]
pub(crate) struct StaticDepositAddress {
    pub(crate) address: String,
}

#[cfg(feature = "test-utils")]
pub mod tests;
