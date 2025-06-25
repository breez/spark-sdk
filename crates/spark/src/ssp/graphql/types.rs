use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Options for creating a GraphQLClient
#[derive(Debug, Clone)]
pub struct GraphQLClientOptions {
    /// Base URL for the GraphQL API
    pub base_url: String,
    /// Schema endpoint path (defaults to "graphql/spark/2025-03-19")
    pub schema_endpoint: Option<String>,
}

/// GraphQL query structure
#[derive(Debug, Serialize)]
pub struct GraphQLQuery {
    pub query: String,
    pub variables: HashMap<String, serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_name: Option<String>,
}

/// GraphQL error structure
#[derive(Debug, Deserialize)]
pub struct GraphQLErrorObject {
    pub message: String,
    #[serde(default)]
    pub locations: Vec<GraphQLErrorLocation>,
    #[serde(default)]
    pub path: Vec<String>,
}

/// GraphQL error location
#[derive(Debug, Deserialize)]
pub struct GraphQLErrorLocation {
    pub line: i32,
    pub column: i32,
}

/// GraphQL response wrapper
#[derive(Debug, Deserialize)]
pub struct GraphQLResponse<T> {
    pub data: Option<T>,
    pub errors: Option<Vec<GraphQLErrorObject>>,
}

/// Bitcoin network enum
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum BitcoinNetwork {
    #[serde(rename = "BITCOIN_NETWORK_MAINNET")]
    Mainnet,
    #[serde(rename = "BITCOIN_NETWORK_TESTNET")]
    Testnet,
    #[serde(rename = "BITCOIN_NETWORK_SIGNET")]
    Signet,
    #[serde(rename = "BITCOIN_NETWORK_REGTEST")]
    Regtest,
}

/// Request status enum
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RequestStatus {
    #[serde(rename = "REQUEST_STATUS_PENDING")]
    Pending,
    #[serde(rename = "REQUEST_STATUS_IN_PROGRESS")]
    InProgress,
    #[serde(rename = "REQUEST_STATUS_COMPLETED")]
    Completed,
    #[serde(rename = "REQUEST_STATUS_FAILED")]
    Failed,
}

/// Exit speed enum for cooperative exits
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum ExitSpeed {
    #[serde(rename = "EXIT_SPEED_NORMAL")]
    Normal,
    #[serde(rename = "EXIT_SPEED_FAST")]
    Fast,
}

/// Claim static deposit request type enum
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum ClaimStaticDepositRequestType {
    #[serde(rename = "CLAIM_STATIC_DEPOSIT_REQUEST_TYPE_FIXED_AMOUNT")]
    FixedAmount,
    #[serde(rename = "CLAIM_STATIC_DEPOSIT_REQUEST_TYPE_FULL_AMOUNT")]
    FullAmount,
}

/// GetChallengeOutput response
#[derive(Debug, Clone, Deserialize)]
pub struct GetChallengeOutput {
    pub protected_challenge: String,
}

/// VerifyChallengeOutput response
#[derive(Debug, Clone, Deserialize)]
pub struct VerifyChallengeOutput {
    pub session_token: String,
    pub valid_until: DateTime<Utc>,
}

/// Lightning invoice structure
#[derive(Debug, Clone, Deserialize)]
pub struct LightningInvoice {
    pub encoded_invoice: String,
    pub payment_hash: String,
    pub amount_sats: i64,
    pub memo: Option<String>,
    pub expiry_timestamp: Option<DateTime<Utc>>,
}

/// LightningReceiveRequest structure
#[derive(Debug, Clone, Deserialize)]
pub struct LightningReceiveRequest {
    pub id: String,
    pub status: RequestStatus,
    pub invoice: LightningInvoice,
}

/// LightningSendRequest structure
#[derive(Debug, Clone, Deserialize)]
pub struct LightningSendRequest {
    pub id: String,
    pub status: RequestStatus,
    pub encoded_invoice: String,
    pub payment_hash: String,
    pub amount_sats: i64,
    pub fee_sats: i64,
    pub preimage: Option<String>,
}

/// LeavesSwapRequest structure
#[derive(Debug, Clone, Deserialize)]
pub struct LeavesSwapRequest {
    pub id: String,
    pub status: RequestStatus,
    pub total_amount_sats: i64,
    pub target_amount_sats: i64,
    pub fee_sats: i64,
    pub user_leaves: Vec<String>,
    pub adaptor_pubkey: Option<String>,
}

/// CoopExitRequest structure
#[derive(Debug, Clone, Deserialize)]
pub struct CoopExitRequest {
    pub id: String,
    pub status: RequestStatus,
    pub leaf_external_ids: Vec<String>,
    pub withdrawal_address: String,
    pub total_amount_sats: i64,
    pub request_fee_sats: i64,
    pub exit_fee_sats: i64,
    pub exit_speed: ExitSpeed,
}

/// Lightning send fee estimate output
#[derive(Debug, Clone, Deserialize)]
pub struct LightningSendFeeEstimateOutput {
    pub fee_sats: i64,
    pub amount_sats: i64,
    pub total_sats: i64,
}

/// Leaves swap fee estimate output
#[derive(Debug, Clone, Deserialize)]
pub struct LeavesSwapFeeEstimateOutput {
    pub fee_sats: i64,
    pub min_per_leaf_amount_sats: i64,
}

/// Cooperative exit fee estimates output
#[derive(Debug, Clone, Deserialize)]
pub struct CoopExitFeeEstimatesOutput {
    pub request_fee_sats: i64,
    pub exit_fee_sats: i64,
    pub min_withdrawal_sats: i64,
}

/// Static deposit quote input
#[derive(Debug, Clone, Serialize)]
pub struct StaticDepositQuoteInput {
    pub transaction_id: String,
    pub output_index: i32,
    pub network: BitcoinNetwork,
}

/// Static deposit quote output
#[derive(Debug, Clone, Deserialize)]
pub struct StaticDepositQuoteOutput {
    pub transaction_id: String,
    pub output_index: i32,
    pub deposit_value_sats: i64,
    pub credit_value_sats: i64,
    pub fee_sats: i64,
}

/// Claim static deposit output
#[derive(Debug, Clone, Deserialize)]
pub struct ClaimStaticDepositOutput {
    pub transaction_id: String,
    pub output_index: i32,
    pub deposit_value_sats: i64,
    pub credit_amount_sats: i64,
    pub fee_sats: i64,
}

/// Request lightning receive input
#[derive(Debug, Clone, Serialize)]
pub struct RequestLightningReceiveInput {
    pub amount_sats: i64,
    pub network: BitcoinNetwork,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiry_secs: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_spark_address: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receiver_identity_pubkey: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description_hash: Option<String>,
}

/// Request lightning send input
#[derive(Debug, Clone, Serialize)]
pub struct RequestLightningSendInput {
    pub encoded_invoice: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

/// Request cooperative exit input
#[derive(Debug, Clone, Serialize)]
pub struct RequestCoopExitInput {
    pub leaf_external_ids: Vec<String>,
    pub withdrawal_address: String,
    pub idempotency_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_speed: Option<ExitSpeed>,
}
