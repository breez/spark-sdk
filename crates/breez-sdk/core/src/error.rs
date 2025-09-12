use crate::{
    Fee,
    persist::{self},
};
use bitcoin::consensus::encode::FromHexError;
use serde::{Deserialize, Serialize};
use spark_wallet::SparkWalletError;
use std::{convert::Infallible, num::TryFromIntError};
use thiserror::Error;

/// Error type for the `BreezSdk`
#[derive(Debug, Error, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Error))]
pub enum SdkError {
    #[error("SparkSdkError: {0}")]
    SparkError(String),

    #[error("Invalid UUID: {0}")]
    InvalidUuid(String),

    /// Invalid input error
    #[error("Invalid input: {0}")]
    InvalidInput(String),

    /// Network error
    #[error("Network error: {0}")]
    NetworkError(String),

    /// Storage error
    #[error("Storage error: {0}")]
    StorageError(String),

    #[error("Chain service error: {0}")]
    ChainServiceError(String),

    #[error(
        "Deposit claim fee exceeds for utxo: {tx}:{vout} with max fee: {max_fee} and actual fee sat: {actual_fee}"
    )]
    DepositClaimFeeExceeded {
        tx: String,
        vout: u32,
        max_fee: Fee,
        actual_fee: u64,
    },

    #[error("Missing utxo: {tx}:{vout}")]
    MissingUtxo { tx: String, vout: u32 },

    #[error("Lnurl error: {0}")]
    LnurlError(String),

    #[error("SparkScan error: {0}")]
    SparkScanApiError(String),

    #[error("Error: {0}")]
    Generic(String),
}

impl From<crate::chain::ChainServiceError> for SdkError {
    fn from(e: crate::chain::ChainServiceError) -> Self {
        SdkError::ChainServiceError(e.to_string())
    }
}

impl From<breez_sdk_common::lnurl::error::LnurlError> for SdkError {
    fn from(e: breez_sdk_common::lnurl::error::LnurlError) -> Self {
        SdkError::LnurlError(e.to_string())
    }
}

impl From<breez_sdk_common::input::ParseError> for SdkError {
    fn from(e: breez_sdk_common::input::ParseError) -> Self {
        SdkError::InvalidInput(e.to_string())
    }
}

impl From<bitcoin::address::ParseError> for SdkError {
    fn from(e: bitcoin::address::ParseError) -> Self {
        SdkError::InvalidInput(e.to_string())
    }
}

impl From<persist::StorageError> for SdkError {
    fn from(e: persist::StorageError) -> Self {
        SdkError::StorageError(e.to_string())
    }
}

impl From<Infallible> for SdkError {
    fn from(value: Infallible) -> Self {
        SdkError::Generic(value.to_string())
    }
}

impl From<String> for SdkError {
    fn from(s: String) -> Self {
        Self::Generic(s)
    }
}

impl From<&str> for SdkError {
    fn from(s: &str) -> Self {
        Self::Generic(s.to_string())
    }
}

impl From<TryFromIntError> for SdkError {
    fn from(e: TryFromIntError) -> Self {
        SdkError::Generic(e.to_string())
    }
}

impl From<serde_json::Error> for SdkError {
    fn from(e: serde_json::Error) -> Self {
        SdkError::Generic(e.to_string())
    }
}

impl From<SparkWalletError> for SdkError {
    fn from(e: SparkWalletError) -> Self {
        SdkError::SparkError(e.to_string())
    }
}

impl From<FromHexError> for SdkError {
    fn from(e: FromHexError) -> Self {
        SdkError::Generic(e.to_string())
    }
}

impl From<uuid::Error> for SdkError {
    fn from(e: uuid::Error) -> Self {
        SdkError::InvalidUuid(e.to_string())
    }
}

impl From<sparkscan::Error<sparkscan::types::HttpValidationError>> for SdkError {
    fn from(e: sparkscan::Error<sparkscan::types::HttpValidationError>) -> Self {
        SdkError::SparkScanApiError(format!("{e:?}"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Error, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum DepositClaimError {
    #[error(
        "Deposit claim fee exceeds for utxo: {tx}:{vout} with max fee: {max_fee} and actual fee sat: {actual_fee}"
    )]
    DepositClaimFeeExceeded {
        tx: String,
        vout: u32,
        max_fee: Fee,
        actual_fee: u64,
    },

    #[error("Missing utxo: {tx}:{vout}")]
    MissingUtxo { tx: String, vout: u32 },

    #[error("Generic error: {message}")]
    Generic { message: String },
}

impl From<SdkError> for DepositClaimError {
    fn from(value: SdkError) -> Self {
        match value {
            SdkError::DepositClaimFeeExceeded {
                tx,
                vout,
                max_fee,
                actual_fee,
            } => DepositClaimError::DepositClaimFeeExceeded {
                tx,
                vout,
                max_fee,
                actual_fee,
            },
            SdkError::MissingUtxo { tx, vout } => DepositClaimError::MissingUtxo { tx, vout },
            SdkError::Generic(e) => DepositClaimError::Generic { message: e },
            _ => DepositClaimError::Generic {
                message: value.to_string(),
            },
        }
    }
}
