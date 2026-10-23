use base64::{
    Engine,
    alphabet::STANDARD,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
    prelude::BASE64_STANDARD,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};

use super::error::{X402Error, X402ErrorReason, X402Result};

pub const X402_VERSION: u32 = 2;

pub const PAYMENT_REQUIRED_HEADER: &str = "PAYMENT-REQUIRED";
pub const PAYMENT_SIGNATURE_HEADER: &str = "PAYMENT-SIGNATURE";
pub const PAYMENT_RESPONSE_HEADER: &str = "PAYMENT-RESPONSE";

/// How x402 messages travel: base64 JSON in a header over HTTP, plain JSON
/// in the message over MCP.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    Http,
    Mcp,
}

/// Headers are standard base64. Some servers drop the padding, which decodes the same.
const HEADER_DECODER: GeneralPurpose = GeneralPurpose::new(
    &STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// The body of a `PAYMENT-REQUIRED` header.
///
/// `accepts` entries stay raw JSON: the client echoes the one it paid, and
/// every server-declared field must come back unchanged.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentRequired {
    pub x402_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub resource: ResourceInfo,
    pub accepts: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceInfo {
    pub url: String,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

/// The body of a `PAYMENT-SIGNATURE` header.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentPayload {
    pub x402_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<ResourceInfo>,
    pub accepted: Value,
    pub payload: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<Value>,
}

impl PaymentRequired {
    /// Decodes a `PAYMENT-REQUIRED` header value.
    pub fn from_header(value: &str) -> X402Result<Self> {
        decode_header(value, X402ErrorReason::InvalidPaymentRequirements)
    }

    /// Parses the JSON an MCP tool result carries in `structuredContent`.
    pub fn from_json(value: &str) -> X402Result<Self> {
        serde_json::from_str(value).map_err(|e| {
            X402Error::new(
                X402ErrorReason::InvalidPaymentRequirements,
                format!("not valid JSON: {e}"),
            )
        })
    }

    /// Accepts either transport's form. A base64 header value never starts
    /// with `{`, so the two cannot be confused.
    pub fn from_header_or_json(value: &str) -> X402Result<Self> {
        if value.trim_start().starts_with('{') {
            Self::from_json(value)
        } else {
            Self::from_header(value)
        }
    }
}

/// The body of a `PAYMENT-RESPONSE` header.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettlementResponse {
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_reason: Option<String>,
    pub transaction: String,
    pub network: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payer: Option<String>,
}

impl PaymentRequired {
    /// Encodes the challenge as a `PAYMENT-REQUIRED` header value, or as JSON
    /// for an MCP tool result's `structuredContent`.
    pub fn encode(&self, transport: Transport) -> X402Result<String> {
        encode_for(self, transport)
    }
}

impl SettlementResponse {
    /// Encodes the response as a `PAYMENT-RESPONSE` header value, or as JSON
    /// for MCP's `_meta["x402/payment-response"]`.
    pub fn encode(&self, transport: Transport) -> X402Result<String> {
        encode_for(self, transport)
    }
}

impl PaymentPayload {
    /// Accepts either a `PAYMENT-SIGNATURE` header value or MCP's
    /// `_meta["x402/payment"]` JSON.
    pub fn from_header_or_json(value: &str) -> X402Result<Self> {
        if value.trim_start().starts_with('{') {
            serde_json::from_str(value).map_err(|e| {
                X402Error::new(
                    X402ErrorReason::InvalidPayload,
                    format!("not valid JSON: {e}"),
                )
            })
        } else {
            decode_header(value, X402ErrorReason::InvalidPayload)
        }
    }

    /// Encodes the payload as a `PAYMENT-SIGNATURE` header value, or as JSON
    /// for MCP's `_meta["x402/payment"]`.
    pub fn encode(&self, transport: Transport) -> X402Result<String> {
        encode_for(self, transport)
    }
}

fn encode_for<T: Serialize>(value: &T, transport: Transport) -> X402Result<String> {
    match transport {
        Transport::Http => encode_header(value),
        Transport::Mcp => encode_json(value),
    }
}

fn encode_json<T: Serialize>(value: &T) -> X402Result<String> {
    serde_json::to_string(value).map_err(|e| {
        X402Error::new(
            X402ErrorReason::InvalidPaymentRequirements,
            format!("failed to serialize: {e}"),
        )
    })
}

fn decode_header<T: DeserializeOwned>(value: &str, reason: X402ErrorReason) -> X402Result<T> {
    let bytes = HEADER_DECODER
        .decode(value.trim())
        .map_err(|e| X402Error::new(reason, format!("header is not base64: {e}")))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| X402Error::new(reason, format!("header is not valid JSON: {e}")))
}

fn encode_header<T: Serialize>(value: &T) -> X402Result<String> {
    let json = serde_json::to_vec(value).map_err(|e| {
        X402Error::new(
            X402ErrorReason::InvalidPaymentRequirements,
            format!("failed to serialize header: {e}"),
        )
    })?;
    Ok(BASE64_STANDARD.encode(json))
}
