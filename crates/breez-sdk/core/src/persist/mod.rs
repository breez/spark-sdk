mod sqlite;

use std::sync::Arc;

use serde::{Deserialize, Serialize};
pub use sqlite::SqliteStorage;
use thiserror::Error;

use crate::{DepositClaimError, DepositInfo, LnurlPayInfo, models::Payment};

const ACCOUNT_INFO_KEY: &str = "account_info";
const SYNC_OFFSET_KEY: &str = "sync_offset";
const TX_CACHE_KEY: &str = "tx_cache";

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

/// Errors that can occur during storage operations
#[derive(Debug, Error, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Error))]
pub enum StorageError {
    /// `SQLite` error
    #[error("Underline implementation error: {0}")]
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

/// Metadata associated with a payment that cannot be extracted from the Spark operator.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PaymentMetadata {
    pub lnurl_pay_info: Option<LnurlPayInfo>,
}

/// Trait for persistent storage
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
pub trait Storage: Send + Sync {
    fn get_cached_item(&self, key: String) -> Result<Option<String>, StorageError>;
    fn set_cached_item(&self, key: String, value: String) -> Result<(), StorageError>;
    /// Lists payments with pagination
    ///
    /// # Arguments
    ///
    /// * `offset` - Number of records to skip
    /// * `limit` - Maximum number of records to return
    ///
    /// # Returns
    ///
    /// A vector of payments or a `StorageError`
    fn list_payments(
        &self,
        offset: Option<u32>,
        limit: Option<u32>,
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
    fn insert_payment(&self, payment: Payment) -> Result<(), StorageError>;

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
    fn set_payment_metadata(
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
    fn get_payment_by_id(&self, id: String) -> Result<Payment, StorageError>;

    /// Add deposit in storage
    /// # Arguments
    ///
    /// * `txid` - The transaction ID of the deposit
    /// * `vout` - The output index of the deposit
    /// * `amount_sats` - The amount of the deposit in sats
    ///
    /// # Returns
    ///
    /// Success or a `StorageError`
    fn add_deposit(&self, txid: String, vout: u32, amount_sats: u64) -> Result<(), StorageError>;

    /// Removes an unclaimed deposit from storage
    /// # Arguments
    ///
    /// * `txid` - The transaction ID of the deposit
    /// * `vout` - The output index of the deposit
    ///
    /// # Returns
    ///
    /// Success or a `StorageError`
    fn delete_deposit(&self, txid: String, vout: u32) -> Result<(), StorageError>;

    /// Lists all unclaimed deposits from storage
    /// # Returns
    ///
    /// A vector of `DepositInfo` or a `StorageError`
    fn list_deposits(&self) -> Result<Vec<DepositInfo>, StorageError>;

    /// Updates or inserts refund transaction details for a deposit
    /// # Arguments
    ///
    /// * `deposit_refund` - The refund information to store
    ///
    /// # Returns
    ///
    /// Success or a `StorageError`
    fn update_deposit(
        &self,
        txid: String,
        vout: u32,
        deposit_refund: UpdateDepositPayload,
    ) -> Result<(), StorageError>;
}

pub(crate) struct ObjectCacheRepository {
    storage: Arc<dyn Storage>,
}

impl ObjectCacheRepository {
    pub(crate) fn new(storage: Arc<dyn Storage>) -> Self {
        ObjectCacheRepository { storage }
    }

    pub(crate) fn save_account_info(&self, value: &CachedAccountInfo) -> Result<(), StorageError> {
        self.storage
            .set_cached_item(ACCOUNT_INFO_KEY.to_string(), serde_json::to_string(value)?)?;
        Ok(())
    }

    pub(crate) fn fetch_account_info(&self) -> Result<Option<CachedAccountInfo>, StorageError> {
        let value = self.storage.get_cached_item(ACCOUNT_INFO_KEY.to_string())?;
        match value {
            Some(value) => Ok(Some(serde_json::from_str(&value)?)),
            None => Ok(None),
        }
    }

    pub(crate) fn save_sync_info(&self, value: &CachedSyncInfo) -> Result<(), StorageError> {
        self.storage
            .set_cached_item(SYNC_OFFSET_KEY.to_string(), serde_json::to_string(value)?)?;
        Ok(())
    }

    pub(crate) fn fetch_sync_info(&self) -> Result<Option<CachedSyncInfo>, StorageError> {
        let value = self.storage.get_cached_item(SYNC_OFFSET_KEY.to_string())?;
        match value {
            Some(value) => Ok(Some(serde_json::from_str(&value)?)),
            None => Ok(None),
        }
    }

    pub(crate) fn save_tx(&self, txid: &str, value: &CachedTx) -> Result<(), StorageError> {
        self.storage.set_cached_item(
            format!("{TX_CACHE_KEY}-{txid}"),
            serde_json::to_string(value)?,
        )?;
        Ok(())
    }

    pub(crate) fn fetch_tx(&self, txid: &str) -> Result<Option<CachedTx>, StorageError> {
        let value = self
            .storage
            .get_cached_item(format!("{TX_CACHE_KEY}-{txid}"))?;
        match value {
            Some(value) => Ok(Some(serde_json::from_str(&value)?)),
            None => Ok(None),
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
pub(crate) struct CachedAccountInfo {
    pub(crate) balance_sats: u64,
}

#[derive(Serialize, Deserialize, Default)]
pub(crate) struct CachedSyncInfo {
    pub(crate) offset: u64,
}

#[derive(Serialize, Deserialize, Default)]
pub(crate) struct CachedTx {
    pub(crate) raw_tx: String,
}
