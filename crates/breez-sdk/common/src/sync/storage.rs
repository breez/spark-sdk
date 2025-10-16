use thiserror::Error;

use crate::sync::{model::Record, OutgoingRecord, UnversionedOutgoingRecord};

/// Errors that can occur during storage operations
#[derive(Debug, Error, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Error))]
pub enum StorageError {
    #[error("Underline implementation error: {0}")]
    Implementation(String),

    /// Database initialization error
    #[error("Failed to initialize database: {0}")]
    InitializationError(String),

    #[error("Failed to serialize/deserialize data: {0}")]
    Serialization(String),
}

#[macros::async_trait]
pub trait SyncStorage: Send + Sync {
    async fn add_outgoing_record(&self, record: &UnversionedOutgoingRecord) -> Result<u32, StorageError>;
    async fn complete_outgoing_sync(&self, record: &OutgoingRecord) -> Result<(), StorageError>;
    async fn get_pending_outgoing_records(&self, limit: usize) -> Result<Vec<OutgoingRecord>, StorageError>;
}