use crate::{
    Fee,
    lnurl::LnurlServerError,
    nostr::NostrError,
    persist::{self},
};
use bitcoin::consensus::encode::FromHexError;
use breez_sdk_common::error::ServiceConnectivityError;
use serde::{Deserialize, Serialize};
use spark_wallet::SparkWalletError;
use std::{convert::Infallible, num::TryFromIntError};
use thiserror::Error;
use tracing_subscriber::util::TryInitError;
use web_time::SystemTimeError;

/// Error type for the `BreezSdk`
#[derive(Debug, Error, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Error))]
pub enum SdkError {
    #[error("SparkSdkError: {0}")]
    SparkError(String),

    #[error("Insufficient funds")]
    InsufficientFunds,

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
        "Max deposit claim fee exceeded for utxo: {tx}:{vout} with max fee: {max_fee:?} and required fee: {required_fee_sats} sats or {required_fee_rate_sat_per_vbyte} sats/vbyte"
    )]
    MaxDepositClaimFeeExceeded {
        tx: String,
        vout: u32,
        max_fee: Option<Fee>,
        required_fee_sats: u64,
        required_fee_rate_sat_per_vbyte: u64,
    },

    #[error("Missing utxo: {tx}:{vout}")]
    MissingUtxo { tx: String, vout: u32 },

    #[error("Lnurl error: {0}")]
    LnurlError(String),

    #[error("Signer error: {0}")]
    Signer(String),

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

impl From<flashnet::FlashnetError> for SdkError {
    fn from(e: flashnet::FlashnetError) -> Self {
        match e {
            flashnet::FlashnetError::Network { reason, code } => {
                let code = match code {
                    Some(c) => format!(" (code: {c})"),
                    None => String::new(),
                };
                SdkError::NetworkError(format!("{reason}{code}"))
            }
            _ => SdkError::Generic(e.to_string()),
        }
    }
}

impl From<crate::token_conversion::ConversionError> for SdkError {
    fn from(e: crate::token_conversion::ConversionError) -> Self {
        use crate::token_conversion::ConversionError;
        match e {
            ConversionError::NoPoolsAvailable => {
                SdkError::Generic("No conversion pools available".to_string())
            }
            ConversionError::ConversionFailed(msg)
            | ConversionError::ValidationFailed(msg)
            | ConversionError::RefundFailed(msg) => SdkError::Generic(msg),
            ConversionError::Sdk(e) => e,
            ConversionError::Storage(e) => SdkError::StorageError(e.to_string()),
            ConversionError::Wallet(e) => SdkError::SparkError(e.to_string()),
        }
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

impl From<SystemTimeError> for SdkError {
    fn from(e: SystemTimeError) -> Self {
        SdkError::Generic(e.to_string())
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
        match e {
            SparkWalletError::InsufficientFunds => SdkError::InsufficientFunds,
            _ => SdkError::SparkError(e.to_string()),
        }
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

impl From<ServiceConnectivityError> for SdkError {
    fn from(value: ServiceConnectivityError) -> Self {
        SdkError::NetworkError(value.to_string())
    }
}

impl From<LnurlServerError> for SdkError {
    fn from(value: LnurlServerError) -> Self {
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

impl From<TryInitError> for SdkError {
    fn from(_value: TryInitError) -> Self {
        SdkError::Generic("Logging can only be initialized once".to_string())
    }
}

impl From<NostrError> for SdkError {
    fn from(value: NostrError) -> Self {
        match value {
            NostrError::KeyDerivationError(e) => {
                SdkError::Generic(format!("Nostr key derivation error: {e}"))
            }
            NostrError::ZapReceiptCreationError(e) => {
                SdkError::Generic(format!("Nostr zap receipt creation error: {e}"))
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Error, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum DepositClaimError {
    #[error(
        "Max deposit claim fee exceeded for utxo: {tx}:{vout} with max fee: {max_fee:?} and required fee: {required_fee_sats} sats or {required_fee_rate_sat_per_vbyte} sats/vbyte"
    )]
    MaxDepositClaimFeeExceeded {
        tx: String,
        vout: u32,
        max_fee: Option<Fee>,
        required_fee_sats: u64,
        required_fee_rate_sat_per_vbyte: u64,
    },

    #[error("Missing utxo: {tx}:{vout}")]
    MissingUtxo { tx: String, vout: u32 },

    #[error("Generic error: {message}")]
    Generic { message: String },
}

impl From<SdkError> for DepositClaimError {
    fn from(value: SdkError) -> Self {
        match value {
            SdkError::MaxDepositClaimFeeExceeded {
                tx,
                vout,
                max_fee,
                required_fee_sats,
                required_fee_rate_sat_per_vbyte,
            } => DepositClaimError::MaxDepositClaimFeeExceeded {
                tx,
                vout,
                max_fee,
                required_fee_sats,
                required_fee_rate_sat_per_vbyte,
            },
            SdkError::MissingUtxo { tx, vout } => DepositClaimError::MissingUtxo { tx, vout },
            SdkError::Generic(e) => DepositClaimError::Generic { message: e },
            _ => DepositClaimError::Generic {
                message: value.to_string(),
            },
        }
    }
}

/// Error type for signer operations
#[derive(Debug, Error, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Error))]
pub enum SignerError {
    #[error("Key derivation error: {0}")]
    KeyDerivation(String),

    #[error("Signing error: {0}")]
    Signing(String),

    #[error("Encryption error: {0}")]
    Encryption(String),

    #[error("Decryption error: {0}")]
    Decryption(String),

    #[error("FROST error: {0}")]
    Frost(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("Generic signer error: {0}")]
    Generic(String),
}

impl From<String> for SignerError {
    fn from(s: String) -> Self {
        SignerError::Generic(s)
    }
}

impl From<&str> for SignerError {
    fn from(s: &str) -> Self {
        SignerError::Generic(s.to_string())
    }
}
