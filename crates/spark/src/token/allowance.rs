use bitcoin::secp256k1::PublicKey;
use platform_utils::time::SystemTime;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenAllowanceFailure {
    NotEnabled,
    NotFound,
    Revoked,
    Expired,
    NotSpendable,
    OverPerPaymentLimit,
    OverTotalLimit,
    RecipientNotAllowed,
    PayerInsufficientFunds,
    AlreadyActive,
    QuotaExceeded,
    PreparedPullStale,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenAllowanceStatus {
    Active,
    Exhausted,
    Expired,
    Revoked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenAllowanceRole {
    Owner,
    Spender,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenAllowance {
    pub id: String,
    pub owner_public_key: PublicKey,
    pub spender_public_key: PublicKey,
    pub token_identifier: String,
    pub max_per_payment: Option<u128>,
    pub max_total: Option<u128>,
    pub spent_amount: u128,
    pub allowed_recipients: Vec<PublicKey>,
    pub expiry_time: u64,
    pub created_at: u64,
    pub revoked_at: Option<u64>,
    pub status: TokenAllowanceStatus,
}

#[derive(Clone, Debug)]
pub struct NewTokenAllowance {
    pub spender_public_key: PublicKey,
    pub token_identifier: String,
    pub max_per_payment: Option<u128>,
    pub max_total: Option<u128>,
    pub expiry_time: u64,
    pub allowed_recipients: Vec<PublicKey>,
}

#[derive(Clone, Debug)]
pub struct TokenAllowanceQuery {
    pub role: TokenAllowanceRole,
    pub counterparty_public_key: Option<PublicKey>,
    pub token_identifier: Option<String>,
    pub include_inactive: bool,
    pub offset: u32,
    pub limit: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullReceiver {
    pub receiver_public_key: PublicKey,
    pub amount: u128,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PreparedTokenPull {
    pub allowance_id: String,
    pub payer_public_key: PublicKey,
    pub token_identifier: String,
    pub receivers: Vec<PullReceiver>,
    pub partial_token_transaction_bytes: Vec<u8>,
    pub partial_token_transaction_hash: Vec<u8>,
    pub created_timestamp: SystemTime,
    pub allowance_expiry_time: u64,
}
