use crate::{
    Fee,
    persist::{self},
};
use bitcoin::{address::ParseError, consensus::encode::FromHexError};
use serde::{Deserialize, Serialize};
use spark_wallet::SparkWalletError;
use std::{convert::Infallible, num::TryFromIntError};
use thiserror::Error;

/// Error type for the `BreezSdk`
#[derive(Debug, Error, Clone)]
pub enum SdkError {
    #[error("SparkSdkError: {0}")]
    SparkError(#[from] SparkWalletError),

    #[error("Invalid UUID: {0}")]
    InvalidUuid(#[from] uuid::Error),

    /// Invalid input error
    #[error("Invalid input: {0}")]
    InvalidInput(String),

    /// Network error
    #[error("Network error: {0}")]
    NetworkError(String),

    /// Storage error
    #[error("Storage error: {0}")]
    StorageError(#[from] persist::StorageError),

    #[error("Parse error: {0}")]
    ParseError(#[from] breez_sdk_common::input::ParseError),

    #[error("Chain service error: {0}")]
    ChainServiceError(#[from] crate::chain::ChainServiceError),

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

    #[error("lnurl error: {0}")]
    LnurlError(#[from] breez_sdk_common::lnurl::error::LnurlError),

    #[error("General error: {0}")]
    GenericError(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, Error)]
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

    #[error("Generic error: {0}")]
    Generic(String),
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
            SdkError::GenericError(e) => DepositClaimError::Generic(e),
            _ => DepositClaimError::Generic(value.to_string()),
        }
    }
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

impl From<FromHexError> for SdkError {
    fn from(e: FromHexError) -> Self {
        SdkError::GenericError(e.to_string())
    }
}
