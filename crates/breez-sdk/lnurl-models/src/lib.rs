use serde::{Deserialize, Serialize};

pub mod nostr;

#[derive(Debug, Serialize, Deserialize)]
pub struct CheckUsernameAvailableResponse {
    pub available: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RecoverLnurlPayRequest {
    pub signature: String,
    pub timestamp: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RecoverLnurlPayResponse {
    pub lnurl: String,
    pub lightning_address: String,
    pub username: String,
    pub description: String,
    pub nostr_pubkey: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterLnurlPayRequest {
    pub username: String,
    pub signature: String,
    pub timestamp: Option<u64>,
    pub description: String,
    pub nostr_pubkey: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UnregisterLnurlPayRequest {
    pub username: String,
    pub signature: String,
    pub timestamp: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterLnurlPayResponse {
    pub lnurl: String,
    pub lightning_address: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ListMetadataRequest {
    pub signature: String,
    pub timestamp: Option<u64>,
    pub offset: Option<u32>,
    pub limit: Option<u32>,
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
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PublishZapReceiptRequest {
    pub signature: String,
    pub timestamp: Option<u64>,
    pub zap_receipt: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InvoicePaidRequest {
    pub signature: String,
    pub timestamp: Option<u64>,
    pub preimage: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PublishZapReceiptResponse {
    pub published: bool,
    pub zap_receipt: String,
}

pub fn sanitize_username(username: &str) -> String {
    username.trim().to_lowercase()
}
