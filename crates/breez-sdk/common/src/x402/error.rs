use thiserror::Error;

pub type X402Result<T> = Result<T, X402Error>;

/// Failure reasons from the x402 `exact` on `lnbtc` specification. The string
/// form is the spec's stable vocabulary, so it can be logged or reported as is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum X402ErrorReason {
    InvalidX402Version,
    InvalidPaymentRequirements,
    InvalidPayload,
    UnsupportedScheme,
    UnsupportedNetwork,
    NetworkMismatch,
    Asset,
    Amount,
    AmountMismatch,
    PayToMalformed,
    PayToMismatch,
    MaxTimeout,
    MaxTimeoutMismatch,
    ExtraMismatch,
    AssetTransferMethod,
    PaymentFlow,
    RequestBinding,
    RequestMismatch,
    InvoiceMissing,
    InvoiceDecodeFailed,
    InvoiceDescription,
    InvoiceRequestMismatch,
    InvoicePayeeMismatch,
    InvoiceCurrencyMismatch,
    InvoiceAmountMismatch,
    InvoiceExpiryMismatch,
    InvoiceCreatedInFuture,
    InvoiceExpired,
    PayerInvoiceMismatch,
    PayerPaymentHashMismatch,
    PayerAmountMismatch,
    PaymentInFlight,
    PaymentNotPaid,
    PayerPreimageRequired,
    PayerPreimageMalformed,
    PayerPreimageHashMismatch,
    PreimageMissing,
    PreimageMalformed,
    PreimageLength,
    PreimageHashMismatch,
    DuplicateSettlement,
    /// The accepted terms carry no valid quote from this receiver.
    InvalidQuote,
    /// The invoice is not yet settled to the receiver. May be retried.
    InvoiceNotSettled,
    /// The receiver could not be asked whether the invoice is settled. May be retried.
    ReceiverUnavailable,
}

impl X402ErrorReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidX402Version => "invalid_x402_version",
            Self::InvalidPaymentRequirements => "invalid_payment_requirements",
            Self::InvalidPayload => "invalid_payload",
            Self::UnsupportedScheme => "unsupported_scheme",
            Self::UnsupportedNetwork => "unsupported_network",
            Self::NetworkMismatch => "network_mismatch",
            Self::Asset => "invalid_exact_lnbtc_asset",
            Self::Amount => "invalid_exact_lnbtc_amount",
            Self::AmountMismatch => "invalid_exact_lnbtc_amount_mismatch",
            Self::PayToMalformed => "invalid_exact_lnbtc_pay_to_malformed",
            Self::PayToMismatch => "invalid_exact_lnbtc_pay_to_mismatch",
            Self::MaxTimeout => "invalid_exact_lnbtc_max_timeout",
            Self::MaxTimeoutMismatch => "invalid_exact_lnbtc_max_timeout_mismatch",
            Self::ExtraMismatch => "invalid_exact_lnbtc_extra_mismatch",
            Self::AssetTransferMethod => "invalid_exact_lnbtc_asset_transfer_method",
            Self::PaymentFlow => "invalid_exact_lnbtc_payment_flow",
            Self::RequestBinding => "invalid_exact_lnbtc_request_binding",
            Self::RequestMismatch => "invalid_exact_lnbtc_request_mismatch",
            Self::InvoiceMissing => "invalid_exact_lnbtc_invoice_missing",
            Self::InvoiceDecodeFailed => "invalid_exact_lnbtc_invoice_decode_failed",
            Self::InvoiceDescription => "invalid_exact_lnbtc_invoice_description",
            Self::InvoiceRequestMismatch => "invalid_exact_lnbtc_invoice_request_mismatch",
            Self::InvoicePayeeMismatch => "invalid_exact_lnbtc_invoice_payee_mismatch",
            Self::InvoiceCurrencyMismatch => "invalid_exact_lnbtc_invoice_currency_mismatch",
            Self::InvoiceAmountMismatch => "invalid_exact_lnbtc_invoice_amount_mismatch",
            Self::InvoiceExpiryMismatch => "invalid_exact_lnbtc_invoice_expiry_mismatch",
            Self::InvoiceCreatedInFuture => "invalid_exact_lnbtc_invoice_created_in_future",
            Self::InvoiceExpired => "invalid_exact_lnbtc_invoice_expired",
            Self::PayerInvoiceMismatch => "invalid_exact_lnbtc_payer_invoice_mismatch",
            Self::PayerPaymentHashMismatch => "invalid_exact_lnbtc_payer_payment_hash_mismatch",
            Self::PayerAmountMismatch => "invalid_exact_lnbtc_payer_amount_mismatch",
            Self::PaymentInFlight => "exact_lnbtc_payment_in_flight",
            Self::PaymentNotPaid => "exact_lnbtc_payment_not_paid",
            Self::PayerPreimageRequired => "invalid_exact_lnbtc_payer_preimage_required",
            Self::PayerPreimageMalformed => "invalid_exact_lnbtc_payer_preimage_malformed",
            Self::PayerPreimageHashMismatch => "invalid_exact_lnbtc_payer_preimage_hash_mismatch",
            Self::PreimageMissing => "invalid_exact_lnbtc_preimage_missing",
            Self::PreimageMalformed => "invalid_exact_lnbtc_preimage_malformed",
            Self::PreimageLength => "invalid_exact_lnbtc_preimage_length",
            Self::PreimageHashMismatch => "invalid_exact_lnbtc_preimage_hash_mismatch",
            Self::DuplicateSettlement => "duplicate_settlement",
            Self::InvalidQuote => "invalid_exact_lnbtc_quote",
            Self::InvoiceNotSettled => "exact_lnbtc_invoice_not_settled",
            Self::ReceiverUnavailable => "exact_lnbtc_receiver_unavailable",
        }
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("{}: {}", .reason.as_str(), .detail)]
pub struct X402Error {
    pub reason: X402ErrorReason,
    pub detail: String,
}

impl X402Error {
    pub fn new(reason: X402ErrorReason, detail: impl Into<String>) -> Self {
        Self {
            reason,
            detail: detail.into(),
        }
    }
}
