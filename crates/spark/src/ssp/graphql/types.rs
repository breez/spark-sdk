use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Config for creating a GraphQLClient
#[derive(Debug, Clone)]
pub(crate) struct GraphQLClientConfig {
    /// Base URL for the GraphQL API
    pub base_url: String,
    /// Schema endpoint path (defaults to "graphql/spark/2025-03-19")
    pub schema_endpoint: Option<String>,
}

/// GraphQL query structure
#[derive(Debug, Serialize)]
pub(crate) struct GraphQLQuery {
    pub query: String,
    pub variables: HashMap<String, serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_name: Option<String>,
}

/// GraphQL error structure
#[derive(Debug, Deserialize)]
pub(crate) struct GraphQLErrorObject {
    pub message: String,
    #[serde(default)]
    pub locations: Vec<GraphQLErrorLocation>,
    #[serde(default)]
    pub path: Vec<String>,
}

/// GraphQL error location
#[derive(Debug, Deserialize)]
pub(crate) struct GraphQLErrorLocation {
    pub line: u32,
    pub column: u32,
}

/// GraphQL response wrapper
#[derive(Debug, Deserialize)]
pub(crate) struct GraphQLResponse<T> {
    pub data: Option<T>,
    pub errors: Option<Vec<GraphQLErrorObject>>,
}

/// Deserialize helper function that returns a default value if deserialization fails
fn ok_or_default<'de, T, D>(deserializer: D) -> Result<T, D::Error>
where
    T: Deserialize<'de> + Default,
    D: Deserializer<'de>,
{
    let v: Value = Deserialize::deserialize(deserializer)?;
    Ok(T::deserialize(v).unwrap_or_default())
}

/// Bitcoin network enum
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BitcoinNetwork {
    Mainnet,
    Testnet,
    Signet,
    Regtest,
}

/// Coop exit request status enum
#[spark_macros::add_unknown_variant]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CoopExitRequestStatus {
    Initiated,
    InboundTransferChecked,
    TxSigned,
    TxBroadcasted,
    WaitingOnTxConfirmations,
    Succeeded,
    Expired,
    Failed,
}

#[spark_macros::add_unknown_variant]
/// Leaves swap request status enum
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LeavesSwapRequestStatus {
    InvoiceCreated,
    TransferCreated,
    TransferCreationFailed,
    RefundSigningCommitmentsQueryingFailed,
    RefundSigningFailed,
    PaymentPreimageRecovered,
    PaymentPreimageRecoveringFailed,
    LightningPaymentReceived,
    TransferFailed,
    TransferCompleted,
}

#[spark_macros::add_unknown_variant]
/// Lightning receive request status enum
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LightningReceiveRequestStatus {
    InvoiceCreated,
    TransferCreated,
    TransferCreationFailed,
    RefundSigningCommitmentsQueryingFailed,
    RefundSigningFailed,
    PaymentPreimageRecovered,
    PaymentPreimageRecoveringFailed,
    LightningPaymentReceived,
    TransferFailed,
    TransferCompleted,
}

#[spark_macros::add_unknown_variant]
/// Lightning send request status enum
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LightningSendRequestStatus {
    Created,
    RequestValidated,
    LightningPaymentInitiated,
    LightningPaymentFailed,
    LightningPaymentSucceeded,
    PreimageProvided,
    TransferCompleted,
}

/// Exit speed enum for cooperative exits
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExitSpeed {
    Normal,
    Fast,
}

/// Claim static deposit request type enum
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ClaimStaticDepositRequestType {
    FixedAmount,
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
    pub bitcoin_network: BitcoinNetwork,
    pub payment_hash: String,
    pub amount: CurrencyAmount,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub memo: Option<String>,
}

/// Currency amount structure
#[derive(Debug, Clone, Deserialize)]
pub struct CurrencyAmount {
    pub original_value: u64,
    pub original_unit: String,
    pub preferred_currency_unit: String,
    pub preferred_currency_value_rounded: u64,
    pub preferred_currency_value_approx: f64,
}

/// Transfer structure
#[derive(Debug, Clone, Deserialize)]
pub struct Transfer {
    pub total_amount: CurrencyAmount,
    pub spark_id: Option<String>,
    pub user_request: Option<UserRequest>,
}

/// UserRequest structure
#[derive(Debug, Clone, Deserialize)]
pub struct UserRequest {
    pub id: String,
}

/// LightningReceiveRequest structure
#[derive(Debug, Clone, Deserialize)]
pub struct LightningReceiveRequest {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub network: BitcoinNetwork,
    #[serde(deserialize_with = "ok_or_default")]
    pub status: LightningReceiveRequestStatus,
    pub invoice: LightningInvoice,
    pub transfer: Option<Transfer>,
    pub payment_preimage: Option<String>,
}

/// LightningSendRequest structure
#[derive(Debug, Clone, Deserialize)]
pub struct LightningSendRequest {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub network: BitcoinNetwork,
    pub encoded_invoice: String,
    pub fee: CurrencyAmount,
    pub idempotency_key: String,
    #[serde(deserialize_with = "ok_or_default")]
    pub status: LightningSendRequestStatus,
    pub transfer: Option<Transfer>,
    pub payment_preimage: Option<String>,
}

/// SwapLeaf structure
#[derive(Debug, Clone, Deserialize)]
pub struct SwapLeaf {
    pub leaf_id: String,
    pub raw_unsigned_refund_transaction: String,
    pub adaptor_signed_signature: String,
}

/// UserLeaf structure
#[derive(Debug, Clone, Serialize)]
pub struct UserLeaf {
    pub leaf_id: String,
    pub raw_unsigned_refund_transaction: String,
    pub adaptor_added_signature: String,
}

/// LeavesSwapRequest structure
#[derive(Debug, Clone, Deserialize)]
pub struct LeavesSwapRequest {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub network: BitcoinNetwork,
    #[serde(deserialize_with = "ok_or_default")]
    pub status: LeavesSwapRequestStatus,
    pub total_amount: CurrencyAmount,
    pub target_amount: CurrencyAmount,
    pub fee: CurrencyAmount,
    pub inbound_transfer: Transfer,
    pub swap_leaves: Vec<SwapLeaf>,
    pub outbound_transfer: Option<Transfer>,
    pub expires_at: Option<String>,
}

/// CoopExitRequest structure
#[derive(Debug, Clone, Deserialize)]
pub struct CoopExitRequest {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub network: BitcoinNetwork,
    pub fee: CurrencyAmount,
    pub l1_broadcast_fee: CurrencyAmount,
    #[serde(deserialize_with = "ok_or_default")]
    pub status: CoopExitRequestStatus,
    pub expires_at: String,
    pub raw_connector_transaction: String,
    pub raw_coop_exit_transaction: String,
    pub coop_exit_txid: String,
    pub transfer: Option<Transfer>,
}

/// Lightning send fee estimate output
#[derive(Debug, Clone, Deserialize)]
pub struct LightningSendFeeEstimateOutput {
    pub fee_estimate: CurrencyAmount,
}

/// CoopExitFeeEstimate structure
#[derive(Debug, Clone, Deserialize)]
pub struct CoopExitFeeEstimate {
    pub user_fee: CurrencyAmount,
    pub l1_broadcast_fee: CurrencyAmount,
}

/// CoopExitFeeEstimatesOutput structure
#[derive(Debug, Clone, Deserialize)]
pub struct CoopExitFeeEstimatesOutput {
    pub speed_fast: Option<CoopExitFeeEstimate>,
    pub speed_medium: Option<CoopExitFeeEstimate>,
    pub speed_slow: Option<CoopExitFeeEstimate>,
}

/// Leaves swap fee estimate output
#[derive(Debug, Clone, Deserialize)]
pub struct LeavesSwapFeeEstimateOutput {
    pub fee_estimate: CurrencyAmount,
}

/// Static deposit quote output
#[derive(Debug, Clone, Deserialize)]
pub struct StaticDepositQuoteOutput {
    pub transaction_id: String,
    pub output_index: u32,
    pub network: BitcoinNetwork,
    pub credit_amount_sats: u64,
    pub signature: String,
}

/// Claim static deposit output
#[derive(Debug, Clone, Deserialize)]
pub struct ClaimStaticDepositOutput {
    pub transfer_id: String,
}

/// Request lightning receive input
#[derive(Debug, Clone, Serialize)]
pub struct RequestLightningReceiveInput {
    pub amount_sats: u64,
    pub network: BitcoinNetwork,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiry_secs: Option<u32>,
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

/// Request leaves swap input
#[derive(Debug, Clone, Serialize)]
pub struct RequestLeavesSwapInput {
    pub adaptor_pubkey: String,
    pub total_amount_sats: u64,
    pub target_amount_sats: u64,
    pub fee_sats: u64,
    pub user_leaves: Vec<UserLeaf>,
    pub idempotency_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_amount_sats_list: Option<Vec<u64>>,
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

/// Claim static deposit input
#[derive(Debug, Clone, Serialize)]
pub struct ClaimStaticDepositInput {
    pub transaction_id: String,
    pub output_index: u32,
    pub network: BitcoinNetwork,
    pub request_type: ClaimStaticDepositRequestType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit_amount_sats: Option<u64>,
    pub deposit_secret_key: String,
    pub signature: String,
    pub quote_signature: String,
}
