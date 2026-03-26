use flashnet::FlashnetError;

use crate::SdkError;

/// Error type for conversion operations
#[derive(Debug, thiserror::Error)]
pub enum ConversionError {
    #[error("No conversion pools available")]
    NoPoolsAvailable,
    #[error("Conversion failed: {0}")]
    ConversionFailed(String),
    #[error("Duplicate transfer: conversion already handled")]
    DuplicateTransfer,
    #[error("Validation failed: {0}")]
    ValidationFailed(String),
    #[error("Refund failed: {0}")]
    RefundFailed(String),
    #[error("SDK error: {0}")]
    Sdk(#[from] SdkError),
    #[error("Storage error: {0}")]
    Storage(#[from] crate::persist::StorageError),
    #[error("Wallet error: {0}")]
    Wallet(#[from] spark_wallet::SparkWalletError),
}

impl ConversionError {
    /// Returns `true` if this error indicates the transfer was already handled
    /// by another instance (duplicate deterministic transfer ID).
    pub fn is_duplicate_transfer(&self) -> bool {
        matches!(self, ConversionError::DuplicateTransfer)
    }
}

impl From<FlashnetError> for ConversionError {
    fn from(e: FlashnetError) -> Self {
        // Detect duplicate transfer from HTTP 409 Conflict
        if let FlashnetError::Network {
            code: Some(409), ..
        } = &e
        {
            return ConversionError::DuplicateTransfer;
        }
        ConversionError::ConversionFailed(e.to_string())
    }
}
