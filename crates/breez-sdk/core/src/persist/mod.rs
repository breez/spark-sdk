mod sqlite;

use serde::{Deserialize, Serialize};
pub use sqlite::SqliteStorage;
use thiserror::Error;

use crate::models::Payment;

/// Errors that can occur during storage operations
#[derive(Debug, Error)]
pub enum StorageError {
    /// `SQLite` error
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// Database initialization error
    #[error("Failed to initialize database: {0}")]
    InitializationError(String),

    #[error("Failed to serialize/deserialize data: {0}")]
    Serialization(#[from] serde_json::Error),
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

#[derive(Serialize, Deserialize)]
pub(crate) struct CachedAccountInfo {
    pub(crate) balance_sats: u64,
}

impl CachedAccountInfo {
    pub(crate) fn save(&self, storage: &dyn Storage) -> Result<(), StorageError> {
        storage.set_cached_item("account_info", serde_json::to_string(self)?)?;
        Ok(())
    }
    pub(crate) fn fetch(storage: &dyn Storage) -> Result<Self, StorageError> {
        let account_info = storage.get_cached_item("account_info")?;
        match account_info {
            Some(account_info) => Ok(serde_json::from_str(&account_info)?),
            None => Ok(CachedAccountInfo { balance_sats: 0 }),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct CachedSyncInfo {
    pub(crate) offset: u64,
}

impl CachedSyncInfo {
    pub(crate) fn save(&self, storage: &dyn Storage) -> Result<(), StorageError> {
        storage.set_cached_item("sync_offset", serde_json::to_string(self)?)?;
        Ok(())
    }
    pub(crate) fn fetch(storage: &dyn Storage) -> Result<Self, StorageError> {
        let offset = storage.get_cached_item("sync_offset")?;
        match offset {
            Some(offset) => Ok(serde_json::from_str(&offset)?),
            None => Ok(CachedSyncInfo { offset: 0 }),
        }
    }
}
