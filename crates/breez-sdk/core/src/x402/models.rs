use serde::Serialize;

use crate::{ConversionOptions, Payment};

/// A request to pay an x402 challenge over Lightning.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PayX402Request {
    /// The server's challenge: the `PAYMENT-REQUIRED` header value of an HTTP
    /// 402 response, or the JSON in an MCP tool result's `structuredContent`.
    pub payment_required: String,
    /// The request being paid for, as it will be sent again with the payment.
    pub request: X402Request,
    /// The most to spend, fees included, and the rail to pay on. A challenge
    /// that would cost more, or doesn't accept that rail, is refused before
    /// anything is paid.
    pub max_amount: X402PaymentAmount,
    /// Converts a token balance to sats to pay with, such as paying from USDB.
    /// The conversion's cost is bounded by its slippage, not by `max_amount`.
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub conversion_options: Option<ConversionOptions>,
    /// How long to wait for the payment to complete. Defaults to 30 seconds.
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub completion_timeout_secs: Option<u32>,
}

/// The request an x402 payment is for. The server's invoice commits to it, so
/// a challenge issued for any other request is refused.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum X402Request {
    Http {
        /// The HTTP method, case preserved.
        method: String,
        /// The absolute URL, exactly as requested.
        url: String,
        /// The request's headers. Only those the server binds are used.
        headers: Vec<X402Header>,
        /// The raw body bytes. Unset for a request without a body.
        body: Option<Vec<u8>>,
    },
    /// An MCP `tools/call`.
    Mcp {
        /// The MCP server URI from the app's own configuration.
        server: String,
        /// The call's `params` object as JSON (`name`, `arguments`, `_meta`).
        params: String,
    },
}

/// An amount on one of the rails an x402 payment can be made on, in that
/// rail's units.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum X402PaymentAmount {
    Lightning { amount_sats: u64 },
}

#[derive(Clone, Debug)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct X402Header {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum X402PaymentStatus {
    /// The invoice is paid and the payment signature is set.
    Paid,
    /// The payment is still in flight. Paying the same challenge again picks
    /// it up without paying twice.
    Pending,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PayX402Response {
    pub status: X402PaymentStatus,
    /// The payment to send the request again with: the `PAYMENT-SIGNATURE`
    /// header value for HTTP, or the JSON for an MCP call's
    /// `params._meta["x402/payment"]`. Set when the status is paid.
    pub payment_signature: Option<String>,
    pub payment: Payment,
}
