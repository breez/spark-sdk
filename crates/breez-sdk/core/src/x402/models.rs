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
        /// The MCP server's URI: for a client, the one it connected to, from
        /// its own configuration, and for a server, its own public URI. The
        /// payment commits to it, so both must be exactly the same string.
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
    /// The payment hasn't completed, or its proof of payment hasn't arrived
    /// yet. Paying the same challenge again picks it up without paying twice.
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

/// A request to create an x402 challenge for a request that hasn't been paid for.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CreateX402ChallengeRequest {
    /// The request being charged for, as received. For HTTP, its URL must be
    /// the public URL the client used, built from the server's configured
    /// origin rather than from client-supplied headers.
    pub request: X402Request,
    /// The HTTP headers, or MCP `params._meta` members, the payment also
    /// commits to besides the method, URL and body, or the tool call. Include
    /// every one that changes what is served or to whom, such as
    /// `authorization`. Sorted, with HTTP header names in lowercase. MCP
    /// member names are case-sensitive.
    pub bound_names: Vec<String>,
    /// The price on each rail the challenge accepts, one amount per rail.
    pub amounts: Vec<X402PaymentAmount>,
    /// How long the challenge can be paid, in seconds. Defaults to 300.
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub max_timeout_secs: Option<u32>,
    /// What is being paid for, shown to the payer.
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub description: Option<String>,
    /// The resource's MIME type.
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub mime_type: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CreateX402ChallengeResponse {
    /// The challenge: the `PAYMENT-REQUIRED` header value of an HTTP 402
    /// response, or the JSON for an MCP tool result's `structuredContent`.
    pub payment_required: String,
    /// The invoice's payment hash, in lowercase hex.
    pub payment_hash: String,
}

/// A request to settle the payment sent with a paid-for request.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SettleX402Request {
    /// The `PAYMENT-SIGNATURE` header value, or the JSON in an MCP call's
    /// `params._meta["x402/payment"]`.
    pub payment_signature: String,
    /// The request being served, as received.
    pub request: X402Request,
    /// The names bound when the challenge was created.
    pub bound_names: Vec<String>,
    /// The request's current price on each rail, as given to
    /// [`X402::create_challenge`](crate::X402::create_challenge). A payment for
    /// another price is refused, so keep a price stable while challenges for
    /// it can still be paid.
    pub amounts: Vec<X402PaymentAmount>,
    /// The challenge lifetime given to
    /// [`X402::create_challenge`](crate::X402::create_challenge). Defaults to
    /// 300.
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub max_timeout_secs: Option<u32>,
}

/// How to answer a request after settling its payment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum X402SettleStatus {
    /// The payment is valid, paid to this SDK and now used. Serve the request.
    Settled,
    /// The payment's receipt isn't confirmed yet, so it may still settle. Over
    /// HTTP, answer with status 402 and no new challenge, so the client sends
    /// the same payment again.
    Pending,
    /// The payment can't be read. Over HTTP, answer with status 400.
    Malformed,
    /// The payment is refused. Answer with a new challenge.
    Refused,
    /// This server couldn't settle the payment, for example because its replay
    /// store can't be reached. The payment stays unused. Over HTTP, answer with
    /// status 500.
    Error,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SettleX402Response {
    pub status: X402SettleStatus,
    /// The x402 reason the payment didn't settle.
    pub error_reason: Option<String>,
    /// The outcome to send with the response, whatever the status: the
    /// `PAYMENT-RESPONSE` header value for HTTP, or the JSON for an MCP
    /// result's `_meta["x402/payment-response"]`.
    pub payment_response: String,
    /// The payment hash, in lowercase hex, when the payment could be
    /// identified. Unless the status is settled, it is the client's claim and
    /// unverified.
    pub payment_hash: Option<String>,
    /// The amount paid, when settled.
    pub amount: Option<X402PaymentAmount>,
}
