use flashnet::FlashnetError;

use crate::SdkError;

/// Error type for conversion operations
#[derive(Debug, thiserror::Error)]
pub enum ConversionError {
    #[error("No conversion pools available")]
    NoPoolsAvailable,
    #[error("Conversion failed: {0}")]
    ConversionFailed(String),
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

impl From<FlashnetError> for ConversionError {
    fn from(e: FlashnetError) -> Self {
        ConversionError::ConversionFailed(e.to_string())
    }
}
