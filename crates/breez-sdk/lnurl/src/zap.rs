#[derive(Debug, Clone)]
pub struct Zap {
    pub payment_hash: String,
    pub zap_request: String,
    pub zap_event: Option<String>,
    pub user_pubkey: String,
    pub invoice_expiry: i64,
    pub updated_at: i64,
    pub is_user_nostr_key: bool,
}
