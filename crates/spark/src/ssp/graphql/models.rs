use crate::ssp::graphql::queries::claim_static_deposit::ClaimStaticDepositClaimStaticDeposit;
use crate::ssp::graphql::queries::complete_coop_exit::{
    CoopExitRequestFragment as CompleteCoopExitCoopExitRequestFragment,
    CurrencyAmountFragment as CompleteCoopExitCurrencyAmountFragment,
    TransferFragment as CompleteCoopExitTransferFragment,
    TransferFragmentUserRequest as CompleteCoopExitUserRequestFragment,
    TransferFragmentUserRequestOn as CompleteCoopExitUserRequestFragmentOn,
};
use crate::ssp::graphql::queries::coop_exit_fee_quote::{
    CoopExitFeeQuoteCoopExitFeeQuoteQuote,
    CurrencyAmountFragment as CoopExitFeeQuoteCurrencyAmountFragment,
};
use crate::ssp::graphql::queries::leaves_swap_fee_estimate::CurrencyAmountFragment as LeavesSwapFeeEstimateCurrencyAmountFragment;
use crate::ssp::graphql::queries::lightning_send_fee_estimate::CurrencyAmountFragment as LightningSendFeeEstimateCurrencyAmountFragment;
use crate::ssp::graphql::queries::request_coop_exit::{
    CoopExitRequestFragment as RequestCoopExitCoopExitRequestFragment,
    CurrencyAmountFragment as RequestCoopExitCurrencyAmountFragment,
    TransferFragment as RequestCoopExitTransferFragment,
    TransferFragmentUserRequest as RequestCoopExitUserRequestFragment,
    TransferFragmentUserRequestOn as RequestCoopExitUserRequestFragmentOn,
};
use crate::ssp::graphql::queries::request_lightning_receive::{
    CurrencyAmountFragment as RequestLightningReceiveCurrencyAmountFragment,
    InvoiceFragment as RequestLightningReceiveInvoiceFragment,
    LightningReceiveRequestFragment as RequestLightningReceiveLightningReceiveRequestFragment,
    TransferFragment as RequestLightningReceiveTransferFragment,
    TransferFragmentUserRequest as RequestLightningReceiveUserRequestFragment,
    TransferFragmentUserRequestOn as RequestLightningReceiveUserRequestFragmentOn,
};
use crate::ssp::graphql::queries::request_lightning_send::{
    CurrencyAmountFragment as RequestLightningSendCurrencyAmountFragment,
    LightningSendRequestFragment as RequestLightningSendLightningSendRequestFragment,
    TransferFragment as RequestLightningSendTransferFragment,
    TransferFragmentUserRequest as RequestLightningSendUserRequestFragment,
    TransferFragmentUserRequestOn as RequestLightningSendUserRequestFragmentOn,
};
use crate::ssp::graphql::queries::request_swap::{
    CurrencyAmountFragment as RequestSwapCurrencyAmountFragment,
    LeavesSwapRequestFragment as RequestSwapLeavesSwapRequestFragment,
    SwapLeafFragment as RequestSwapSwapLeafFragment,
    TransferFragment as RequestSwapTransferFragment,
    TransferFragmentUserRequest as RequestSwapUserRequestFragment,
    TransferFragmentUserRequestOn as RequestSwapUserRequestFragmentOn,
};
use crate::ssp::graphql::queries::static_deposit_quote::StaticDepositQuoteStaticDepositQuote;
use crate::ssp::graphql::queries::transfers::{
    ClaimStaticDepositFragment as TransfersClaimStaticDepositFragment,
    ClaimStaticDepositStatus as TransfersClaimStaticDepositStatus,
    CoopExitRequestFragment as TransfersCoopExitRequestFragment,
    CurrencyAmountFragment as TransferCurrencyAmountFragment, FullTransferFragment,
    InvoiceFragment as TransfersInvoiceFragment,
    LeavesSwapRequestFragment as TransfersLeavesSwapRequestFragment,
    LightningReceiveRequestFragment as TransfersLightningReceiveRequestFragment,
    LightningSendRequestFragment as TransfersLightningSendRequestFragment,
    SwapLeafFragment as TransfersSwapLeafFragment, TransferFragment as TransfersTransferFragment,
    TransferFragmentUserRequest as TransfersTransferFragmentUserRequest,
    TransferFragmentUserRequestOn as TransfersTransferFragmentUserRequestOn,
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
    TransferFragmentUserRequest as UserRequestTransferFragmentUserRequest,
    TransferFragmentUserRequestOn as UserRequestTransferFragmentUserRequestOn,
};
use bitcoin::secp256k1::PublicKey;
use chrono::{DateTime, Utc};
use enum_to_enum::FromEnum;
use serde::{Deserialize, Serialize};

pub use crate::ssp::graphql::queries::claim_static_deposit::ClaimStaticDepositInput;
pub use crate::ssp::graphql::queries::request_coop_exit::RequestCoopExitInput;
pub use crate::ssp::graphql::queries::request_lightning_receive::RequestLightningReceiveInput;
pub use crate::ssp::graphql::queries::request_lightning_send::RequestLightningSendInput;
pub use crate::ssp::graphql::queries::request_swap::RequestSwapInput;
pub use crate::ssp::graphql::queries::request_swap::UserLeafInput;

/// Config for creating a GraphQLClient
#[derive(Debug, Clone)]
pub(crate) struct GraphQLClientConfig {
    /// Base URL for the GraphQL API
    pub base_url: String,
    /// Schema endpoint path (defaults to "graphql/spark/2025-03-19")
    pub schema_endpoint: Option<String>,

    pub ssp_identity_public_key: PublicKey,
    pub user_agent: Option<String>,
}

/// Bitcoin network enum
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BitcoinNetwork {
    Mainnet,
    Testnet,
    Signet,
    Regtest,
}

/// Currency unit enum
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CurrencyUnit {
    Bitcoin,
    Satoshi,
    Millisatoshi,
    Usd,
    Mxn,
    Php,
    Eur,
    Gbp,
    Inr,
    Brl,
    Cad,
    Dkk,
    Hkd,
    Idr,
    Myr,
    Sgd,
    Thb,
    Vnd,
    Ngn,
    Zar,
    Kes,
    Tzs,
    Ugx,
    Bwp,
    Xof,
    Xaf,
    Mwk,
    Rwf,
    Zmw,
    Aed,
    Gtq,
    Usdt,
    Usdc,
    #[serde(other, skip_serializing)]
    Unknown,
}

/// Coop exit request status enum
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SparkCoopExitRequestStatus {
    Initiated,
    CompleteRequestReceived,
    InboundTransferChecked,
    TxBroadcasted,
    OnChainTxConfirmed,
    InboundTransferClaimingFailed,
    Succeeded,
    ExpiringScheduled,
    ExpiringFailed,
    Expired,
    FailingScheduled,
    FailingFailed,
    Failed,
    #[serde(other, skip_serializing)]
    Unknown,
}

/// Leaves swap request status enum
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SparkLeavesSwapRequestStatus {
    Created,
    InboundTransferVerified,
    InboundTransferVerifyingFailed,
    OutboundTransferSent,
    OutboundTransferSendingFailed,
    TransfersCompletingScheduled,
    OutboundTransferCompleted,
    OutboundTransferCompletingFailed,
    InboundTransferClaimingFailed,
    RequestFailingFromVerifyingScheduled,
    RequestFailingFromSendingScheduled,
    RequestFailingFromVerifyingFailed,
    RequestFailingFromSendingFailed,
    Succeeded,
    #[serde(other, skip_serializing)]
    Unknown,
}

/// Lightning receive request status enum
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LightningReceiveRequestStatus {
    InvoiceCreated,
    HtlcReceived,
    TransferCreated,
    TransferCreationFailed,
    PaymentPreimagePending,
    PaymentPreimageRecovered,
    PaymentPreimageQueryingFailed,
    PaymentPreimageRecoveringFailed,
    TransferCanceled,
    HtlcFailed,
    LightningPaymentReceived,
    TransferFailed,
    TransferCompleted,
    RefundSigningCommitmentsQueryingFailed,
    RefundSigningFailed,
    #[serde(other, skip_serializing)]
    Unknown,
}

/// Lightning send request status enum
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LightningSendRequestStatus {
    Created,
    UserTransferValidationFailed,
    LightningPaymentInitiated,
    LightningPaymentFailed,
    LightningPaymentSucceeded,
    PreimageProvided,
    PreimageProvidingFailed,
    TransferCompleted,
    TransferFailed,
    PendingUserSwapReturn,
    UserSwapReturned,
    UserSwapReturnFailed,
    RequestValidated,
    #[serde(other, skip_serializing)]
    Unknown,
}

/// Exit speed enum for cooperative exits
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExitSpeed {
    Fast,
    Medium,
    Slow,
}

/// Claim static deposit request type enum
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ClaimStaticDepositRequestType {
    FixedAmount,
    MaxFee,
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
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[macros::derive_from(RequestLightningReceiveInvoiceFragment)]
#[macros::derive_from(UserRequestInvoiceFragment)]
#[macros::derive_from(TransfersInvoiceFragment)]
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
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[macros::derive_from(CompleteCoopExitCurrencyAmountFragment)]
#[macros::derive_from(CoopExitFeeQuoteCurrencyAmountFragment)]
#[macros::derive_from(LeavesSwapFeeEstimateCurrencyAmountFragment)]
#[macros::derive_from(LightningSendFeeEstimateCurrencyAmountFragment)]
#[macros::derive_from(RequestCoopExitCurrencyAmountFragment)]
#[macros::derive_from(RequestSwapCurrencyAmountFragment)]
#[macros::derive_from(RequestLightningReceiveCurrencyAmountFragment)]
#[macros::derive_from(RequestLightningSendCurrencyAmountFragment)]
#[macros::derive_from(TransferCurrencyAmountFragment)]
#[macros::derive_from(UserRequestCurrencyAmountFragment)]
pub struct CurrencyAmount {
    pub original_value: u64,
    pub original_unit: CurrencyUnit,
    pub preferred_currency_unit: CurrencyUnit,
    pub preferred_currency_value_rounded: u64,
    pub preferred_currency_value_approx: f64,
}

impl Default for CurrencyAmount {
    fn default() -> Self {
        Self {
            original_value: 0,
            original_unit: CurrencyUnit::Satoshi,
            preferred_currency_unit: CurrencyUnit::Satoshi,
            preferred_currency_value_rounded: 0,
            preferred_currency_value_approx: 0.0,
        }
    }
}

/// Transfer structure
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[macros::derive_from(CompleteCoopExitTransferFragment)]
#[macros::derive_from(RequestCoopExitTransferFragment)]
#[macros::derive_from(RequestSwapTransferFragment)]
#[macros::derive_from(RequestLightningReceiveTransferFragment)]
#[macros::derive_from(RequestLightningSendTransferFragment)]
#[macros::derive_from(UserRequestTransferFragment)]
#[macros::derive_from(TransfersTransferFragment)]
pub struct Transfer {
    pub total_amount: CurrencyAmount,
    pub spark_id: Option<String>,
    pub user_request: Option<UserRequest>,
}

/// UserRequest structure
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[macros::derive_from(CompleteCoopExitUserRequestFragment)]
#[macros::derive_from(RequestCoopExitUserRequestFragment)]
#[macros::derive_from(RequestSwapUserRequestFragment)]
#[macros::derive_from(RequestLightningReceiveUserRequestFragment)]
#[macros::derive_from(RequestLightningSendUserRequestFragment)]
#[macros::derive_from(UserRequestTransferFragmentUserRequest)]
#[macros::derive_from(TransfersTransferFragmentUserRequest)]
pub struct UserRequest {
    pub id: String,
    pub on: TransferFragmentUserRequestOn,
}

#[derive(FromEnum, Debug, Clone, Copy, Deserialize, Serialize, PartialEq)]
#[from_enum(RequestCoopExitUserRequestFragmentOn)]
#[from_enum(RequestSwapUserRequestFragmentOn)]
#[from_enum(RequestLightningReceiveUserRequestFragmentOn)]
#[from_enum(RequestLightningSendUserRequestFragmentOn)]
#[from_enum(UserRequestTransferFragmentUserRequestOn)]
#[from_enum(CompleteCoopExitUserRequestFragmentOn)]
#[from_enum(TransfersTransferFragmentUserRequestOn)]
pub enum TransferFragmentUserRequestOn {
    ClaimStaticDeposit,
    CoopExitRequest,
    LeavesSwapRequest,
    LightningReceiveRequest,
    LightningSendRequest,
}

//#[macros::derive_from(TransferTransferFragment)]
#[macros::derive_from(FullTransferFragment)]
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SspTransfer {
    pub total_amount: CurrencyAmount,
    pub spark_id: Option<String>,
    pub user_request: Option<SspUserRequest>,
}

#[derive(FromEnum, Debug, Clone, Deserialize, Serialize, PartialEq)]
#[from_enum(TransferUserRequestFragment)]
pub enum SspUserRequest {
    ClaimStaticDeposit(ClaimStaticDepositInfo),
    CoopExitRequest(CoopExitRequest),
    LeavesSwapRequest(LeavesSwapRequest),
    LightningReceiveRequest(LightningReceiveRequest),
    LightningSendRequest(LightningSendRequest),
}

impl SspUserRequest {
    pub fn get_lightning_invoice(&self) -> Option<String> {
        let invoice = match self {
            SspUserRequest::LightningReceiveRequest(request) => {
                Some(request.invoice.encoded_invoice.clone())
            }
            SspUserRequest::LightningSendRequest(request) => Some(request.encoded_invoice.clone()),
            _ => None,
        };
        invoice.map(|i| i.to_lowercase())
    }

    pub fn get_lightning_preimage(&self) -> Option<String> {
        match self {
            SspUserRequest::LightningReceiveRequest(request) => {
                request.lightning_receive_payment_preimage.clone()
            }
            SspUserRequest::LightningSendRequest(request) => {
                request.lightning_send_payment_preimage.clone()
            }
            _ => None,
        }
    }
}

#[macros::derive_from(TransfersClaimStaticDepositFragment)]
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ClaimStaticDepositInfo {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub network: BitcoinNetwork,
    pub deposit_amount: CurrencyAmount,
    pub deposit_status: ClaimStaticDepositStatus,
    pub credit_amount: CurrencyAmount,
    pub max_fee: CurrencyAmount,
    pub transaction_id: String,
    pub output_index: i64,
    pub bitcoin_network: BitcoinNetwork,
    pub transfer_spark_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ClaimStaticDepositStatus {
    Created,
    TransferCreated,
    TransferCreationFailed,
    TransferCompleted,
    UtxoSwappingFailed,
    SpendTxCreated,
    SpendTxBroadcast,
    SpendTxConfirmed,
    #[serde(other, skip_serializing)]
    Unknown,
}

impl From<TransfersClaimStaticDepositStatus> for ClaimStaticDepositStatus {
    fn from(value: TransfersClaimStaticDepositStatus) -> Self {
        match value {
            TransfersClaimStaticDepositStatus::CREATED => ClaimStaticDepositStatus::Created,
            TransfersClaimStaticDepositStatus::TRANSFER_CREATED => {
                ClaimStaticDepositStatus::TransferCreated
            }
            TransfersClaimStaticDepositStatus::TRANSFER_CREATION_FAILED => {
                ClaimStaticDepositStatus::TransferCreationFailed
            }
            TransfersClaimStaticDepositStatus::UTXO_SWAPPING_FAILED => {
                ClaimStaticDepositStatus::UtxoSwappingFailed
            }
            TransfersClaimStaticDepositStatus::TRANSFER_COMPLETED => {
                ClaimStaticDepositStatus::TransferCompleted
            }
            TransfersClaimStaticDepositStatus::SPEND_TX_CREATED => {
                ClaimStaticDepositStatus::SpendTxCreated
            }
            TransfersClaimStaticDepositStatus::SPEND_TX_BROADCAST => {
                ClaimStaticDepositStatus::SpendTxBroadcast
            }
            TransfersClaimStaticDepositStatus::SPEND_TX_CONFIRMED => {
                ClaimStaticDepositStatus::SpendTxConfirmed
            }
            TransfersClaimStaticDepositStatus::Other(_) => ClaimStaticDepositStatus::Unknown,
        }
    }
}

/// LightningReceiveRequest structure
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[macros::derive_from(RequestLightningReceiveLightningReceiveRequestFragment)]
#[macros::derive_from(UserRequestLightningReceiveRequestFragment)]
#[macros::derive_from(TransfersLightningReceiveRequestFragment)]
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
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[macros::derive_from(RequestLightningSendLightningSendRequestFragment)]
#[macros::derive_from(UserRequestLightningSendRequestFragment)]
#[macros::derive_from(TransfersLightningSendRequestFragment)]
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
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[macros::derive_from(RequestSwapSwapLeafFragment)]
#[macros::derive_from(UserRequestSwapLeafFragment)]
#[macros::derive_from(TransfersSwapLeafFragment)]
pub struct SwapLeaf {
    pub leaf_id: String,
    pub raw_unsigned_refund_transaction: String,
    pub adaptor_signed_signature: String,
}

/// LeavesSwapRequest structure
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[macros::derive_from(RequestSwapLeavesSwapRequestFragment)]
#[macros::derive_from(UserRequestLeavesSwapRequestFragment)]
#[macros::derive_from(TransfersLeavesSwapRequestFragment)]
pub struct LeavesSwapRequest {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub network: BitcoinNetwork,
    pub swap_status: SparkLeavesSwapRequestStatus,
    pub total_amount: CurrencyAmount,
    pub target_amount: CurrencyAmount,
    pub fee: CurrencyAmount,
    pub inbound_transfer: Option<Transfer>,
    pub swap_leaves: Option<Vec<SwapLeaf>>,
    pub outbound_transfer: Option<Transfer>,
    pub swap_expired_at: Option<DateTime<Utc>>,
}

/// CoopExitRequest structure
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[macros::derive_from(CompleteCoopExitCoopExitRequestFragment)]
#[macros::derive_from(RequestCoopExitCoopExitRequestFragment)]
#[macros::derive_from(UserRequestCoopExitRequestFragment)]
#[macros::derive_from(TransfersCoopExitRequestFragment)]
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

/// CoopExitFeeQuote structure
#[derive(Debug, Clone, Deserialize)]
#[macros::derive_from(CoopExitFeeQuoteCoopExitFeeQuoteQuote)]
pub struct CoopExitFeeQuote {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub network: BitcoinNetwork,
    pub total_amount: CurrencyAmount,
    pub user_fee_fast: CurrencyAmount,
    pub user_fee_medium: CurrencyAmount,
    pub user_fee_slow: CurrencyAmount,
    pub l1_broadcast_fee_fast: CurrencyAmount,
    pub l1_broadcast_fee_medium: CurrencyAmount,
    pub l1_broadcast_fee_slow: CurrencyAmount,
}

/// Static deposit quote output
#[derive(Debug, Clone)]
#[macros::derive_from(StaticDepositQuoteStaticDepositQuote)]
pub struct StaticDepositQuote {
    pub transaction_id: String,
    pub output_index: i64,
    pub network: BitcoinNetwork,
    pub credit_amount_sats: u64,
    pub signature: String,
}

/// Claim static deposit output
#[derive(Debug, Clone, Deserialize)]
#[macros::derive_from(ClaimStaticDepositClaimStaticDeposit)]
pub struct ClaimStaticDeposit {
    pub transfer_id: String,
}
