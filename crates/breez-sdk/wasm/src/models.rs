#[macros::extern_wasm_bindgen(breez_sdk_core::SdkEvent)]
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

#[macros::extern_wasm_bindgen(breez_sdk_core::DepositInfo)]
pub struct DepositInfo {
    pub txid: String,
    pub vout: u32,
    pub amount_sats: Option<u64>,
    pub error: Option<DepositClaimError>,
}

#[macros::extern_wasm_bindgen(breez_sdk_core::DepositClaimError)]
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
    BitcoinAddress(BitcoinAddress),
    Bolt11Invoice(DetailedBolt11Invoice),
    Bolt12Invoice(DetailedBolt12Invoice),
    Bolt12Offer(DetailedBolt12Offer),
    LightningAddress(LightningAddress),
    LnurlPay(LnurlPayRequestData),
    SilentPaymentAddress(SilentPaymentAddress),
    LnurlAuth(LnurlAuthRequestData),
    Url(String),
    Bip21(Bip21),
    Bolt12InvoiceRequest(Bolt12InvoiceRequest),
    LnurlWithdraw(LnurlWithdrawRequestData),
}

#[macros::extern_wasm_bindgen(breez_sdk_common::input::BitcoinAddress)]
pub struct BitcoinAddress {
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

#[macros::extern_wasm_bindgen(breez_sdk_common::input::DetailedBolt11Invoice)]
pub struct DetailedBolt11Invoice {
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

#[macros::extern_wasm_bindgen(breez_sdk_common::input::DetailedBolt12Invoice)]
pub struct DetailedBolt12Invoice {
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

#[macros::extern_wasm_bindgen(breez_sdk_common::input::DetailedBolt12Offer)]
pub struct DetailedBolt12Offer {
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

#[macros::extern_wasm_bindgen(breez_sdk_common::input::LightningAddress)]
pub struct LightningAddress {
    pub address: String,
    pub pay_request: LnurlPayRequestData,
}

#[macros::extern_wasm_bindgen(breez_sdk_common::lnurl::pay::LnurlPayRequestData)]
pub struct LnurlPayRequestData {
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

#[macros::extern_wasm_bindgen(breez_sdk_common::input::SilentPaymentAddress)]

pub struct SilentPaymentAddress {
    pub address: String,
    pub network: BitcoinNetwork,
    pub source: PaymentRequestSource,
}

#[macros::extern_wasm_bindgen(breez_sdk_common::lnurl::auth::LnurlAuthRequestData)]
pub struct LnurlAuthRequestData {
    pub k1: String,
    pub action: Option<String>,
    pub domain: String,
    pub url: String,
}

#[macros::extern_wasm_bindgen(breez_sdk_common::input::Bip21)]
pub struct Bip21 {
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

#[macros::extern_wasm_bindgen(breez_sdk_common::input::Bolt12InvoiceRequest)]
pub struct Bolt12InvoiceRequest {
    // TODO: Fill fields
}

#[macros::extern_wasm_bindgen(breez_sdk_common::input::LnurlWithdrawRequestData)]

pub struct LnurlWithdrawRequestData {
    pub callback: String,
    pub k1: String,
    pub default_description: String,
    pub min_withdrawable: u64,
    pub max_withdrawable: u64,
}

#[macros::extern_wasm_bindgen(breez_sdk_core::PaymentType)]
pub enum PaymentType {
    Send,
    Receive,
}

#[macros::extern_wasm_bindgen(breez_sdk_core::PaymentStatus)]
pub enum PaymentStatus {
    Completed,
    Pending,
    Failed,
}

#[macros::extern_wasm_bindgen(breez_sdk_core::Payment)]
pub struct Payment {
    pub id: String,
    pub payment_type: PaymentType,
    pub status: PaymentStatus,
    pub amount: u64,
    pub fees: u64,
    pub timestamp: u64,
    pub details: PaymentDetails,
}

#[macros::extern_wasm_bindgen(breez_sdk_core::PaymentDetails)]
pub enum PaymentDetails {
    Spark,
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

#[macros::extern_wasm_bindgen(breez_sdk_core::LnurlPayInfo)]
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

#[macros::extern_wasm_bindgen(breez_sdk_core::Network)]
pub enum Network {
    Mainnet,
    Regtest,
}

#[macros::extern_wasm_bindgen(breez_sdk_core::Config)]
pub struct Config {
    pub network: Network,
    pub deposits_monitoring_interval_secs: u32,
    pub max_deposit_claim_fee: Option<Fee>,
}

#[macros::extern_wasm_bindgen(breez_sdk_core::Fee)]
pub enum Fee {
    Fixed { amount: u64 },
    Rate { sat_per_vbyte: u64 },
}

#[macros::extern_wasm_bindgen(breez_sdk_core::Credentials)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

#[macros::extern_wasm_bindgen(breez_sdk_core::GetInfoRequest)]
pub struct GetInfoRequest {}

#[macros::extern_wasm_bindgen(breez_sdk_core::GetInfoResponse)]
pub struct GetInfoResponse {
    pub balance_sats: u64,
}

#[macros::extern_wasm_bindgen(breez_sdk_core::SyncWalletRequest)]
pub struct SyncWalletRequest {}

#[macros::extern_wasm_bindgen(breez_sdk_core::SyncWalletResponse)]
pub struct SyncWalletResponse {}

#[macros::extern_wasm_bindgen(breez_sdk_core::ReceivePaymentMethod)]
pub enum ReceivePaymentMethod {
    SparkAddress,
    BitcoinAddress,
    Bolt11Invoice {
        description: String,
        amount_sats: Option<u64>,
    },
}

#[macros::extern_wasm_bindgen(breez_sdk_core::SendPaymentMethod)]
pub enum SendPaymentMethod {
    BitcoinAddress {
        address: BitcoinAddress,
    },
    Bolt11Invoice {
        detailed_invoice: DetailedBolt11Invoice,
    }, // should be replaced with the parsed invoice
    SparkAddress {
        address: String,
    },
}

#[macros::extern_wasm_bindgen(breez_sdk_core::PrepareReceivePaymentRequest)]
pub struct PrepareReceivePaymentRequest {
    pub payment_method: ReceivePaymentMethod,
}

#[macros::extern_wasm_bindgen(breez_sdk_core::PrepareReceivePaymentResponse)]
pub struct PrepareReceivePaymentResponse {
    pub payment_method: ReceivePaymentMethod,
    pub fee_sats: u64,
}

#[macros::extern_wasm_bindgen(breez_sdk_core::ReceivePaymentRequest)]
pub struct ReceivePaymentRequest {
    pub prepare_response: PrepareReceivePaymentResponse,
}

#[macros::extern_wasm_bindgen(breez_sdk_core::ReceivePaymentResponse)]
pub struct ReceivePaymentResponse {
    pub payment_request: String,
}

#[macros::extern_wasm_bindgen(breez_sdk_core::PrepareSendPaymentRequest)]
pub struct PrepareSendPaymentRequest {
    pub payment_request: String,
    pub amount_sats: Option<u64>,
    pub prefer_spark: Option<bool>,
}

#[macros::extern_wasm_bindgen(breez_sdk_core::PrepareSendPaymentResponse)]
pub struct PrepareSendPaymentResponse {
    pub payment_method: SendPaymentMethod,
    pub amount_sats: u64,
    pub fee_sats: u64,
    pub prefer_spark: bool,
}

#[macros::extern_wasm_bindgen(breez_sdk_core::SendPaymentRequest)]
pub struct SendPaymentRequest {
    pub prepare_response: PrepareSendPaymentResponse,
}

#[macros::extern_wasm_bindgen(breez_sdk_core::SendPaymentResponse)]
pub struct SendPaymentResponse {
    pub payment: Payment,
}

#[macros::extern_wasm_bindgen(breez_sdk_core::ListPaymentsRequest)]
pub struct ListPaymentsRequest {
    pub offset: Option<u32>,
    pub limit: Option<u32>,
}

#[macros::extern_wasm_bindgen(breez_sdk_core::ListPaymentsResponse)]
pub struct ListPaymentsResponse {
    pub payments: Vec<Payment>,
}

#[macros::extern_wasm_bindgen(breez_sdk_core::GetPaymentRequest)]
pub struct GetPaymentRequest {
    pub payment_id: String,
}

#[macros::extern_wasm_bindgen(breez_sdk_core::GetPaymentResponse)]
pub struct GetPaymentResponse {
    pub payment: Payment,
}

#[macros::extern_wasm_bindgen(breez_sdk_core::LogEntry)]
pub struct LogEntry {
    pub line: String,
    pub level: String,
}
