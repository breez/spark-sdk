#![allow(clippy::wildcard_imports)]

use async_graphql::*;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::scalars::{Hash32, Long, PublicKey};

#[derive(Enum, Copy, Clone, Eq, PartialEq, Debug)]
pub enum BitcoinNetwork {
    #[graphql(name = "MAINNET")]
    Mainnet,
    #[graphql(name = "REGTEST")]
    Regtest,
    #[graphql(name = "SIGNET")]
    Signet,
    #[graphql(name = "TESTNET")]
    Testnet,
}

impl From<BitcoinNetwork> for spark::Network {
    fn from(network: BitcoinNetwork) -> Self {
        match network {
            BitcoinNetwork::Mainnet => spark::Network::Mainnet,
            BitcoinNetwork::Regtest => spark::Network::Regtest,
            BitcoinNetwork::Signet => spark::Network::Signet,
            BitcoinNetwork::Testnet => spark::Network::Testnet,
        }
    }
}

#[derive(Enum, Copy, Clone, Eq, PartialEq, Debug)]
pub enum ClaimStaticDepositRequestType {
    #[graphql(name = "FIXED_AMOUNT")]
    FixedAmount,
    #[graphql(name = "MAX_FEE")]
    MaxFee,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq, Debug)]
pub enum ClaimStaticDepositStatus {
    #[graphql(name = "CREATED")]
    Created,
    #[graphql(name = "TRANSFER_CREATED")]
    TransferCreated,
    #[graphql(name = "TRANSFER_CREATION_FAILED")]
    TransferCreationFailed,
    #[graphql(name = "TRANSFER_COMPLETED")]
    TransferCompleted,
    #[graphql(name = "UTXO_SWAPPING_FAILED")]
    UtxoSwappingFailed,
    #[graphql(name = "SPEND_TX_CREATED")]
    SpendTxCreated,
    #[graphql(name = "SPEND_TX_BROADCAST")]
    SpendTxBroadcast,
    #[graphql(name = "SPEND_TX_CONFIRMED")]
    SpendTxConfirmed,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq, Debug)]
pub enum CurrencyUnit {
    #[graphql(name = "BITCOIN")]
    Bitcoin,
    #[graphql(name = "SATOSHI")]
    Satoshi,
    #[graphql(name = "MILLISATOSHI")]
    Millisatoshi,
    #[graphql(name = "USD")]
    Usd,
    #[graphql(name = "MXN")]
    Mxn,
    #[graphql(name = "PHP")]
    Php,
    #[graphql(name = "EUR")]
    Eur,
    #[graphql(name = "GBP")]
    Gbp,
    #[graphql(name = "INR")]
    Inr,
    #[graphql(name = "BRL")]
    Brl,
    #[graphql(name = "CAD")]
    Cad,
    #[graphql(name = "DKK")]
    Dkk,
    #[graphql(name = "HKD")]
    Hkd,
    #[graphql(name = "IDR")]
    Idr,
    #[graphql(name = "MYR")]
    Myr,
    #[graphql(name = "SGD")]
    Sgd,
    #[graphql(name = "THB")]
    Thb,
    #[graphql(name = "VND")]
    Vnd,
    #[graphql(name = "NGN")]
    Ngn,
    #[graphql(name = "ZAR")]
    Zar,
    #[graphql(name = "KES")]
    Kes,
    #[graphql(name = "TZS")]
    Tzs,
    #[graphql(name = "UGX")]
    Ugx,
    #[graphql(name = "BWP")]
    Bwp,
    #[graphql(name = "XOF")]
    Xof,
    #[graphql(name = "XAF")]
    Xaf,
    #[graphql(name = "MWK")]
    Mwk,
    #[graphql(name = "RWF")]
    Rwf,
    #[graphql(name = "ZMW")]
    Zmw,
    #[graphql(name = "AED")]
    Aed,
    #[graphql(name = "GTQ")]
    Gtq,
    #[graphql(name = "USDT")]
    Usdt,
    #[graphql(name = "USDC")]
    Usdc,
    #[graphql(name = "NANOBITCOIN", deprecation = "Use BITCOIN instead.")]
    Nanobitcoin,
    #[graphql(name = "MICROBITCOIN", deprecation = "Use BITCOIN instead.")]
    Microbitcoin,
    #[graphql(name = "MILLIBITCOIN", deprecation = "Use BITCOIN instead.")]
    Millibitcoin,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq, Debug)]
pub enum ExitSpeed {
    #[graphql(name = "FAST")]
    Fast,
    #[graphql(name = "MEDIUM")]
    Medium,
    #[graphql(name = "SLOW")]
    Slow,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq, Debug)]
pub enum LightningReceiveRequestStatus {
    #[graphql(name = "INVOICE_CREATED")]
    InvoiceCreated,
    #[graphql(name = "HTLC_RECEIVED")]
    HtlcReceived,
    #[graphql(name = "TRANSFER_CREATED")]
    TransferCreated,
    #[graphql(name = "TRANSFER_CREATION_FAILED")]
    TransferCreationFailed,
    #[graphql(name = "PAYMENT_PREIMAGE_PENDING")]
    PaymentPreimagePending,
    #[graphql(name = "PAYMENT_PREIMAGE_RECOVERED")]
    PaymentPreimageRecovered,
    #[graphql(name = "PAYMENT_PREIMAGE_QUERYING_FAILED")]
    PaymentPreimageQueryingFailed,
    #[graphql(name = "PAYMENT_PREIMAGE_RECOVERING_FAILED")]
    PaymentPreimageRecoveringFailed,
    #[graphql(name = "TRANSFER_CANCELED")]
    TransferCanceled,
    #[graphql(name = "HTLC_FAILED")]
    HtlcFailed,
    #[graphql(name = "LIGHTNING_PAYMENT_RECEIVED")]
    LightningPaymentReceived,
    #[graphql(name = "TRANSFER_FAILED")]
    TransferFailed,
    #[graphql(name = "TRANSFER_COMPLETED")]
    TransferCompleted,
    #[graphql(name = "REFUND_SIGNING_COMMITMENTS_QUERYING_FAILED")]
    RefundSigningCommitmentsQueryingFailed,
    #[graphql(name = "REFUND_SIGNING_FAILED")]
    RefundSigningFailed,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq, Debug)]
pub enum LightningSendRequestStatus {
    #[graphql(name = "CREATED")]
    Created,
    #[graphql(name = "USER_TRANSFER_VALIDATION_FAILED")]
    UserTransferValidationFailed,
    #[graphql(name = "LIGHTNING_PAYMENT_INITIATED")]
    LightningPaymentInitiated,
    #[graphql(name = "LIGHTNING_PAYMENT_FAILED")]
    LightningPaymentFailed,
    #[graphql(name = "LIGHTNING_PAYMENT_SUCCEEDED")]
    LightningPaymentSucceeded,
    #[graphql(name = "PREIMAGE_PROVIDED")]
    PreimageProvided,
    #[graphql(name = "PREIMAGE_PROVIDING_FAILED")]
    PreimageProvidingFailed,
    #[graphql(name = "TRANSFER_COMPLETED")]
    TransferCompleted,
    #[graphql(name = "TRANSFER_FAILED")]
    TransferFailed,
    #[graphql(name = "PENDING_USER_SWAP_RETURN")]
    PendingUserSwapReturn,
    #[graphql(name = "USER_SWAP_RETURNED")]
    UserSwapReturned,
    #[graphql(name = "USER_SWAP_RETURN_FAILED")]
    UserSwapReturnFailed,
    #[graphql(name = "REQUEST_VALIDATED", deprecation = "Use CREATED instead")]
    RequestValidated,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq, Debug)]
pub enum SparkCoopExitRequestStatus {
    #[graphql(name = "INITIATED")]
    Initiated,
    #[graphql(name = "COMPLETE_REQUEST_RECEIVED")]
    CompleteRequestReceived,
    #[graphql(name = "INBOUND_TRANSFER_CHECKED")]
    InboundTransferChecked,
    #[graphql(name = "TX_BROADCASTED")]
    TxBroadcasted,
    #[graphql(name = "ON_CHAIN_TX_CONFIRMED")]
    OnChainTxConfirmed,
    #[graphql(name = "INBOUND_TRANSFER_CLAIMING_FAILED")]
    InboundTransferClaimingFailed,
    #[graphql(name = "SUCCEEDED")]
    Succeeded,
    #[graphql(name = "EXPIRING_SCHEDULED")]
    ExpiringScheduled,
    #[graphql(name = "EXPIRING_FAILED")]
    ExpiringFailed,
    #[graphql(name = "EXPIRED")]
    Expired,
    #[graphql(name = "FAILING_SCHEDULED")]
    FailingScheduled,
    #[graphql(name = "FAILING_FAILED")]
    FailingFailed,
    #[graphql(name = "FAILED")]
    Failed,
    #[graphql(name = "TX_SIGNED", deprecation = "No more needed")]
    TxSigned,
    #[graphql(name = "WAITING_ON_TX_CONFIRMATIONS", deprecation = "No more needed")]
    WaitingOnTxConfirmations,
    #[graphql(
        name = "INBOUND_TRANSFER_CLAIMING_SCHEDULED",
        deprecation = "No more needed"
    )]
    InboundTransferClaimingScheduled,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq, Debug)]
pub enum SparkLeavesSwapRequestStatus {
    #[graphql(name = "CREATED")]
    Created,
    #[graphql(name = "INBOUND_TRANSFER_VERIFIED")]
    InboundTransferVerified,
    #[graphql(name = "INBOUND_TRANSFER_VERIFYING_FAILED")]
    InboundTransferVerifyingFailed,
    #[graphql(name = "OUTBOUND_TRANSFER_SENT")]
    OutboundTransferSent,
    #[graphql(name = "OUTBOUND_TRANSFER_SENDING_FAILED")]
    OutboundTransferSendingFailed,
    #[graphql(name = "TRANSFERS_COMPLETING_SCHEDULED")]
    TransfersCompletingScheduled,
    #[graphql(name = "OUTBOUND_TRANSFER_COMPLETED")]
    OutboundTransferCompleted,
    #[graphql(name = "OUTBOUND_TRANSFER_COMPLETING_FAILED")]
    OutboundTransferCompletingFailed,
    #[graphql(name = "INBOUND_TRANSFER_CLAIMING_FAILED")]
    InboundTransferClaimingFailed,
    #[graphql(name = "REQUEST_FAILING_FROM_VERIFYING_SCHEDULED")]
    RequestFailingFromVerifyingScheduled,
    #[graphql(name = "REQUEST_FAILING_FROM_SENDING_SCHEDULED")]
    RequestFailingFromSendingScheduled,
    #[graphql(name = "REQUEST_FAILING_FROM_VERIFYING_FAILED")]
    RequestFailingFromVerifyingFailed,
    #[graphql(name = "REQUEST_FAILING_FROM_SENDING_FAILED")]
    RequestFailingFromSendingFailed,
    #[graphql(name = "SUCCEEDED")]
    Succeeded,
    #[graphql(name = "FAILED", deprecation = "No more needed")]
    Failed,
    #[graphql(name = "INITIATED", deprecation = "No more needed")]
    Initiated,
    #[graphql(name = "EXPIRED", deprecation = "No more needed")]
    Expired,
    #[graphql(name = "LEAVES_LOCKED", deprecation = "No more needed")]
    LeavesLocked,
    #[graphql(
        name = "REFUND_TX_ADAPTOR_SIGNED",
        deprecation = "Adaptor is not needed in signing"
    )]
    RefundTxAdaptorSigned,
    #[graphql(name = "INBOUND_TRANSFER_CLAIMED", deprecation = "No more needed")]
    InboundTransferClaimed,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq, Debug)]
pub enum SparkUserRequestStatus {
    #[graphql(name = "CREATED")]
    Created,
    #[graphql(name = "IN_PROGRESS")]
    InProgress,
    #[graphql(name = "SUCCEEDED")]
    Succeeded,
    #[graphql(name = "FAILED")]
    Failed,
    #[graphql(name = "CANCELED")]
    Canceled,
    #[graphql(name = "UNKNOWN")]
    Unknown,
}

#[allow(clippy::enum_variant_names)]
#[derive(Enum, Copy, Clone, Eq, PartialEq, Debug)]
pub enum SparkWalletWebhookEventType {
    #[graphql(name = "SPARK_LIGHTNING_RECEIVE_FINISHED")]
    SparkLightningReceiveFinished,
    #[graphql(name = "SPARK_LIGHTNING_SEND_FINISHED")]
    SparkLightningSendFinished,
    #[graphql(name = "SPARK_COOP_EXIT_FINISHED")]
    SparkCoopExitFinished,
    #[graphql(name = "SPARK_STATIC_DEPOSIT_FINISHED")]
    SparkStaticDepositFinished,
}

#[derive(SimpleObject, Debug, Clone)]
#[graphql(rename_fields = "snake_case")]
pub struct CurrencyAmount {
    pub original_value: Long,
    pub original_unit: CurrencyUnit,
    pub preferred_currency_unit: CurrencyUnit,
    pub preferred_currency_value_rounded: Long,
    pub preferred_currency_value_approx: f64,
}

#[derive(SimpleObject, Debug, Clone)]
#[graphql(rename_fields = "snake_case")]
pub struct Invoice {
    pub encoded_invoice: String,
    pub bitcoin_network: BitcoinNetwork,
    pub payment_hash: Hash32,
    pub amount: CurrencyAmount,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub memo: Option<String>,
}

#[derive(SimpleObject, Debug, Clone)]
#[graphql(rename_fields = "snake_case")]
pub struct Leaf {
    pub amount: CurrencyAmount,
    pub spark_node_id: Uuid,
}

#[derive(SimpleObject, Debug, Clone)]
#[graphql(rename_fields = "snake_case")]
pub struct SwapLeaf {
    pub leaf_id: Uuid,
    pub raw_unsigned_refund_transaction: String,
    pub adaptor_signed_signature: String,
    pub direct_raw_unsigned_refund_transaction: Option<String>,
    pub direct_from_cpfp_raw_unsigned_refund_transaction: Option<String>,
    pub direct_adaptor_signed_signature: Option<String>,
    pub direct_from_cpfp_adaptor_signed_signature: Option<String>,
}

#[derive(SimpleObject, Debug, Clone)]
#[graphql(rename_fields = "snake_case")]
pub struct PageInfo {
    pub has_next_page: Option<bool>,
    pub has_previous_page: Option<bool>,
    pub start_cursor: Option<String>,
    pub end_cursor: Option<String>,
}

#[derive(SimpleObject, Debug, Clone)]
#[graphql(rename_fields = "snake_case")]
pub struct SparkTransferToLeavesConnection {
    pub count: i32,
    pub page_info: PageInfo,
    pub entities: Vec<Leaf>,
}

#[derive(SimpleObject, Debug, Clone)]
#[graphql(rename_fields = "snake_case")]
pub struct Transfer {
    pub total_amount: CurrencyAmount,
    pub spark_id: Option<Uuid>,
    pub leaves: SparkTransferToLeavesConnection,
    pub user_request: Option<Box<UserRequest>>,
}

#[derive(SimpleObject, Debug, Clone)]
#[graphql(name = "ClaimStaticDeposit", rename_fields = "snake_case")]
pub struct ClaimStaticDepositType {
    pub id: ID,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub network: BitcoinNetwork,
    pub request_status: Option<SparkUserRequestStatus>,
    pub deposit_amount: CurrencyAmount,
    pub credit_amount: CurrencyAmount,
    pub max_fee: CurrencyAmount,
    pub status: ClaimStaticDepositStatus,
    pub transaction_id: String,
    pub output_index: i32,
    pub bitcoin_network: BitcoinNetwork,
    pub transfer_spark_id: Option<Uuid>,
    pub static_deposit_address: Option<String>,
}

#[derive(SimpleObject, Debug, Clone)]
#[graphql(rename_fields = "snake_case")]
pub struct CoopExitFeeQuote {
    pub id: ID,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub network: BitcoinNetwork,
    pub total_amount: CurrencyAmount,
    pub user_fee_fast: CurrencyAmount,
    pub user_fee_medium: CurrencyAmount,
    pub user_fee_slow: CurrencyAmount,
    #[graphql(name = "l1_broadcast_fee_fast")]
    pub l1_broadcast_fee_fast: CurrencyAmount,
    #[graphql(name = "l1_broadcast_fee_medium")]
    pub l1_broadcast_fee_medium: CurrencyAmount,
    #[graphql(name = "l1_broadcast_fee_slow")]
    pub l1_broadcast_fee_slow: CurrencyAmount,
    pub expires_at: DateTime<Utc>,
}

#[derive(SimpleObject, Debug, Clone)]
#[graphql(name = "CoopExitRequest", rename_fields = "snake_case")]
pub struct CoopExitRequestType {
    pub id: ID,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub network: BitcoinNetwork,
    pub request_status: Option<SparkUserRequestStatus>,
    pub fee: CurrencyAmount,
    pub withdrawal_address: Option<String>,
    #[graphql(name = "l1_broadcast_fee")]
    pub l1_broadcast_fee: CurrencyAmount,
    pub fee_quote: Option<CoopExitFeeQuote>,
    pub exit_speed: Option<ExitSpeed>,
    pub status: SparkCoopExitRequestStatus,
    pub expires_at: DateTime<Utc>,
    pub raw_connector_transaction: String,
    pub raw_coop_exit_transaction: String,
    pub coop_exit_txid: String,
    pub transfer_spark_id: Option<Uuid>,
    pub transfer: Option<Transfer>,
}

#[derive(SimpleObject, Debug, Clone)]
#[graphql(name = "LeavesSwapRequest", rename_fields = "snake_case")]
pub struct LeavesSwapRequestType {
    pub id: ID,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub network: BitcoinNetwork,
    pub request_status: Option<SparkUserRequestStatus>,
    pub status: SparkLeavesSwapRequestStatus,
    pub total_amount: CurrencyAmount,
    pub target_amount: CurrencyAmount,
    pub fee: CurrencyAmount,
    pub inbound_transfer: Option<Transfer>,
    pub outbound_transfer: Option<Transfer>,
    pub expires_at: Option<DateTime<Utc>>,
    pub swap_leaves: Option<Vec<SwapLeaf>>,
}

#[derive(SimpleObject, Debug, Clone)]
#[graphql(name = "LightningReceiveRequest", rename_fields = "snake_case")]
pub struct LightningReceiveRequestType {
    pub id: ID,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub network: BitcoinNetwork,
    pub request_status: Option<SparkUserRequestStatus>,
    pub invoice: Invoice,
    pub status: LightningReceiveRequestStatus,
    pub transfer: Option<Transfer>,
    pub payment_preimage: Option<Hash32>,
    pub receiver_identity_public_key: Option<PublicKey>,
}

#[derive(SimpleObject, Debug, Clone)]
#[graphql(name = "LightningSendRequest", rename_fields = "snake_case")]
pub struct LightningSendRequestType {
    pub id: ID,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub network: BitcoinNetwork,
    pub request_status: Option<SparkUserRequestStatus>,
    pub encoded_invoice: String,
    pub fee: CurrencyAmount,
    pub idempotency_key: String,
    pub status: LightningSendRequestStatus,
    pub transfer: Option<Transfer>,
    pub payment_preimage: Option<String>,
}

#[derive(Interface, Debug, Clone)]
// `created_at` and `updated_at` share a `ty`, which `duplicated_attributes` flags.
#[allow(clippy::duplicated_attributes)]
#[graphql(
    rename_fields = "snake_case",
    field(name = "id", ty = "&ID"),
    field(name = "created_at", ty = "&DateTime<Utc>"),
    field(name = "updated_at", ty = "&DateTime<Utc>"),
    field(name = "network", ty = "&BitcoinNetwork"),
    field(name = "request_status", ty = "&Option<SparkUserRequestStatus>")
)]
pub enum UserRequest {
    ClaimStaticDeposit(ClaimStaticDepositType),
    CoopExitRequest(CoopExitRequestType),
    LeavesSwapRequest(LeavesSwapRequestType),
    LightningReceiveRequest(LightningReceiveRequestType),
    LightningSendRequest(LightningSendRequestType),
}

#[derive(SimpleObject, Debug, Clone)]
#[graphql(rename_fields = "snake_case")]
pub struct ClaimStaticDepositOutput {
    pub transfer_id: Uuid,
}

#[derive(SimpleObject, Debug, Clone)]
#[graphql(rename_fields = "snake_case")]
pub struct CompleteCoopExitOutput {
    pub request: CoopExitRequestType,
}

#[derive(SimpleObject, Debug, Clone)]
#[graphql(rename_fields = "snake_case")]
pub struct InstantStaticDepositQuote {
    pub id: ID,
    pub transaction_id: String,
    pub output_index: i32,
    pub deposit_amount: CurrencyAmount,
    pub credit_amount: CurrencyAmount,
    pub quote_signature: String,
}

#[derive(SimpleObject, Debug, Clone)]
#[graphql(rename_fields = "snake_case")]
pub struct StaticDepositPlan {
    pub id: ID,
    pub amount: CurrencyAmount,
    pub confirmations: i32,
}

#[derive(SimpleObject, Debug, Clone)]
#[graphql(rename_fields = "snake_case")]
pub struct CreateInstantStaticDepositQuoteOutput {
    pub quote: InstantStaticDepositQuote,
    pub fulfillment_plans: Vec<StaticDepositPlan>,
}

#[derive(SimpleObject, Debug, Clone)]
#[graphql(rename_fields = "snake_case")]
pub struct CreateClaimInstantStaticDepositOutput {
    pub claim_id: Uuid,
}

#[derive(SimpleObject, Debug, Clone)]
#[graphql(rename_fields = "snake_case")]
pub struct GetChallengeOutput {
    pub protected_challenge: String,
}

#[derive(SimpleObject, Debug, Clone)]
#[graphql(rename_fields = "snake_case")]
pub struct VerifyChallengeOutput {
    pub valid_until: DateTime<Utc>,
    pub session_token: String,
}

#[derive(SimpleObject, Debug, Clone)]
#[graphql(rename_fields = "snake_case")]
pub struct CoopExitFeeQuoteOutput {
    pub quote: CoopExitFeeQuote,
}

#[derive(SimpleObject, Debug, Clone)]
#[graphql(rename_fields = "snake_case")]
pub struct LeavesSwapFeeEstimateOutput {
    pub fee_estimate: CurrencyAmount,
}

#[derive(SimpleObject, Debug, Clone)]
#[graphql(rename_fields = "snake_case")]
pub struct LightningSendFeeEstimateOutput {
    pub fee_estimate: CurrencyAmount,
}

#[derive(SimpleObject, Debug, Clone)]
#[graphql(rename_fields = "snake_case")]
pub struct StaticDepositQuoteOutput {
    pub transaction_id: String,
    pub output_index: i32,
    pub network: BitcoinNetwork,
    pub credit_amount_sats: Long,
    pub signature: String,
}

#[derive(SimpleObject, Debug, Clone)]
#[graphql(rename_fields = "snake_case")]
pub struct RequestCoopExitOutput {
    pub request: CoopExitRequestType,
}

#[derive(SimpleObject, Debug, Clone)]
#[graphql(rename_fields = "snake_case")]
pub struct RequestLightningReceiveOutput {
    pub request: LightningReceiveRequestType,
}

#[derive(SimpleObject, Debug, Clone)]
#[graphql(rename_fields = "snake_case")]
pub struct RequestLightningSendOutput {
    pub request: LightningSendRequestType,
}

#[derive(SimpleObject, Debug, Clone)]
#[graphql(rename_fields = "snake_case")]
pub struct RequestSwapOutput {
    pub request: LeavesSwapRequestType,
}

#[derive(SimpleObject, Debug, Clone)]
#[graphql(rename_fields = "snake_case")]
pub struct RequestRegtestFundsOutput {
    pub transaction_hash: String,
}

#[derive(SimpleObject, Debug, Clone)]
#[graphql(rename_fields = "snake_case")]
pub struct RegisterSparkWalletWebhookOutput {
    pub webhook_id: ID,
}

#[derive(SimpleObject, Debug, Clone)]
#[graphql(rename_fields = "snake_case")]
pub struct DeleteSparkWalletWebhookOutput {
    pub success: bool,
}

#[derive(SimpleObject, Debug, Clone)]
#[graphql(rename_fields = "snake_case")]
pub struct SparkWalletWebhookEntry {
    pub webhook_id: ID,
    pub url: String,
    pub event_types: Vec<SparkWalletWebhookEventType>,
}

#[derive(SimpleObject, Debug, Clone)]
#[graphql(rename_fields = "snake_case")]
pub struct ListSparkWalletWebhooksOutput {
    pub webhooks: Vec<SparkWalletWebhookEntry>,
}

#[derive(InputObject, Debug)]
#[graphql(rename_fields = "snake_case")]
pub struct ClaimStaticDepositInput {
    pub transaction_id: String,
    pub output_index: i32,
    pub network: BitcoinNetwork,
    pub request_type: ClaimStaticDepositRequestType,
    pub credit_amount_sats: Option<Long>,
    pub max_fee_sats: Option<Long>,
    pub deposit_secret_key: Option<String>,
    pub encrypted_deposit_secret_key: Option<String>,
    pub signature: String,
    pub quote_signature: String,
}

#[derive(InputObject, Debug)]
#[graphql(rename_fields = "snake_case")]
pub struct CompleteCoopExitInput {
    pub user_outbound_transfer_external_id: Uuid,
    pub coop_exit_request_id: Option<ID>,
}

#[derive(InputObject, Debug)]
#[graphql(rename_fields = "snake_case")]
pub struct CoopExitFeeQuoteInput {
    #[graphql(validator(max_items = 1000))]
    pub leaf_external_ids: Vec<Uuid>,
    pub withdrawal_address: String,
}

#[derive(InputObject, Debug)]
#[graphql(rename_fields = "snake_case")]
pub struct CreateInstantStaticDepositQuoteInput {
    pub transaction_id: String,
    pub output_index: i32,
    pub network: BitcoinNetwork,
}

#[derive(InputObject, Debug)]
#[graphql(rename_fields = "snake_case")]
pub struct CreateClaimInstantStaticDepositInput {
    pub static_deposit_quote_id: ID,
    pub static_deposit_address_private_key_share: Option<String>,
    pub encrypted_static_deposit_address_private_key_share: Option<String>,
    pub signature: String,
}

#[derive(InputObject, Debug)]
#[graphql(rename_fields = "snake_case")]
pub struct GetChallengeInput {
    pub public_key: PublicKey,
}

#[derive(InputObject, Debug)]
#[graphql(rename_fields = "snake_case")]
pub struct LeavesSwapFeeEstimateInput {
    pub total_amount_sats: i32,
}

#[derive(InputObject, Debug)]
#[graphql(rename_fields = "snake_case")]
pub struct LightningSendFeeEstimateInput {
    pub encoded_invoice: String,
    pub amount_sats: Option<Long>,
}

#[derive(InputObject, Debug)]
#[graphql(rename_fields = "snake_case")]
pub struct Provider {
    pub account_id: ID,
    pub jwt: String,
}

#[derive(InputObject, Debug)]
#[graphql(rename_fields = "snake_case")]
pub struct RequestCoopExitInput {
    #[graphql(validator(max_items = 1000))]
    pub leaf_external_ids: Vec<Uuid>,
    pub withdrawal_address: String,
    pub exit_speed: ExitSpeed,
    #[graphql(default = true)]
    pub withdraw_all: bool,
    #[graphql(validator(max_items = 1000))]
    pub fee_leaf_external_ids: Option<Vec<Uuid>>,
    pub fee_quote_id: Option<ID>,
    pub idempotency_key: Option<String>,
    pub user_outbound_transfer_external_id: Option<Uuid>,
}

#[derive(InputObject, Debug)]
#[graphql(rename_fields = "snake_case")]
pub struct UserLeafInput {
    pub leaf_id: Uuid,
    pub raw_unsigned_refund_transaction: String,
    pub adaptor_added_signature: String,
    pub direct_raw_unsigned_refund_transaction: Option<String>,
    pub direct_from_cpfp_raw_unsigned_refund_transaction: Option<String>,
    pub direct_adaptor_added_signature: Option<String>,
    pub direct_from_cpfp_adaptor_added_signature: Option<String>,
}

#[derive(InputObject, Debug)]
#[graphql(rename_fields = "snake_case")]
pub struct RequestLightningReceiveInput {
    pub network: BitcoinNetwork,
    pub amount_sats: Long,
    pub payment_hash: Hash32,
    pub expiry_secs: Option<i32>,
    pub memo: Option<String>,
    pub receiver_identity_pubkey: Option<PublicKey>,
    #[graphql(default = false)]
    pub include_spark_address: bool,
    pub description_hash: Option<Hash32>,
    pub spark_invoice: Option<String>,
}

#[derive(InputObject, Debug)]
#[graphql(rename_fields = "snake_case")]
pub struct RequestLightningSendInput {
    pub encoded_invoice: String,
    pub amount_sats: Option<Long>,
    pub idempotency_key: Option<String>,
    pub user_outbound_transfer_external_id: Option<Uuid>,
}

#[derive(InputObject, Debug)]
#[graphql(rename_fields = "snake_case")]
pub struct RequestSwapInput {
    pub adaptor_pubkey: PublicKey,
    pub total_amount_sats: Long,
    #[graphql(validator(max_items = 1000))]
    pub target_amount_sats: Vec<Long>,
    pub fee_sats: Long,
    #[graphql(validator(max_items = 1000))]
    pub user_leaves: Vec<UserLeafInput>,
    pub user_outbound_transfer_external_id: Uuid,
}

#[derive(InputObject, Debug)]
#[graphql(rename_fields = "snake_case")]
pub struct RequestRegtestFundsInput {
    pub amount_sats: Long,
    pub address: String,
}

#[derive(InputObject, Debug)]
#[graphql(rename_fields = "snake_case")]
pub struct StaticDepositQuoteInput {
    pub transaction_id: String,
    pub output_index: i32,
    pub network: BitcoinNetwork,
}

#[derive(InputObject, Debug)]
#[graphql(rename_fields = "snake_case")]
pub struct VerifyChallengeInput {
    pub protected_challenge: String,
    pub signature: String,
    pub identity_public_key: PublicKey,
    pub provider: Option<Provider>,
}

#[derive(InputObject, Debug)]
#[graphql(rename_fields = "snake_case")]
pub struct RegisterSparkWalletWebhookInput {
    pub url: String,
    pub secret: String,
    pub event_types: Vec<SparkWalletWebhookEventType>,
}

#[derive(InputObject, Debug)]
#[graphql(rename_fields = "snake_case")]
pub struct DeleteSparkWalletWebhookInput {
    pub webhook_id: ID,
}
