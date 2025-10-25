use crate::{
    Fee,
    lnurl::{LnurlServerError, ReqwestLnurlServerClientError},
    persist::{self},
};
use bitcoin::consensus::encode::FromHexError;
use breez_sdk_common::error::ServiceConnectivityError;
use serde::{Deserialize, Serialize};
use spark_wallet::SparkWalletError;
use std::{convert::Infallible, fmt::Display, num::TryFromIntError};
use thiserror::Error;
use tracing::error;
use tracing_subscriber::util::TryInitError;

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

    #[error("Error: {0}")]
    Generic(String),
}

impl SdkError {
    pub fn generic(s: impl Display) -> Self {
        error!("Generic error: {s}");
        SdkError::Generic(s.to_string())
    }

    pub fn invalid_input(s: impl Display) -> Self {
        error!("Invalid input error: {s}");
        SdkError::InvalidInput(s.to_string())
    }
}

impl From<crate::chain::ChainServiceError> for SdkError {
    fn from(e: crate::chain::ChainServiceError) -> Self {
        error!("Chain service error: {e}");
        SdkError::ChainServiceError(e.to_string())
    }
}

impl From<breez_sdk_common::lnurl::error::LnurlError> for SdkError {
    fn from(e: breez_sdk_common::lnurl::error::LnurlError) -> Self {
        error!("Lnurl error: {e}");
        SdkError::LnurlError(e.to_string())
    }
}

impl From<breez_sdk_common::input::ParseError> for SdkError {
    fn from(e: breez_sdk_common::input::ParseError) -> Self {
        error!("Input parse error: {e}");
        SdkError::InvalidInput(e.to_string())
    }
}

impl From<bitcoin::address::ParseError> for SdkError {
    fn from(e: bitcoin::address::ParseError) -> Self {
        error!("Bitcoin address parse error: {e}");
        SdkError::InvalidInput(e.to_string())
    }
}

impl From<persist::StorageError> for SdkError {
    fn from(e: persist::StorageError) -> Self {
        error!("Storage error: {e}");
        SdkError::StorageError(e.to_string())
    }
}

impl From<Infallible> for SdkError {
    fn from(value: Infallible) -> Self {
        error!("Infallible error: {value}");
        SdkError::Generic(value.to_string())
    }
}

impl From<String> for SdkError {
    fn from(s: String) -> Self {
        error!("Generic error: {s}");
        Self::Generic(s)
    }
}

impl From<&str> for SdkError {
    fn from(s: &str) -> Self {
        error!("Generic error: {s}");
        Self::Generic(s.to_string())
    }
}

impl From<TryFromIntError> for SdkError {
    fn from(e: TryFromIntError) -> Self {
        error!("TryFromIntError: {e}");
        SdkError::Generic(e.to_string())
    }
}

impl From<serde_json::Error> for SdkError {
    fn from(e: serde_json::Error) -> Self {
        error!("Serde JSON error: {e}");
        SdkError::Generic(e.to_string())
    }
}

impl From<SparkWalletError> for SdkError {
    fn from(e: SparkWalletError) -> Self {
        error!("Spark wallet error: {e}");
        SdkError::SparkError(e.to_string())
    }
}

impl From<FromHexError> for SdkError {
    fn from(e: FromHexError) -> Self {
        error!("FromHexError: {e}");
        SdkError::Generic(e.to_string())
    }
}

impl From<uuid::Error> for SdkError {
    fn from(e: uuid::Error) -> Self {
        error!("UUID error: {e}");
        SdkError::InvalidUuid(e.to_string())
    }
}

impl From<ServiceConnectivityError> for SdkError {
    fn from(value: ServiceConnectivityError) -> Self {
        error!("Service connectivity error: {value}");
        SdkError::NetworkError(value.to_string())
    }
}

impl From<LnurlServerError> for SdkError {
    fn from(value: LnurlServerError) -> Self {
        error!("Lnurl server error: {value:?}");
        match value {
            LnurlServerError::InvalidApiKey => {
                SdkError::InvalidInput("Invalid api key".to_string())
            }
            LnurlServerError::Network {
                statuscode,
                message,
            } => SdkError::NetworkError(format!(
                "network request failed with status {statuscode}: {}",
                message.unwrap_or(String::new())
            )),
            LnurlServerError::RequestFailure(e) => SdkError::NetworkError(e),
            LnurlServerError::SigningError(e) => {
                SdkError::Generic(format!("Failed to sign message: {e}"))
            }
        }
    }
}

impl From<ReqwestLnurlServerClientError> for SdkError {
    fn from(value: ReqwestLnurlServerClientError) -> Self {
        error!("Reqwest Lnurl server client error: {value}");
        SdkError::Generic(value.to_string())
    }
}

impl From<TryInitError> for SdkError {
    fn from(value: TryInitError) -> Self {
        error!("Try init error: {value}");
        SdkError::Generic("Logging can only be initialized once".to_string())
    }
}

impl From<bip39::Error> for SdkError {
    fn from(value: bip39::Error) -> Self {
        error!("BIP39 error: {}", value);
        SdkError::InvalidInput(format!("Invalid mnemonic: {value}"))
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
