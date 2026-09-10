use crate::{
    Fee,
    lnurl::LnurlServerError,
    persist::{self},
};
use bitcoin::consensus::encode::FromHexError;
use breez_sdk_common::error::ServiceConnectivityError;
use platform_utils::time::SystemTimeError;
use serde::{Deserialize, Serialize};
use spark_wallet::SparkWalletError;
use std::{convert::Infallible, num::TryFromIntError};
use thiserror::Error;
use tracing_subscriber::util::TryInitError;

/// Error type for the `BreezSdk`
#[derive(Debug, Error, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Error))]
pub enum SdkError {
    #[error("SparkSdkError: {0}")]
    SparkError(String),

    #[error("Insufficient funds{}", .token_identifier.as_deref().map(|t| format!(" for token {t}")).unwrap_or_default())]
    InsufficientFunds {
        /// The token that cannot cover the payment. Unset when the shortfall is
        /// in sats or when no single token can be named.
        token_identifier: Option<String>,
    },

    #[error("Invalid UUID: {0}")]
    InvalidUuid(String),

    /// Invalid input error
    #[error("Invalid input: {0}")]
    InvalidInput(String),

    /// A cross-chain provider rejected the amount as outside what it accepts
    /// for the route.
    ///
    /// The bound fields carry what the route publishes in the direction that
    /// failed, in whichever denominations the provider publishes. They can be
    /// looser than what it actually enforces: when `dynamic_limits_possible` is
    /// set, the route is carried over legs with their own moving minimums and
    /// liquidity ceilings, so an amount inside the published bound can still
    /// land here.
    ///
    /// The message is the provider's own rejection text with the published
    /// bound appended, so it already names the direction.
    #[error(
        "{reason}{}",
        render_bound(*too_small, *bound_amount, *bound_usd_cents, *dynamic_limits_possible)
    )]
    CrossChainAmountOutOfRange {
        reason: String,
        /// `true` for a rejection below the minimum, `false` for one above the
        /// maximum or beyond available liquidity.
        too_small: bool,
        /// The published bound in the base units of the asset paid in: the
        /// Spark-side asset on a send, the external asset on a receive.
        bound_amount: Option<u128>,
        /// The published bound as an order value in USD cents.
        bound_usd_cents: Option<u64>,
        /// Whether the provider can reject an amount that satisfies the bound.
        dynamic_limits_possible: bool,
    },

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

    /// Another claim on this deposit is already running.
    #[error("Deposit claim already in progress: {tx}:{vout}")]
    DepositClaimInProgress { tx: String, vout: u32 },

    /// A refund for this deposit is already on the network, and the requested
    /// replacement does not pay enough to displace it.
    #[error(
        "A refund is already pending at {pending_fee_sats} sats: a replacement must pay at least {required_fee_sats} sats"
    )]
    RefundReplacementFeeTooLow {
        pending_fee_sats: u64,
        required_fee_sats: u64,
    },

    #[error("Lnurl error: {0}")]
    LnurlError(String),

    #[error("Signer error: {0}")]
    Signer(String),

    /// `optimize_leaves` was called while another optimization run (auto or
    /// manual) was already in flight.
    #[error("Optimization is already in progress")]
    OptimizationAlreadyRunning,

    /// `optimize_leaves` was preempted by the SDK to free leaves for a
    /// higher-priority operation (typically a payment).
    #[error("Optimization was cancelled by the SDK to free leaves")]
    OptimizationCancelled,

    /// The provided CPFP funding is too low to cover the exit's on-chain fees.
    #[error("Insufficient CPFP funding: need at least {required_sat} sats")]
    InsufficientCpfpFunds { required_sat: u64 },

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
            flashnet::FlashnetError::AmountOutOfRange { reason, too_small } => {
                SdkError::CrossChainAmountOutOfRange {
                    reason,
                    too_small,
                    bound_amount: None,
                    bound_usd_cents: None,
                    dynamic_limits_possible: false,
                }
            }
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

impl From<boltz_client::BoltzError> for SdkError {
    fn from(e: boltz_client::BoltzError) -> Self {
        use boltz_client::BoltzError;
        match e {
            BoltzError::Api { reason, code } => {
                let code = match code {
                    Some(c) => format!(" (code: {c})"),
                    None => String::new(),
                };
                SdkError::NetworkError(format!("Boltz API: {reason}{code}"))
            }
            BoltzError::WebSocket(s) => SdkError::NetworkError(format!("Boltz WebSocket: {s}")),
            BoltzError::Store(s) => SdkError::StorageError(format!("Boltz store: {s}")),
            BoltzError::AmountOutOfRange { .. }
            | BoltzError::QuoteExpired
            | BoltzError::InvalidQuote(_)
            | BoltzError::QuoteDegradedBeyondSlippage { .. }
            | BoltzError::DuplicatePreimage
            | BoltzError::InvalidConfig(_) => SdkError::InvalidInput(e.to_string()),
            _ => SdkError::Generic(format!("Boltz: {e}")),
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
            ConversionError::DuplicateTransfer => {
                SdkError::Generic("Duplicate transfer: conversion already handled".to_string())
            }
            ConversionError::Sdk(e) => e,
            ConversionError::Storage(e) => SdkError::StorageError(e.to_string()),
            ConversionError::Wallet(e) => SdkError::SparkError(e.to_string()),
        }
    }
}

impl From<persist::StorageError> for SdkError {
    fn from(e: persist::StorageError) -> Self {
        match e {
            persist::StorageError::NotFound => SdkError::InvalidInput("Not found".to_string()),
            _ => SdkError::StorageError(e.to_string()),
        }
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
            SparkWalletError::InsufficientFunds => SdkError::InsufficientFunds {
                token_identifier: None,
            },
            SparkWalletError::TokenOutputServiceError(
                spark_wallet::TokenOutputServiceError::InsufficientFunds { token_identifier },
            ) => SdkError::InsufficientFunds { token_identifier },
            SparkWalletError::ServiceError(spark_wallet::ServiceError::InvalidInput(msg)) => {
                SdkError::InvalidInput(msg)
            }
            SparkWalletError::ServiceError(
                spark_wallet::ServiceError::InsufficientCpfpBudget { required_sat },
            ) => SdkError::InsufficientCpfpFunds { required_sat },
            _ => SdkError::SparkError(e.to_string()),
        }
    }
}

impl From<spark_wallet::OptimizationError> for SdkError {
    fn from(e: spark_wallet::OptimizationError) -> Self {
        match e {
            spark_wallet::OptimizationError::AlreadyRunning => SdkError::OptimizationAlreadyRunning,
            spark_wallet::OptimizationError::Cancelled => SdkError::OptimizationCancelled,
            spark_wallet::OptimizationError::Tree(inner) => SdkError::SparkError(inner.to_string()),
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

    #[error("Encryption unavailable: {0}")]
    EncryptionUnavailable(String),

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

/// Renders the bound an amount rejection ran into, for the error message.
/// Empty when the provider publishes nothing in that direction.
///
/// Both denominations are named when both are published: which one the provider
/// applied is not reported, and they are not interconvertible without a rate.
fn render_bound(
    too_small: bool,
    bound_amount: Option<u128>,
    bound_usd_cents: Option<u64>,
    dynamic_limits_possible: bool,
) -> String {
    let mut bounds = Vec::new();
    if let Some(amount) = bound_amount {
        bounds.push(format!("{amount} base units"));
    }
    if let Some(cents) = bound_usd_cents {
        bounds.push(format!("{}.{:02} USD", cents / 100, cents % 100));
    }
    if bounds.is_empty() {
        return String::new();
    }
    let label = if too_small { "minimum" } else { "maximum" };
    // Live legs move the bound in whichever direction failed: their own
    // minimums raise the floor, their liquidity lowers the ceiling.
    let caveat = match (dynamic_limits_possible, too_small) {
        (true, true) => "; the route can enforce a higher one",
        (true, false) => "; the route can enforce a lower one",
        (false, _) => "",
    };
    format!(" (published {label}: {}{caveat})", bounds.join(" or "))
}

#[cfg(test)]
mod render_bound_tests {
    use super::*;

    /// The rendered message is the only channel bindings that flatten
    /// `SdkError` to a string have, so the number has to survive into it.
    fn message(
        too_small: bool,
        bound_amount: Option<u128>,
        bound_usd_cents: Option<u64>,
        dynamic_limits_possible: bool,
    ) -> String {
        // Orchestra's own wording for each direction.
        let reason = if too_small {
            "Amount too small"
        } else {
            "Amount too large"
        };
        SdkError::CrossChainAmountOutOfRange {
            reason: reason.to_string(),
            too_small,
            bound_amount,
            bound_usd_cents,
            dynamic_limits_possible,
        }
        .to_string()
    }

    #[test]
    fn names_both_denominations_when_both_are_published() {
        assert_eq!(
            message(true, Some(1200), Some(80), false),
            "Amount too small (published minimum: 1200 base units or 0.80 USD)"
        );
    }

    #[test]
    fn flags_a_route_that_can_enforce_a_higher_minimum() {
        assert_eq!(
            message(true, None, Some(80), true),
            "Amount too small (published minimum: 0.80 USD; the route can enforce a higher one)"
        );
    }

    #[test]
    fn a_too_large_rejection_reads_as_a_maximum() {
        // A liquidity rejection lands here, and live liquidity sits below the
        // published ceiling, so the caveat qualifies a maximum too.
        assert_eq!(
            message(false, None, Some(8_980_000), true),
            "Amount too large (published maximum: 89800.00 USD; the route can enforce a lower one)"
        );
        assert_eq!(
            message(false, None, Some(8_980_000), false),
            "Amount too large (published maximum: 89800.00 USD)",
            "a route that publishes exact bounds carries no caveat"
        );
    }

    #[test]
    fn says_nothing_when_the_provider_publishes_no_bound() {
        assert_eq!(message(true, None, None, true), "Amount too small");
    }

    #[test]
    fn pads_a_sub_dollar_bound_to_two_decimals() {
        assert!(message(true, None, Some(5), false).ends_with("(published minimum: 0.05 USD)"));
    }
}
