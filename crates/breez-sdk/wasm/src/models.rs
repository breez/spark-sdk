use std::collections::HashMap;

use wasm_bindgen::prelude::wasm_bindgen;

#[allow(clippy::large_enum_variant)]
#[macros::extern_wasm_bindgen(breez_sdk_spark::SdkEvent)]
pub enum SdkEvent {
    Synced,
    ClaimDepositsFailed {
        unclaimed_deposits: Vec<DepositInfo>,
    },
    ClaimDepositsSucceeded {
        claimed_deposits: Vec<DepositInfo>,
    },
    PaymentSucceeded {
        payment: Payment,
    },
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::ConnectRequest)]
pub struct ConnectRequest {
    pub config: Config,
    pub mnemonic: String,
    pub storage_dir: String,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::DepositInfo)]
pub struct DepositInfo {
    pub txid: String,
    pub vout: u32,
    pub amount_sats: u64,
    pub refund_tx: Option<String>,
    pub refund_tx_id: Option<String>,
    pub claim_error: Option<DepositClaimError>,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::ClaimDepositRequest)]
pub struct ClaimDepositRequest {
    pub txid: String,
    pub vout: u32,
    pub max_fee: Option<Fee>,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::ClaimDepositResponse)]
pub struct ClaimDepositResponse {
    pub payment: Payment,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::RefundDepositRequest)]
pub struct RefundDepositRequest {
    pub txid: String,
    pub vout: u32,
    pub destination_address: String,
    pub fee: Fee,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::RefundDepositResponse)]
pub struct RefundDepositResponse {
    pub tx_id: String,
    pub tx_hex: String,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::ListUnclaimedDepositsRequest)]
pub struct ListUnclaimedDepositsRequest {}

#[macros::extern_wasm_bindgen(breez_sdk_spark::ListUnclaimedDepositsResponse)]
pub struct ListUnclaimedDepositsResponse {
    pub deposits: Vec<DepositInfo>,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::DepositClaimError)]
pub enum DepositClaimError {
    DepositClaimFeeExceeded {
        tx: String,
        vout: u32,
        max_fee: Fee,
        actual_fee: u64,
    },
    MissingUtxo {
        tx: String,
        vout: u32,
    },
    Generic {
        message: String,
    },
}

#[macros::extern_wasm_bindgen(breez_sdk_common::input::InputType)]
pub enum InputType {
    BitcoinAddress(BitcoinAddressDetails),
    Bolt11Invoice(Bolt11InvoiceDetails),
    Bolt12Invoice(Bolt12InvoiceDetails),
    Bolt12Offer(Bolt12OfferDetails),
    LightningAddress(LightningAddressDetails),
    LnurlPay(LnurlPayRequestDetails),
    SilentPaymentAddress(SilentPaymentAddressDetails),
    LnurlAuth(LnurlAuthRequestDetails),
    Url(String),
    Bip21(Bip21Details),
    Bolt12InvoiceRequest(Bolt12InvoiceRequestDetails),
    LnurlWithdraw(LnurlWithdrawRequestDetails),
    SparkAddress(SparkAddressDetails),
}

#[macros::extern_wasm_bindgen(breez_sdk_common::input::SparkAddressDetails)]
pub struct SparkAddressDetails {
    pub address: String,
    pub decoded_address: SparkAddress,
    pub source: PaymentRequestSource,
}

#[macros::extern_wasm_bindgen(breez_sdk_common::input::SparkAddress)]
pub struct SparkAddress {
    pub identity_public_key: String,
    pub network: BitcoinNetwork,
    pub spark_invoice_fields: Option<SparkInvoiceFields>,
    pub signature: Option<String>,
}

#[macros::extern_wasm_bindgen(breez_sdk_common::input::SparkInvoiceFields)]
pub struct SparkInvoiceFields {
    pub id: String,
    pub version: u32,
    pub memo: Option<String>,
    pub sender_public_key: Option<String>,
    pub expiry_time: Option<u64>,
    pub payment_type: Option<SparkAddressPaymentType>,
}

#[macros::extern_wasm_bindgen(breez_sdk_common::input::SparkAddressPaymentType)]
pub enum SparkAddressPaymentType {
    TokensPayment(TokensPaymentDetails),
    SatsPayment(SatsPaymentDetails),
}

#[macros::extern_wasm_bindgen(breez_sdk_common::input::TokensPaymentDetails)]
pub struct TokensPaymentDetails {
    pub token_identifier: Option<String>,
    pub amount: Option<u64>,
}

#[macros::extern_wasm_bindgen(breez_sdk_common::input::SatsPaymentDetails)]
pub struct SatsPaymentDetails {
    pub amount: Option<u64>,
}

#[macros::extern_wasm_bindgen(breez_sdk_common::input::BitcoinAddressDetails)]
pub struct BitcoinAddressDetails {
    pub address: String,
    pub network: BitcoinNetwork,
    pub source: PaymentRequestSource,
}

#[macros::extern_wasm_bindgen(breez_sdk_common::network::BitcoinNetwork)]
pub enum BitcoinNetwork {
    Bitcoin,
    Testnet3,
    Testnet4,
    Signet,
    Regtest,
}

#[macros::extern_wasm_bindgen(breez_sdk_common::input::PaymentRequestSource)]
pub struct PaymentRequestSource {
    pub bip_21_uri: Option<String>,
    pub bip_353_address: Option<String>,
}

#[macros::extern_wasm_bindgen(breez_sdk_common::input::Bolt11InvoiceDetails)]
pub struct Bolt11InvoiceDetails {
    pub amount_msat: Option<u64>,
    pub description: Option<String>,
    pub description_hash: Option<String>,
    pub expiry: u64,
    pub invoice: Bolt11Invoice,
    pub min_final_cltv_expiry_delta: u64,
    pub network: BitcoinNetwork,
    pub payee_pubkey: String,
    pub payment_hash: String,
    pub payment_secret: String,
    pub routing_hints: Vec<Bolt11RouteHint>,
    pub timestamp: u64,
}

#[macros::extern_wasm_bindgen(breez_sdk_common::input::Bolt11Invoice)]
pub struct Bolt11Invoice {
    pub bolt11: String,
    pub source: PaymentRequestSource,
}

#[macros::extern_wasm_bindgen(breez_sdk_common::input::Bolt11RouteHint)]
pub struct Bolt11RouteHint {
    pub hops: Vec<Bolt11RouteHintHop>,
}

#[macros::extern_wasm_bindgen(breez_sdk_common::input::Bolt11RouteHintHop)]
pub struct Bolt11RouteHintHop {
    pub src_node_id: String,
    pub short_channel_id: String,
    pub fees_base_msat: u32,
    pub fees_proportional_millionths: u32,
    pub cltv_expiry_delta: u16,
    pub htlc_minimum_msat: Option<u64>,
    pub htlc_maximum_msat: Option<u64>,
}

#[macros::extern_wasm_bindgen(breez_sdk_common::input::Bolt12InvoiceDetails)]
pub struct Bolt12InvoiceDetails {
    pub amount_msat: u64,
    pub invoice: Bolt12Invoice,
}

#[macros::extern_wasm_bindgen(breez_sdk_common::input::Bolt12Invoice)]
pub struct Bolt12Invoice {
    pub invoice: String,
    pub source: PaymentRequestSource,
}

#[macros::extern_wasm_bindgen(breez_sdk_common::input::Bolt12Offer)]
pub struct Bolt12Offer {
    pub offer: String,
    pub source: PaymentRequestSource,
}

#[macros::extern_wasm_bindgen(breez_sdk_common::input::Bolt12OfferDetails)]
pub struct Bolt12OfferDetails {
    pub absolute_expiry: Option<u64>,
    pub chains: Vec<String>,
    pub description: Option<String>,
    pub issuer: Option<String>,
    pub min_amount: Option<Amount>,
    pub offer: Bolt12Offer,
    pub paths: Vec<Bolt12OfferBlindedPath>,
    pub signing_pubkey: Option<String>,
}

#[macros::extern_wasm_bindgen(breez_sdk_common::input::Bolt12OfferBlindedPath)]
pub struct Bolt12OfferBlindedPath {
    pub blinded_hops: Vec<String>,
}

#[macros::extern_wasm_bindgen(breez_sdk_common::input::Amount)]
pub enum Amount {
    Bitcoin {
        amount_msat: u64,
    },
    Currency {
        iso4217_code: String,
        fractional_amount: u64,
    },
}

#[macros::extern_wasm_bindgen(breez_sdk_common::input::LightningAddressDetails)]
pub struct LightningAddressDetails {
    pub address: String,
    pub pay_request: LnurlPayRequestDetails,
}

#[macros::extern_wasm_bindgen(breez_sdk_common::lnurl::pay::LnurlPayRequestDetails)]
pub struct LnurlPayRequestDetails {
    pub callback: String,
    pub min_sendable: u64,
    pub max_sendable: u64,
    pub metadata_str: String,
    pub comment_allowed: u16,
    pub domain: String,
    pub url: String,
    pub address: Option<String>,
    pub allows_nostr: bool,
    pub nostr_pubkey: Option<String>,
}

#[macros::extern_wasm_bindgen(breez_sdk_common::input::SilentPaymentAddressDetails)]

pub struct SilentPaymentAddressDetails {
    pub address: String,
    pub network: BitcoinNetwork,
    pub source: PaymentRequestSource,
}

#[macros::extern_wasm_bindgen(breez_sdk_common::lnurl::auth::LnurlAuthRequestDetails)]
pub struct LnurlAuthRequestDetails {
    pub k1: String,
    pub action: Option<String>,
    pub domain: String,
    pub url: String,
}

#[macros::extern_wasm_bindgen(breez_sdk_common::input::Bip21Details)]
pub struct Bip21Details {
    pub amount_sat: Option<u64>,
    pub asset_id: Option<String>,
    pub uri: String,
    pub extras: Vec<Bip21Extra>,
    pub label: Option<String>,
    pub message: Option<String>,
    pub payment_methods: Vec<InputType>,
}

#[macros::extern_wasm_bindgen(breez_sdk_common::input::Bip21Extra)]
pub struct Bip21Extra {
    pub key: String,
    pub value: String,
}

#[macros::extern_wasm_bindgen(breez_sdk_common::input::Bolt12InvoiceRequestDetails)]
pub struct Bolt12InvoiceRequestDetails {
    // TODO: Fill fields
}

#[macros::extern_wasm_bindgen(breez_sdk_common::input::LnurlWithdrawRequestDetails)]

pub struct LnurlWithdrawRequestDetails {
    pub callback: String,
    pub k1: String,
    pub default_description: String,
    pub min_withdrawable: u64,
    pub max_withdrawable: u64,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::PaymentType)]
pub enum PaymentType {
    Send,
    Receive,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::PaymentStatus)]
pub enum PaymentStatus {
    Completed,
    Pending,
    Failed,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::Payment)]
pub struct Payment {
    pub id: String,
    pub payment_type: PaymentType,
    pub status: PaymentStatus,
    pub amount: u64,
    pub fees: u64,
    pub timestamp: u64,
    pub method: PaymentMethod,
    pub details: Option<PaymentDetails>,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::PaymentDetails)]
pub enum PaymentDetails {
    Spark,
    Token {
        metadata: TokenMetadata,
    },
    Lightning {
        description: Option<String>,
        preimage: Option<String>,
        invoice: String,
        payment_hash: String,
        destination_pubkey: String,
        lnurl_pay_info: Option<LnurlPayInfo>,
    },
    Withdraw {
        tx_id: String,
    },
    Deposit {
        tx_id: String,
    },
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::PaymentMethod)]
pub enum PaymentMethod {
    Lightning,
    Spark,
    Token,
    Deposit,
    Withdraw,
    Unknown,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::LnurlPayInfo)]
pub struct LnurlPayInfo {
    pub ln_address: Option<String>,
    pub comment: Option<String>,
    pub domain: Option<String>,
    pub metadata: Option<String>,
    pub processed_success_action: Option<SuccessActionProcessed>,
    pub raw_success_action: Option<SuccessAction>,
}

#[macros::extern_wasm_bindgen(breez_sdk_common::lnurl::pay::SuccessActionProcessed)]
pub enum SuccessActionProcessed {
    Aes { result: AesSuccessActionDataResult },
    Message { data: MessageSuccessActionData },
    Url { data: UrlSuccessActionData },
}

#[macros::extern_wasm_bindgen(breez_sdk_common::lnurl::pay::AesSuccessActionDataResult)]
pub enum AesSuccessActionDataResult {
    Decrypted { data: AesSuccessActionDataDecrypted },
    ErrorStatus { reason: String },
}

#[macros::extern_wasm_bindgen(breez_sdk_common::lnurl::pay::AesSuccessActionDataDecrypted)]
pub struct AesSuccessActionDataDecrypted {
    pub description: String,
    pub plaintext: String,
}

#[macros::extern_wasm_bindgen(breez_sdk_common::lnurl::pay::MessageSuccessActionData)]
pub struct MessageSuccessActionData {
    pub message: String,
}

#[macros::extern_wasm_bindgen(breez_sdk_common::lnurl::pay::UrlSuccessActionData)]
pub struct UrlSuccessActionData {
    pub description: String,
    pub url: String,
    pub matches_callback_domain: bool,
}

#[macros::extern_wasm_bindgen(breez_sdk_common::lnurl::pay::SuccessAction)]
pub enum SuccessAction {
    Aes { data: AesSuccessActionData },
    Message { data: MessageSuccessActionData },
    Url { data: UrlSuccessActionData },
}

#[macros::extern_wasm_bindgen(breez_sdk_common::lnurl::pay::AesSuccessActionData)]
pub struct AesSuccessActionData {
    pub description: String,
    pub ciphertext: String,
    pub iv: String,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::Network)]
pub enum Network {
    Mainnet,
    Regtest,
}

impl std::fmt::Display for Network {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Network::Mainnet => write!(f, "Mainnet"),
            Network::Regtest => write!(f, "Regtest"),
        }
    }
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::Config)]
pub struct Config {
    pub api_key: Option<String>,
    pub network: Network,
    pub sync_interval_secs: u32,
    pub max_deposit_claim_fee: Option<Fee>,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::Fee)]
pub enum Fee {
    Fixed { amount: u64 },
    Rate { sat_per_vbyte: u64 },
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::Credentials)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::GetInfoRequest)]
pub struct GetInfoRequest {}

#[macros::extern_wasm_bindgen(breez_sdk_spark::GetInfoResponse)]
pub struct GetInfoResponse {
    pub balance_sats: u64,
    pub token_balances: HashMap<String, TokenBalance>,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::TokenBalance)]
pub struct TokenBalance {
    pub balance: u64,
    pub token_metadata: TokenMetadata,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::TokenMetadata)]
pub struct TokenMetadata {
    pub identifier: String,
    /// Hex representation of the issuer public key
    pub issuer_public_key: String,
    pub name: String,
    pub ticker: String,
    /// Number of decimals the token uses
    pub decimals: u32,
    /// Decimal representation of the token max supply (unsigned 128-bit integer)
    pub max_supply: u64,
    pub is_freezable: bool,
    pub creation_entity_public_key: Option<String>,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::SyncWalletRequest)]
pub struct SyncWalletRequest {}

#[macros::extern_wasm_bindgen(breez_sdk_spark::SyncWalletResponse)]
pub struct SyncWalletResponse {}

#[macros::extern_wasm_bindgen(breez_sdk_spark::ReceivePaymentMethod)]
pub enum ReceivePaymentMethod {
    SparkAddress,
    BitcoinAddress,
    Bolt11Invoice {
        description: String,
        amount_sats: Option<u64>,
    },
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::SendOnchainFeeQuote)]
pub struct SendOnchainFeeQuote {
    pub id: String,
    pub expires_at: u64,
    pub speed_fast: SendOnchainSpeedFeeQuote,
    pub speed_medium: SendOnchainSpeedFeeQuote,
    pub speed_slow: SendOnchainSpeedFeeQuote,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::SendOnchainSpeedFeeQuote)]
pub struct SendOnchainSpeedFeeQuote {
    pub user_fee_sat: u64,
    pub l1_broadcast_fee_sat: u64,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::SendPaymentMethod)]
pub enum SendPaymentMethod {
    BitcoinAddress {
        address: BitcoinAddressDetails,
        fee_quote: SendOnchainFeeQuote,
    },
    Bolt11Invoice {
        invoice_details: Bolt11InvoiceDetails,
        spark_transfer_fee_sats: Option<u64>,
        lightning_fee_sats: u64,
    }, // should be replaced with the parsed invoice
    SparkAddress {
        address: String,
        fee: u64,
        token_identifier: Option<String>,
    },
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::ReceivePaymentRequest)]
pub struct ReceivePaymentRequest {
    pub payment_method: ReceivePaymentMethod,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::ReceivePaymentResponse)]
pub struct ReceivePaymentResponse {
    pub payment_request: String,
    pub fee_sats: u64,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::PrepareLnurlPayRequest)]
pub struct PrepareLnurlPayRequest {
    pub amount_sats: u64,
    pub comment: Option<String>,
    pub pay_request: LnurlPayRequestDetails,
    pub validate_success_action_url: Option<bool>,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::PrepareLnurlPayResponse)]
pub struct PrepareLnurlPayResponse {
    pub amount_sats: u64,
    pub comment: Option<String>,
    pub pay_request: LnurlPayRequestDetails,
    pub fee_sats: u64,
    pub invoice_details: Bolt11InvoiceDetails,
    pub success_action: Option<SuccessAction>,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::LnurlPayRequest)]
pub struct LnurlPayRequest {
    pub prepare_response: PrepareLnurlPayResponse,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::LnurlPayResponse)]
pub struct LnurlPayResponse {
    pub payment: Payment,
    pub success_action: Option<SuccessActionProcessed>,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::PrepareSendPaymentRequest)]
pub struct PrepareSendPaymentRequest {
    pub payment_request: String,
    pub amount: Option<u64>,
    pub token_identifier: Option<String>,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::PrepareSendPaymentResponse)]
pub struct PrepareSendPaymentResponse {
    pub payment_method: SendPaymentMethod,
    pub amount: u64,
    pub token_identifier: Option<String>,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::OnchainConfirmationSpeed)]
pub enum OnchainConfirmationSpeed {
    Fast,
    Medium,
    Slow,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::SendPaymentOptions)]
pub enum SendPaymentOptions {
    BitcoinAddress {
        confirmation_speed: OnchainConfirmationSpeed,
    },
    Bolt11Invoice {
        use_spark: bool,
    },
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::SendPaymentRequest)]
pub struct SendPaymentRequest {
    pub prepare_response: PrepareSendPaymentResponse,
    pub options: Option<SendPaymentOptions>,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::SendPaymentResponse)]
pub struct SendPaymentResponse {
    pub payment: Payment,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::ListPaymentsRequest)]
pub struct ListPaymentsRequest {
    pub offset: Option<u32>,
    pub limit: Option<u32>,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::ListPaymentsResponse)]
pub struct ListPaymentsResponse {
    pub payments: Vec<Payment>,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::GetPaymentRequest)]
pub struct GetPaymentRequest {
    pub payment_id: String,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::GetPaymentResponse)]
pub struct GetPaymentResponse {
    pub payment: Payment,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::LogEntry)]
pub struct LogEntry {
    pub line: String,
    pub level: String,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::PaymentMetadata)]
pub struct PaymentMetadata {
    pub lnurl_pay_info: Option<LnurlPayInfo>,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::UpdateDepositPayload)]
pub enum UpdateDepositPayload {
    ClaimError {
        error: DepositClaimError,
    },
    Refund {
        refund_txid: String,
        refund_tx: String,
    },
}
