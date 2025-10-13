use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct CheckUsernameAvailableResponse {
    pub available: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RecoverLnurlPayRequest {
    pub signature: String,
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
    pub description: String,
    pub nostr_pubkey: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UnregisterLnurlPayRequest {
    pub username: String,
    pub signature: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterLnurlPayResponse {
    pub lnurl: String,
    pub lightning_address: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ListInvoicesRequest {
    pub signature: String,
    pub offset: Option<u32>,
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ListInvoicesResponse {
    pub invoices: Vec<ListInvoicesInvoice>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ListInvoicesInvoice {
    pub invoice: String,
    pub sender_comment: Option<String>,
    pub nostr_zap_request: Option<String>,
}