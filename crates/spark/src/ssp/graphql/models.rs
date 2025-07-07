use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ssp::graphql::queries::claim_static_deposit::ClaimStaticDepositClaimStaticDeposit;
use crate::ssp::graphql::queries::complete_coop_exit::{
    CoopExitRequestFragment as CompleteCoopExitCoopExitRequestFragment,
    CurrencyAmountFragment as CompleteCoopExitCurrencyAmountFragment,
    TransferFragment as CompleteCoopExitTransferFragment,
    UserRequestFragment as CompleteCoopExitUserRequestFragment,
};
use crate::ssp::graphql::queries::complete_leaves_swap::{
    CurrencyAmountFragment as CompleteLeavesSwapCurrencyAmountFragment,
    LeavesSwapRequestFragment as CompleteLeavesSwapLeavesSwapRequestFragment,
    SwapLeafFragment as CompleteLeavesSwapSwapLeafFragment,
    TransferFragment as CompleteLeavesSwapTransferFragment,
    UserRequestFragment as CompleteLeavesSwapUserRequestFragment,
};
use crate::ssp::graphql::queries::coop_exit_fee_estimates::{
    CoopExitFeeEstimateFragment, CoopExitFeeEstimatesCoopExitFeeEstimates,
    CurrencyAmountFragment as CoopExitFeeEstimatesCurrencyAmountFragment,
};
use crate::ssp::graphql::queries::leaves_swap_fee_estimate::CurrencyAmountFragment as LeavesSwapFeeEstimateCurrencyAmountFragment;
use crate::ssp::graphql::queries::lightning_send_fee_estimate::CurrencyAmountFragment as LightningSendFeeEstimateCurrencyAmountFragment;
use crate::ssp::graphql::queries::request_coop_exit::{
    CoopExitRequestFragment as RequestCoopExitCoopExitRequestFragment,
    CurrencyAmountFragment as RequestCoopExitCurrencyAmountFragment,
    TransferFragment as RequestCoopExitTransferFragment,
    UserRequestFragment as RequestCoopExitUserRequestFragment,
};
use crate::ssp::graphql::queries::request_leaves_swap::{
    CurrencyAmountFragment as RequestLeavesSwapCurrencyAmountFragment,
    LeavesSwapRequestFragment as RequestLeavesSwapLeavesSwapRequestFragment,
    SwapLeafFragment as RequestLeavesSwapSwapLeafFragment,
    TransferFragment as RequestLeavesSwapTransferFragment,
    UserRequestFragment as RequestLeavesSwapUserRequestFragment,
};
use crate::ssp::graphql::queries::request_lightning_receive::{
    CurrencyAmountFragment as RequestLightningReceiveCurrencyAmountFragment,
    InvoiceFragment as RequestLightningReceiveInvoiceFragment,
    LightningReceiveRequestFragment as RequestLightningReceiveLightningReceiveRequestFragment,
    TransferFragment as RequestLightningReceiveTransferFragment,
    UserRequestFragment as RequestLightningReceiveUserRequestFragment,
};
use crate::ssp::graphql::queries::request_lightning_send::{
    CurrencyAmountFragment as RequestLightningSendCurrencyAmountFragment,
    LightningSendRequestFragment as RequestLightningSendLightningSendRequestFragment,
    TransferFragment as RequestLightningSendTransferFragment,
    UserRequestFragment as RequestLightningSendUserRequestFragment,
};
use crate::ssp::graphql::queries::static_deposit_quote::StaticDepositQuoteStaticDepositQuote;
use crate::ssp::graphql::queries::transfer::{
    CurrencyAmountFragment as TransferCurrencyAmountFragment,
    TransferFragment as TransferTransferFragment,
    UserRequestFragment as TransferUserRequestFragment,
};
use crate::ssp::graphql::queries::user_request::{
    CoopExitRequestFragment as UserRequestCoopExitRequestFragment,
    CurrencyAmountFragment as UserRequestCurrencyAmountFragment,
    InvoiceFragment as UserRequestInvoiceFragment,
    LeavesSwapRequestFragment as UserRequestLeavesSwapRequestFragment,
    LightningReceiveRequestFragment as UserRequestLightningReceiveRequestFragment,
    LightningSendRequestFragment as UserRequestLightningSendRequestFragment,
    SwapLeafFragment as UserRequestSwapLeafFragment,
    TransferFragment as UserRequestTransferFragment,
    UserRequestFragment as UserRequestUserRequestFragment,
};

pub use crate::ssp::graphql::queries::claim_static_deposit::ClaimStaticDepositInput;
pub use crate::ssp::graphql::queries::request_coop_exit::RequestCoopExitInput;
pub use crate::ssp::graphql::queries::request_leaves_swap::RequestLeavesSwapInput;
pub use crate::ssp::graphql::queries::request_leaves_swap::UserLeafInput;
pub use crate::ssp::graphql::queries::request_lightning_receive::RequestLightningReceiveInput;
pub use crate::ssp::graphql::queries::request_lightning_send::RequestLightningSendInput;

/// Config for creating a GraphQLClient
#[derive(Debug, Clone)]
pub(crate) struct GraphQLClientConfig {
    /// Base URL for the GraphQL API
    pub base_url: String,
    /// Schema endpoint path (defaults to "graphql/spark/2025-03-19")
    pub schema_endpoint: Option<String>,
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

/// Currency unit enum
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CurrencyUnit {
    Satoshi,
    Millisatoshi,
    Bitcoin,
    Fiat,
    #[serde(other, skip_serializing)]
    Unknown,
}

/// Coop exit request status enum
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SparkCoopExitRequestStatus {
    Initiated,
    InboundTransferChecked,
    TxSigned,
    TxBroadcasted,
    WaitingOnTxConfirmations,
    Succeeded,
    Expired,
    Failed,
    #[serde(other, skip_serializing)]
    Unknown,
}

/// Leaves swap request status enum
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SparkLeavesSwapRequestStatus {
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
    #[serde(other, skip_serializing)]
    Unknown,
}

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
    #[serde(other, skip_serializing)]
    Unknown,
}

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
    #[serde(other, skip_serializing)]
    Unknown,
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
#[spark_macros::derive_from(RequestLightningReceiveInvoiceFragment)]
#[spark_macros::derive_from(UserRequestInvoiceFragment)]
pub struct LightningInvoice {
    pub encoded_invoice: String,
    pub bitcoin_network: BitcoinNetwork,
    pub payment_hash: String,
    pub amount: CurrencyAmount,
    pub created_at: DateTime<Utc>,
    pub invoice_expired_at: DateTime<Utc>,
    pub memo: Option<String>,
}

/// Currency amount structure
#[derive(Debug, Clone, Deserialize)]
#[spark_macros::derive_from(CompleteCoopExitCurrencyAmountFragment)]
#[spark_macros::derive_from(CompleteLeavesSwapCurrencyAmountFragment)]
#[spark_macros::derive_from(CoopExitFeeEstimatesCurrencyAmountFragment)]
#[spark_macros::derive_from(LeavesSwapFeeEstimateCurrencyAmountFragment)]
#[spark_macros::derive_from(LightningSendFeeEstimateCurrencyAmountFragment)]
#[spark_macros::derive_from(RequestCoopExitCurrencyAmountFragment)]
#[spark_macros::derive_from(RequestLeavesSwapCurrencyAmountFragment)]
#[spark_macros::derive_from(RequestLightningReceiveCurrencyAmountFragment)]
#[spark_macros::derive_from(RequestLightningSendCurrencyAmountFragment)]
#[spark_macros::derive_from(TransferCurrencyAmountFragment)]
#[spark_macros::derive_from(UserRequestCurrencyAmountFragment)]
pub struct CurrencyAmount {
    pub original_value: u64,
    pub original_unit: CurrencyUnit,
    pub preferred_currency_unit: CurrencyUnit,
    pub preferred_currency_value_rounded: u64,
    pub preferred_currency_value_approx: f64,
}

/// Transfer structure
#[derive(Debug, Clone, Deserialize)]
#[spark_macros::derive_from(CompleteCoopExitTransferFragment)]
#[spark_macros::derive_from(CompleteLeavesSwapTransferFragment)]
#[spark_macros::derive_from(RequestCoopExitTransferFragment)]
#[spark_macros::derive_from(RequestLeavesSwapTransferFragment)]
#[spark_macros::derive_from(RequestLightningReceiveTransferFragment)]
#[spark_macros::derive_from(RequestLightningSendTransferFragment)]
#[spark_macros::derive_from(TransferTransferFragment)]
#[spark_macros::derive_from(UserRequestTransferFragment)]
pub struct Transfer {
    pub total_amount: CurrencyAmount,
    pub spark_id: Option<String>,
    pub user_request: Option<UserRequest>,
}

/// UserRequest structure
#[derive(Debug, Clone, Deserialize)]
#[spark_macros::derive_from(CompleteCoopExitUserRequestFragment)]
#[spark_macros::derive_from(CompleteLeavesSwapUserRequestFragment)]
#[spark_macros::derive_from(RequestCoopExitUserRequestFragment)]
#[spark_macros::derive_from(RequestLeavesSwapUserRequestFragment)]
#[spark_macros::derive_from(RequestLightningReceiveUserRequestFragment)]
#[spark_macros::derive_from(RequestLightningSendUserRequestFragment)]
#[spark_macros::derive_from(TransferUserRequestFragment)]
#[spark_macros::derive_from(UserRequestUserRequestFragment)]
pub struct UserRequest {
    pub id: String,
}

/// LightningReceiveRequest structure
#[derive(Debug, Clone, Deserialize)]
#[spark_macros::derive_from(RequestLightningReceiveLightningReceiveRequestFragment)]
#[spark_macros::derive_from(UserRequestLightningReceiveRequestFragment)]
pub struct LightningReceiveRequest {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub network: BitcoinNetwork,
    pub lightning_request_status: LightningReceiveRequestStatus,
    pub invoice: LightningInvoice,
    pub transfer: Option<Transfer>,
    pub lightning_receive_payment_preimage: Option<String>,
}

/// LightningSendRequest structure
#[derive(Debug, Clone, Deserialize)]
#[spark_macros::derive_from(RequestLightningSendLightningSendRequestFragment)]
#[spark_macros::derive_from(UserRequestLightningSendRequestFragment)]
pub struct LightningSendRequest {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub network: BitcoinNetwork,
    pub encoded_invoice: String,
    pub fee: CurrencyAmount,
    pub idempotency_key: String,
    pub status: LightningSendRequestStatus,
    pub transfer: Option<Transfer>,
    pub lightning_send_payment_preimage: Option<String>,
}

/// SwapLeaf structure
#[derive(Debug, Clone, Deserialize)]
#[spark_macros::derive_from(CompleteLeavesSwapSwapLeafFragment)]
#[spark_macros::derive_from(RequestLeavesSwapSwapLeafFragment)]
#[spark_macros::derive_from(UserRequestSwapLeafFragment)]
pub struct SwapLeaf {
    pub leaf_id: String,
    pub raw_unsigned_refund_transaction: String,
    pub adaptor_signed_signature: String,
}

/// LeavesSwapRequest structure
#[derive(Debug, Clone, Deserialize)]
#[spark_macros::derive_from(CompleteLeavesSwapLeavesSwapRequestFragment)]
#[spark_macros::derive_from(RequestLeavesSwapLeavesSwapRequestFragment)]
#[spark_macros::derive_from(UserRequestLeavesSwapRequestFragment)]
pub struct LeavesSwapRequest {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub network: BitcoinNetwork,
    pub swap_status: SparkLeavesSwapRequestStatus,
    pub total_amount: CurrencyAmount,
    pub target_amount: CurrencyAmount,
    pub fee: CurrencyAmount,
    pub inbound_transfer: Transfer,
    pub swap_leaves: Vec<SwapLeaf>,
    pub outbound_transfer: Option<Transfer>,
    pub swap_expired_at: Option<DateTime<Utc>>,
}

/// CoopExitRequest structure
#[derive(Debug, Clone)]
#[spark_macros::derive_from(CompleteCoopExitCoopExitRequestFragment)]
#[spark_macros::derive_from(RequestCoopExitCoopExitRequestFragment)]
#[spark_macros::derive_from(UserRequestCoopExitRequestFragment)]
pub struct CoopExitRequest {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub network: BitcoinNetwork,
    pub fee: CurrencyAmount,
    pub l1_broadcast_fee: CurrencyAmount,
    pub exit_status: SparkCoopExitRequestStatus,
    pub exit_expired_at: DateTime<Utc>,
    pub raw_connector_transaction: String,
    pub raw_coop_exit_transaction: String,
    pub coop_exit_txid: String,
    pub transfer: Option<Transfer>,
}

/// CoopExitFeeEstimate structure
#[derive(Debug, Clone, Deserialize)]
#[spark_macros::derive_from(CoopExitFeeEstimateFragment)]
pub struct CoopExitFeeEstimate {
    pub user_fee: CurrencyAmount,
    pub l1_broadcast_fee: CurrencyAmount,
}

/// CoopExitFeeEstimatesOutput structure
#[derive(Debug, Clone, Deserialize)]
#[spark_macros::derive_from(CoopExitFeeEstimatesCoopExitFeeEstimates)]
pub struct CoopExitFeeEstimates {
    pub speed_fast: Option<CoopExitFeeEstimate>,
    pub speed_medium: Option<CoopExitFeeEstimate>,
    pub speed_slow: Option<CoopExitFeeEstimate>,
}

/// Static deposit quote output
#[derive(Debug, Clone)]
#[spark_macros::derive_from(StaticDepositQuoteStaticDepositQuote)]
pub struct StaticDepositQuote {
    pub transaction_id: String,
    pub output_index: i64,
    pub network: BitcoinNetwork,
    pub credit_amount_sats: u64,
    pub signature: String,
}

/// Claim static deposit output
#[derive(Debug, Clone, Deserialize)]
#[spark_macros::derive_from(ClaimStaticDepositClaimStaticDeposit)]
pub struct ClaimStaticDeposit {
    pub transfer_id: String,
}
