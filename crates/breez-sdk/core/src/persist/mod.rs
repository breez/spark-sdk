mod sqlite;

use std::sync::Arc;

use serde::{Deserialize, Serialize};
pub use sqlite::SqliteStorage;
use thiserror::Error;

use crate::{DepositInfo, models::Payment};

/// Errors that can occur during storage operations
#[derive(Debug, Error, Clone)]
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

/// Trait for persistent storage
pub trait Storage: Send + Sync {
    fn get_cached_item(&self, key: &str) -> Result<Option<String>, StorageError>;
    fn set_cached_item(&self, key: &str, value: String) -> Result<(), StorageError>;
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
    fn insert_payment(&self, payment: &Payment) -> Result<(), StorageError>;

    /// Gets a payment by its ID
    /// # Arguments
    ///
    /// * `id` - The ID of the payment to retrieve
    ///
    /// # Returns
    ///
    /// The payment if found or None if not found
    fn get_payment_by_id(&self, id: &str) -> Result<Payment, StorageError>;
}

pub(crate) struct ObjectCacheRepository {
    storage: Arc<dyn Storage>,
}

impl ObjectCacheRepository {
    pub(crate) fn new(storage: Arc<dyn Storage>) -> Self {
        ObjectCacheRepository { storage }
    }

    pub(crate) fn save_account_info(&self, value: CachedAccountInfo) -> Result<(), StorageError> {
        self.storage
            .set_cached_item("account_info", serde_json::to_string(&value)?)?;
        Ok(())
    }

    pub(crate) fn fetch_account_info(&self) -> Result<Option<CachedAccountInfo>, StorageError> {
        let value = self.storage.get_cached_item("account_info")?;
        match value {
            Some(value) => Ok(Some(serde_json::from_str(&value)?)),
            None => Ok(None),
        }
    }

    pub(crate) fn save_sync_info(&self, value: CachedSyncInfo) -> Result<(), StorageError> {
        self.storage
            .set_cached_item("sync_offset", serde_json::to_string(&value)?)?;
        Ok(())
    }

    pub(crate) fn fetch_sync_info(&self) -> Result<Option<CachedSyncInfo>, StorageError> {
        let value = self.storage.get_cached_item("sync_offset")?;
        match value {
            Some(value) => Ok(Some(serde_json::from_str(&value)?)),
            None => Ok(None),
        }
    }

    pub(crate) fn save_unclaimed_deposits(
        &self,
        value: &Vec<DepositInfo>,
    ) -> Result<(), StorageError> {
        self.storage
            .set_cached_item("unclaimed_deposits", serde_json::to_string(value)?)?;
        Ok(())
    }

    pub(crate) fn fetch_unclaimed_deposits(
        &self,
    ) -> Result<Option<Vec<DepositInfo>>, StorageError> {
        let value = self.storage.get_cached_item("unclaimed_deposits")?;
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
