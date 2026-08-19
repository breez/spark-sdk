use serde::{Deserialize, Serialize};

pub mod signed_message;

#[derive(Debug, Serialize, Deserialize)]
pub struct CheckUsernameAvailableResponse {
    pub available: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RecoverLnurlPayRequest {
    pub signature: String,
    pub timestamp: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RecoverLnurlPayResponse {
    pub lnurl: String,
    pub lightning_address: String,
    pub username: String,
    pub description: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterLnurlPayRequest {
    pub username: String,
    pub signature: String,
    pub timestamp: u64,
    pub description: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UnregisterLnurlPayRequest {
    pub username: String,
    pub signature: String,
    pub timestamp: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterLnurlPayResponse {
    pub lnurl: String,
    pub lightning_address: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TransferLnurlPayRequest {
    pub username: String,
    pub description: String,
    /// Hex-encoded secp256k1 compressed public key of the current owner (A).
    pub from_pubkey: String,
    /// Hex-encoded DER ECDSA signature by A over
    /// [`signed_message::transfer_from`].
    pub from_signature: String,
    /// Hex-encoded DER ECDSA signature by B (the `to_pubkey` in the URL path)
    /// over [`signed_message::transfer_to`].
    pub to_signature: String,
    /// Seconds since the Unix epoch, covered by both signatures, bounding the
    /// authorization in time.
    ///
    /// Absent selects the legacy untimestamped messages, which is what a client
    /// predating the v2 format sends. Required once the legacy candidates are
    /// dropped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TransferLnurlPayResponse {
    pub lnurl: String,
    pub lightning_address: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ListMetadataRequest {
    /// Hex-encoded DER ECDSA signature over [`signed_message::metadata`].
    ///
    /// Optional in the query string because the signature travels in the
    /// `X-Breez-Signature` header, which keeps it out of proxy and access logs.
    /// The query parameter is the compatibility path for clients predating the
    /// header, and is required again once those are dropped.
    pub signature: Option<String>,
    /// Seconds since the Unix epoch. Travels in `X-Breez-Timestamp`, with the
    /// same query-parameter compatibility path as `signature`.
    pub timestamp: Option<u64>,
    pub offset: Option<u32>,
    pub limit: Option<u32>,
    /// Only return metadata updated after this timestamp (milliseconds)
    pub updated_after: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ListMetadataResponse {
    pub metadata: Vec<ListMetadataMetadata>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ListMetadataMetadata {
    pub payment_hash: String,
    pub sender_comment: Option<String>,
    pub nostr_zap_request: Option<String>,
    /// The zap receipt event (kind 9735) as JSON, if created
    pub nostr_zap_receipt: Option<String>,
    /// Unix timestamp (milliseconds) when this metadata was last updated
    pub updated_at: i64,
    /// The payment preimage if invoice has been paid
    pub preimage: Option<String>,
}

pub fn sanitize_username(username: &str) -> String {
    username.trim().to_lowercase()
}
