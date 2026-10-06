use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::{Payment, PaymentStatus, SignedTransferPackage};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum TokenAllowanceLimit {
    Unlimited,
    Amount { amount: u128 },
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CreateTokenAllowanceRequest {
    pub spender_public_key: String,
    pub token_identifier: String,
    /// The most a single pull may move.
    pub max_per_payment: TokenAllowanceLimit,
    /// The most all pulls together may move.
    pub max_total: TokenAllowanceLimit,
    /// When the allowance expires, in Unix seconds.
    pub expiry_time: u64,
    /// Identity public keys the spender may pay, or empty to allow any recipient.
    pub allowed_recipients: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CreateTokenAllowanceResponse {
    pub allowance: TokenAllowance,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RevokeTokenAllowanceRequest {
    pub allowance_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum TokenAllowanceRole {
    Owner,
    Spender,
}

impl FromStr for TokenAllowanceRole {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "owner" => Ok(Self::Owner),
            "spender" => Ok(Self::Spender),
            _ => Err(format!("Invalid token allowance role '{s}'")),
        }
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ListTokenAllowancesRequest {
    pub role: TokenAllowanceRole,
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub counterparty_public_key: Option<String>,
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub token_identifier: Option<String>,
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub include_inactive: Option<bool>,
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub offset: Option<u32>,
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ListTokenAllowancesResponse {
    pub allowances: Vec<TokenAllowance>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum TokenAllowanceStatus {
    Active,
    Exhausted,
    Expired,
    Revoked,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TokenAllowance {
    pub id: String,
    pub owner_public_key: String,
    pub spender_public_key: String,
    pub token_identifier: String,
    /// The most a single pull may move.
    pub max_per_payment: TokenAllowanceLimit,
    /// The most all pulls together may move.
    pub max_total: TokenAllowanceLimit,
    /// The amount of every pull sent so far, including pulls that later failed, so it can be
    /// higher than what was actually pulled.
    pub spent_amount: u128,
    /// Identity public keys the spender may pay, or empty to allow any recipient.
    pub allowed_recipients: Vec<String>,
    /// When the allowance expires, in Unix seconds.
    pub expiry_time: u64,
    pub created_at: u64,
    pub revoked_at: Option<u64>,
    pub status: TokenAllowanceStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PullReceiver {
    pub amount: u128,
    /// The receiver's identity public key, or unset to pay this wallet. Responses always set it.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub receiver_public_key: Option<String>,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PreparePullPaymentRequest {
    pub payer_public_key: String,
    pub token_identifier: String,
    pub receivers: Vec<PullReceiver>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PreparePullPaymentResponse {
    pub payer_public_key: String,
    pub token_identifier: String,
    pub receivers: Vec<PullReceiver>,
    pub amount: u128,
    pub allowance_id: String,
    /// When the prepared pull stops being sendable, in Unix seconds.
    pub expiry_time: u64,
    /// Opaque data describing the prepared pull, to pass back unchanged.
    pub pull_context: Vec<u8>,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PullPaymentRequest {
    pub prepare_response: PreparePullPaymentResponse,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PullPaymentResponse {
    pub tx_hash: String,
    /// Pending while the pull is accepted but not final yet (repeat the same call to refresh
    /// it), otherwise completed.
    pub status: PaymentStatus,
    /// This wallet's payment from the pull, unset while the pull is pending or when this wallet
    /// isn't one of its receivers.
    pub payment: Option<Payment>,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BuildUnsignedPullPackageRequest {
    pub prepare_response: PreparePullPaymentResponse,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PublishSignedPullPackageRequest {
    pub signed_package: SignedTransferPackage,
}

impl From<TokenAllowanceLimit> for Option<u128> {
    fn from(limit: TokenAllowanceLimit) -> Self {
        match limit {
            TokenAllowanceLimit::Unlimited => None,
            TokenAllowanceLimit::Amount { amount } => Some(amount),
        }
    }
}

impl From<Option<u128>> for TokenAllowanceLimit {
    fn from(limit: Option<u128>) -> Self {
        match limit {
            Some(amount) => Self::Amount { amount },
            None => Self::Unlimited,
        }
    }
}

impl From<TokenAllowanceRole> for spark_wallet::TokenAllowanceRole {
    fn from(role: TokenAllowanceRole) -> Self {
        match role {
            TokenAllowanceRole::Owner => Self::Owner,
            TokenAllowanceRole::Spender => Self::Spender,
        }
    }
}

impl From<spark_wallet::TokenAllowanceStatus> for TokenAllowanceStatus {
    fn from(status: spark_wallet::TokenAllowanceStatus) -> Self {
        match status {
            spark_wallet::TokenAllowanceStatus::Active => Self::Active,
            spark_wallet::TokenAllowanceStatus::Exhausted => Self::Exhausted,
            spark_wallet::TokenAllowanceStatus::Expired => Self::Expired,
            spark_wallet::TokenAllowanceStatus::Revoked => Self::Revoked,
        }
    }
}

impl From<spark_wallet::TokenAllowance> for TokenAllowance {
    fn from(allowance: spark_wallet::TokenAllowance) -> Self {
        Self {
            id: allowance.id,
            owner_public_key: allowance.owner_public_key.to_string(),
            spender_public_key: allowance.spender_public_key.to_string(),
            token_identifier: allowance.token_identifier,
            max_per_payment: allowance.max_per_payment.into(),
            max_total: allowance.max_total.into(),
            spent_amount: allowance.spent_amount,
            allowed_recipients: allowance
                .allowed_recipients
                .iter()
                .map(ToString::to_string)
                .collect(),
            expiry_time: allowance.expiry_time,
            created_at: allowance.created_at,
            revoked_at: allowance.revoked_at,
            status: allowance.status.into(),
        }
    }
}
