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
    /// failed, in whichever denominations the provider publishes. A route can
    /// enforce a tighter bound than it publishes, so an amount inside the
    /// published one can still land here.
    ///
    /// The message is the provider's own rejection text with the published
    /// bound appended, so it already names the direction.
    #[error("{reason}{}", render_bound(*too_small, *bound_amount, *bound_usd_cents))]
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
        /// Guide section explaining amount limits.
        docs_url: String,
    },

    /// A cross-chain provider won't serve the route, for now or at all.
    ///
    /// The message leads with a fixed phrase for each case, for bindings that
    /// only see the text, followed by the provider's own reason.
    #[error("{}: {reason}", if *temporary { "Cross-chain route temporarily unavailable" } else { "Cross-chain route not supported" })]
    CrossChainRouteUnavailable {
        reason: String,
        /// `true` when the provider expects the route back shortly, so the
        /// same request can succeed later. Otherwise try another route.
        temporary: bool,
        /// Guide section explaining unavailable routes.
        docs_url: String,
    },

    /// Cross-chain payments are not enabled on this SDK instance.
    #[error("Cross-chain payments are disabled: set the cross-chain config to enable them")]
    CrossChainDisabled {
        /// Guide section explaining how to enable cross-chain payments.
        docs_url: String,
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
        /// Guide section explaining how to claim at a higher fee.
        docs_url: String,
    },

    #[error("Missing utxo: {tx}:{vout}")]
    MissingUtxo { tx: String, vout: u32 },

    /// The deposit is worth too little to claim: after the claim fee, what would
    /// be credited is below the dust limit. A drop in on-chain fees can make it
    /// claimable.
    #[error("Deposit too small to claim: {tx}:{vout}")]
    DepositTooSmall {
        tx: String,
        vout: u32,
        /// Guide section explaining when the deposit becomes claimable.
        docs_url: String,
    },

    /// Another claim on this deposit is already running.
    #[error("Deposit claim already in progress: {tx}:{vout}")]
    DepositClaimInProgress {
        tx: String,
        vout: u32,
        /// Guide section explaining why this needs no action.
        docs_url: String,
    },

    /// A refund for this deposit is already on the network, and the requested
    /// replacement does not pay enough to displace it.
    #[error(
        "A refund is already pending at {pending_fee_sats} sats: a replacement must pay at least {required_fee_sats} sats"
    )]
    RefundReplacementFeeTooLow {
        pending_fee_sats: u64,
        required_fee_sats: u64,
        /// Guide section explaining refund replacement.
        docs_url: String,
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
    InsufficientCpfpFunds {
        required_sat: u64,
        /// Guide section explaining how to fund the exit.
        docs_url: String,
    },

    #[error("Error: {0}")]
    Generic(String),
}

/// Sections of the guide's errors page that errors link to through their
/// `docs_url` field. Each section's id is the variant name in kebab-case.
mod docs_url {
    /// Builds the link to a section of the guide's errors page. Released SDKs
    /// keep linking to these sections, so their ids must not change.
    macro_rules! errors_page {
        ($section:literal) => {
            concat!(
                "https://sdk-doc-spark.breez.technology/guide/errors.html#",
                $section
            )
        };
    }

    pub(super) const CROSS_CHAIN_AMOUNT_OUT_OF_RANGE: &str =
        errors_page!("cross-chain-amount-out-of-range");
    pub(super) const CROSS_CHAIN_ROUTE_UNAVAILABLE: &str =
        errors_page!("cross-chain-route-unavailable");
    pub(super) const CROSS_CHAIN_DISABLED: &str = errors_page!("cross-chain-disabled");
    pub(super) const MAX_DEPOSIT_CLAIM_FEE_EXCEEDED: &str =
        errors_page!("max-deposit-claim-fee-exceeded");
    pub(super) const DEPOSIT_TOO_SMALL: &str = errors_page!("deposit-too-small");
    pub(super) const DEPOSIT_CLAIM_IN_PROGRESS: &str = errors_page!("deposit-claim-in-progress");
    pub(super) const REFUND_REPLACEMENT_FEE_TOO_LOW: &str =
        errors_page!("refund-replacement-fee-too-low");
    pub(super) const INSUFFICIENT_CPFP_FUNDS: &str = errors_page!("insufficient-cpfp-funds");

    #[cfg(all(test, not(target_family = "wasm")))]
    pub(super) const SECTION_PREFIX: &str = errors_page!("");
}

impl SdkError {
    pub(crate) fn cross_chain_amount_out_of_range(
        reason: String,
        too_small: bool,
        bound_amount: Option<u128>,
        bound_usd_cents: Option<u64>,
    ) -> Self {
        Self::CrossChainAmountOutOfRange {
            reason,
            too_small,
            bound_amount,
            bound_usd_cents,
            docs_url: docs_url::CROSS_CHAIN_AMOUNT_OUT_OF_RANGE.to_string(),
        }
    }

    pub(crate) fn cross_chain_route_unavailable(reason: String, temporary: bool) -> Self {
        Self::CrossChainRouteUnavailable {
            reason,
            temporary,
            docs_url: docs_url::CROSS_CHAIN_ROUTE_UNAVAILABLE.to_string(),
        }
    }

    pub(crate) fn cross_chain_disabled() -> Self {
        Self::CrossChainDisabled {
            docs_url: docs_url::CROSS_CHAIN_DISABLED.to_string(),
        }
    }

    pub(crate) fn max_deposit_claim_fee_exceeded(
        tx: String,
        vout: u32,
        max_fee: Option<Fee>,
        required_fee_sats: u64,
        required_fee_rate_sat_per_vbyte: u64,
    ) -> Self {
        Self::MaxDepositClaimFeeExceeded {
            tx,
            vout,
            max_fee,
            required_fee_sats,
            required_fee_rate_sat_per_vbyte,
            docs_url: docs_url::MAX_DEPOSIT_CLAIM_FEE_EXCEEDED.to_string(),
        }
    }

    pub(crate) fn deposit_too_small(tx: String, vout: u32) -> Self {
        Self::DepositTooSmall {
            tx,
            vout,
            docs_url: docs_url::DEPOSIT_TOO_SMALL.to_string(),
        }
    }

    pub(crate) fn deposit_claim_in_progress(tx: String, vout: u32) -> Self {
        Self::DepositClaimInProgress {
            tx,
            vout,
            docs_url: docs_url::DEPOSIT_CLAIM_IN_PROGRESS.to_string(),
        }
    }

    pub(crate) fn refund_replacement_fee_too_low(
        pending_fee_sats: u64,
        required_fee_sats: u64,
    ) -> Self {
        Self::RefundReplacementFeeTooLow {
            pending_fee_sats,
            required_fee_sats,
            docs_url: docs_url::REFUND_REPLACEMENT_FEE_TOO_LOW.to_string(),
        }
    }

    pub(crate) fn insufficient_cpfp_funds(required_sat: u64) -> Self {
        Self::InsufficientCpfpFunds {
            required_sat,
            docs_url: docs_url::INSUFFICIENT_CPFP_FUNDS.to_string(),
        }
    }

    /// Link to the guide page that explains this error, when it has one.
    pub fn docs_url(&self) -> Option<&str> {
        match self {
            Self::CrossChainAmountOutOfRange { docs_url, .. }
            | Self::CrossChainRouteUnavailable { docs_url, .. }
            | Self::CrossChainDisabled { docs_url }
            | Self::MaxDepositClaimFeeExceeded { docs_url, .. }
            | Self::DepositTooSmall { docs_url, .. }
            | Self::DepositClaimInProgress { docs_url, .. }
            | Self::RefundReplacementFeeTooLow { docs_url, .. }
            | Self::InsufficientCpfpFunds { docs_url, .. } => Some(docs_url),
            _ => None,
        }
    }
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
                SdkError::cross_chain_amount_out_of_range(reason, too_small, None, None)
            }
            flashnet::FlashnetError::RouteUnavailable { reason, temporary } => {
                SdkError::cross_chain_route_unavailable(reason, temporary)
            }
            flashnet::FlashnetError::Network { reason, code } => {
                let code = match code {
                    Some(c) => format!(" (code: {c})"),
                    None => String::new(),
                };
                SdkError::NetworkError(format!("{reason}{code}"))
            }
            flashnet::FlashnetError::InvalidRequest { reason, code }
            | flashnet::FlashnetError::AffiliateRejected { reason, code, .. } => {
                SdkError::NetworkError(format!("{reason} (code: {code})"))
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
            | ConversionError::FailedAfterSwap { message: msg, .. }
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
            ) => SdkError::insufficient_cpfp_funds(required_sat),
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

    /// The deposit is worth too little to claim: after the claim fee, what would
    /// be credited is below the dust limit. A drop in on-chain fees can make it
    /// claimable.
    #[error("Deposit too small to claim: {tx}:{vout}")]
    DepositTooSmall { tx: String, vout: u32 },

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
                ..
            } => DepositClaimError::MaxDepositClaimFeeExceeded {
                tx,
                vout,
                max_fee,
                required_fee_sats,
                required_fee_rate_sat_per_vbyte,
            },
            SdkError::MissingUtxo { tx, vout } => DepositClaimError::MissingUtxo { tx, vout },
            SdkError::DepositTooSmall { tx, vout, .. } => {
                DepositClaimError::DepositTooSmall { tx, vout }
            }
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
/// The bound is the published one, which a route can enforce more tightly than,
/// so the wording says "published" rather than naming it as the limit.
fn render_bound(
    too_small: bool,
    bound_amount: Option<u128>,
    bound_usd_cents: Option<u64>,
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
    format!(" (published {label}: {})", bounds.join(" or "))
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
    ) -> String {
        // Orchestra's own wording for each direction.
        let reason = if too_small {
            "Amount too small"
        } else {
            "Amount too large"
        };
        SdkError::cross_chain_amount_out_of_range(
            reason.to_string(),
            too_small,
            bound_amount,
            bound_usd_cents,
        )
        .to_string()
    }

    #[test]
    fn names_both_denominations_when_both_are_published() {
        assert_eq!(
            message(true, Some(1200), Some(80)),
            "Amount too small (published minimum: 1200 base units or 0.80 USD)"
        );
    }

    #[test]
    fn a_too_large_rejection_reads_as_a_maximum() {
        assert_eq!(
            message(false, None, Some(8_980_000)),
            "Amount too large (published maximum: 89800.00 USD)"
        );
    }

    #[test]
    fn says_nothing_when_the_provider_publishes_no_bound() {
        assert_eq!(message(true, None, None), "Amount too small");
    }

    #[test]
    fn pads_a_sub_dollar_bound_to_two_decimals() {
        assert!(message(true, None, Some(5)).ends_with("(published minimum: 0.05 USD)"));
    }
}

#[cfg(test)]
mod route_unavailable_tests {
    use super::*;

    fn from_provider(temporary: bool, reason: &str) -> SdkError {
        flashnet::FlashnetError::RouteUnavailable {
            reason: reason.to_string(),
            temporary,
        }
        .into()
    }

    #[test]
    fn keeps_whether_a_retry_can_succeed() {
        assert!(matches!(
            from_provider(true, "busy"),
            SdkError::CrossChainRouteUnavailable {
                temporary: true,
                ..
            }
        ));
    }

    #[test]
    fn the_message_names_the_case_before_the_provider_reason() {
        assert_eq!(
            from_provider(
                true,
                "This route is temporarily unavailable. Please try again later."
            )
            .to_string(),
            "Cross-chain route temporarily unavailable: This route is temporarily unavailable. \
             Please try again later."
        );
        assert_eq!(
            from_provider(false, "Unsupported route").to_string(),
            "Cross-chain route not supported: Unsupported route"
        );
    }
}

#[cfg(test)]
mod invalid_request_tests {
    use super::*;

    #[test]
    fn reads_the_same_as_a_plain_provider_error() {
        let invalid: SdkError = flashnet::FlashnetError::InvalidRequest {
            reason: "Unknown affiliateId: breez_ff".to_string(),
            code: 400,
        }
        .into();
        let plain: SdkError = flashnet::FlashnetError::Network {
            reason: "Unknown affiliateId: breez_ff".to_string(),
            code: Some(400),
        }
        .into();
        assert!(matches!(invalid, SdkError::NetworkError(_)));
        assert_eq!(invalid.to_string(), plain.to_string());
    }

    #[test]
    fn an_affiliate_rejection_reads_the_same_as_a_plain_provider_error() {
        let rejected: SdkError = flashnet::FlashnetError::AffiliateRejected {
            reason: "Affiliate is disabled".to_string(),
            code: 400,
        }
        .into();
        let plain: SdkError = flashnet::FlashnetError::Network {
            reason: "Affiliate is disabled".to_string(),
            code: Some(400),
        }
        .into();
        assert!(matches!(rejected, SdkError::NetworkError(_)));
        assert_eq!(rejected.to_string(), plain.to_string());
    }
}

#[cfg(test)]
#[cfg(not(target_family = "wasm"))]
mod docs_url_tests {
    use super::*;

    fn errors_page() -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../docs/breez-sdk/src/guide/errors.md");
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
    }

    fn has_section(page: &str, id: &str) -> bool {
        page.contains(&format!("id=\"{id}\""))
    }

    /// `CrossChainDisabled` to `cross-chain-disabled`.
    fn kebab_case(name: &str) -> String {
        let mut id = String::new();
        for c in name.chars() {
            if c.is_ascii_uppercase() && !id.is_empty() {
                id.push('-');
            }
            id.push(c.to_ascii_lowercase());
        }
        id
    }

    /// This file's `SdkError` definition.
    fn sdk_error_source() -> &'static str {
        let source = include_str!("error.rs");
        let start = source
            .find("pub enum SdkError {")
            .expect("SdkError definition");
        let body = &source[start..];
        &body[..body.find("\n}\n").expect("end of SdkError")]
    }

    fn sdk_error_variants() -> Vec<String> {
        sdk_error_source()
            .lines()
            .filter_map(|line| {
                let name = line.strip_prefix("    ")?;
                let name: String = name
                    .chars()
                    .take_while(char::is_ascii_alphanumeric)
                    .collect();
                name.starts_with(|c: char| c.is_ascii_uppercase())
                    .then_some(name)
            })
            .collect()
    }

    /// One of each variant that links to the guide, from its constructor.
    fn linked_errors() -> Vec<SdkError> {
        vec![
            SdkError::cross_chain_amount_out_of_range(String::new(), true, None, None),
            SdkError::cross_chain_route_unavailable(String::new(), true),
            SdkError::cross_chain_disabled(),
            SdkError::max_deposit_claim_fee_exceeded(String::new(), 0, None, 0, 0),
            SdkError::deposit_too_small(String::new(), 0),
            SdkError::deposit_claim_in_progress(String::new(), 0),
            SdkError::refund_replacement_fee_too_low(0, 0),
            SdkError::insufficient_cpfp_funds(0),
        ]
    }

    #[test]
    fn every_linked_variant_points_at_its_own_section_of_the_errors_page() {
        let page = errors_page();
        let errors = linked_errors();
        assert_eq!(
            errors.len(),
            sdk_error_source().matches("docs_url: String,").count(),
            "linked_errors() must list every variant with a docs_url field"
        );
        for err in errors {
            let name: String = format!("{err:?}")
                .chars()
                .take_while(char::is_ascii_alphanumeric)
                .collect();
            let url = err
                .docs_url()
                .unwrap_or_else(|| panic!("{name} has no docs_url"));
            let anchor = url
                .strip_prefix(docs_url::SECTION_PREFIX)
                .unwrap_or_else(|| panic!("{url} is not a section of the errors page"));
            assert_eq!(anchor, kebab_case(&name), "{name} links to another section");
            assert!(
                has_section(&page, anchor),
                "errors.md has no section #{anchor}"
            );
        }
    }

    #[test]
    fn every_sdk_error_variant_has_a_section_on_the_errors_page() {
        let page = errors_page();
        let variants = sdk_error_variants();
        assert!(variants.contains(&"CrossChainDisabled".to_string()));
        let missing: Vec<String> = variants
            .iter()
            .map(|v| kebab_case(v))
            .filter(|id| !has_section(&page, id))
            .collect();
        assert!(
            missing.is_empty(),
            "errors.md has no section for: {missing:?}"
        );
    }

    #[test]
    fn messages_leave_the_link_to_docs_url() {
        for err in linked_errors() {
            assert!(!err.to_string().contains("https://"), "{err:?}");
        }
    }
}
