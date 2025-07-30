use crate::persist::{self};
use bitcoin::address::ParseError;
use spark_wallet::SparkWalletError;
use std::{convert::Infallible, num::TryFromIntError};
use thiserror::Error;

/// Error type for the `BreezSdk`
#[derive(Debug, Error)]
pub enum SdkError {
    #[error("SparkSdkError: {0}")]
    SparkError(#[from] SparkWalletError),

    #[error("Invalid UUID: {0}")]
    InvalidUuid(#[from] uuid::Error),
    /// Invalid input error
    #[error("Invalid input: {0}")]
    InvalidInput(String),

    /// Network error
    #[error("Network error: {network_err}")]
    NetworkError { network_err: String },

    /// Storage error
    #[error("Storage error: {0}")]
    StorageError(#[from] persist::StorageError),

    #[error("Parse error: {0}")]
    ParseError(#[from] breez_sdk_common::input::ParseError),

    #[error("General error: {0}")]
    GenericError(String),
}

impl From<ParseError> for SdkError {
    fn from(e: ParseError) -> Self {
        SdkError::InvalidInput(e.to_string())
    }
}

impl From<Infallible> for SdkError {
    fn from(value: Infallible) -> Self {
        SdkError::GenericError(value.to_string())
    }
}

impl From<String> for SdkError {
    fn from(s: String) -> Self {
        Self::GenericError(s)
    }
}

impl From<&str> for SdkError {
    fn from(s: &str) -> Self {
        Self::GenericError(s.to_string())
    }
}

impl From<TryFromIntError> for SdkError {
    fn from(e: TryFromIntError) -> Self {
        SdkError::GenericError(e.to_string())
    }
}

impl From<serde_json::Error> for SdkError {
    fn from(e: serde_json::Error) -> Self {
        SdkError::GenericError(e.to_string())
    }
}
