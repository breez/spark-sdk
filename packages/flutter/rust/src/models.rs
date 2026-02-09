pub use breez_sdk_spark::sync_storage::*;
pub use breez_sdk_spark::*;
use flutter_rust_bridge::frb;
use std::collections::HashMap;

#[frb(mirror(BitcoinAddressDetails))]
pub struct _BitcoinAddressDetails {
    pub address: String,
    pub network: BitcoinNetwork,
    pub source: PaymentRequestSource,
}

#[frb(mirror(BitcoinNetwork))]
pub enum _BitcoinNetwork {
    Bitcoin,
    Testnet3,
    Testnet4,
    Signet,
    Regtest,
}

#[frb(mirror(Config))]
pub struct _Config {
    pub api_key: Option<String>,
    pub network: Network,
    pub sync_interval_secs: u32,
    pub max_deposit_claim_fee: Option<MaxFee>,
    pub lnurl_domain: Option<String>,
    pub prefer_spark_over_lightning: bool,
    pub external_input_parsers: Option<Vec<ExternalInputParser>>,
    pub use_default_external_input_parsers: bool,
    pub real_time_sync_server_url: Option<String>,
    pub private_enabled_default: bool,
    pub optimization_config: OptimizationConfig,
}

#[frb(mirror(OptimizationConfig))]
pub struct _OptimizationConfig {
    pub auto_enabled: bool,
    pub multiplicity: u8,
}

#[frb(mirror(ExternalInputParser))]
pub struct _ExternalInputParser {
    pub provider_id: String,
    pub input_regex: String,
    pub parser_url: String,
}

#[frb(mirror(Seed))]
pub enum _Seed {
    Mnemonic {
        mnemonic: String,
        passphrase: Option<String>,
    },
    Entropy(Vec<u8>),
}

#[frb(mirror(ConnectRequest))]
pub struct _ConnectRequest {
    pub config: Config,
    pub seed: Seed,
    pub storage_dir: String,
}

#[frb(mirror(KeySetConfig))]
pub struct _KeySetConfig {
    pub key_set_type: KeySetType,
    pub use_address_index: bool,
    pub account_number: Option<u32>,
}

#[frb(mirror(CheckMessageRequest))]
pub struct _CheckMessageRequest {
    pub message: String,
    pub pubkey: String,
    pub signature: String,
}

#[frb(mirror(CheckMessageResponse))]
pub struct _CheckMessageResponse {
    pub is_valid: bool,
}

#[frb(mirror(ClaimDepositRequest))]
pub struct _ClaimDepositRequest {
    pub txid: String,
    pub vout: u32,
    pub max_fee: Option<MaxFee>,
}

#[frb(mirror(ClaimDepositResponse))]
pub struct _ClaimDepositResponse {
    pub payment: Payment,
}

#[frb(mirror(Credentials))]
pub struct _Credentials {
    pub username: String,
    pub password: String,
}

#[frb(mirror(DepositInfo))]
pub struct _DepositInfo {
    pub txid: String,
    pub vout: u32,
    pub amount_sats: u64,
    pub refund_tx: Option<String>,
    pub refund_tx_id: Option<String>,
    pub claim_error: Option<DepositClaimError>,
}

#[frb(mirror(MaxFee))]
pub enum _MaxFee {
    Fixed { amount: u64 },
    Rate { sat_per_vbyte: u64 },
    NetworkRecommended { leeway_sat_per_vbyte: u64 },
}

#[frb(mirror(Fee))]
pub enum _Fee {
    Fixed { amount: u64 },
    Rate { sat_per_vbyte: u64 },
}

#[frb(mirror(GetInfoRequest))]
pub struct _GetInfoRequest {
    pub ensure_synced: Option<bool>,
}

#[frb(mirror(GetInfoResponse))]
pub struct _GetInfoResponse {
    pub identity_pubkey: String,
    pub balance_sats: u64,
    pub token_balances: HashMap<String, TokenBalance>,
}

#[frb(mirror(TokenBalance))]
pub struct _TokenBalance {
    pub balance: u128,
    pub token_metadata: TokenMetadata,
}

#[frb(mirror(TokenMetadata))]
pub struct _TokenMetadata {
    pub identifier: String,
    pub issuer_public_key: String,
    pub name: String,
    pub ticker: String,
    pub decimals: u32,
    pub max_supply: u128,
    pub is_freezable: bool,
}

#[frb(mirror(GetPaymentRequest))]
pub struct _GetPaymentRequest {
    pub payment_id: String,
}

#[frb(mirror(GetPaymentResponse))]
pub struct _GetPaymentResponse {
    pub payment: Payment,
}

#[frb(mirror(InputType))]
pub enum _InputType {
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
    SparkInvoice(SparkInvoiceDetails),
}

#[frb(mirror(PaymentDetailsFilter))]
pub enum _PaymentDetailsFilter {
    Spark {
        htlc_status: Option<Vec<SparkHtlcStatus>>,
        conversion_refund_needed: Option<bool>,
    },
    Token {
        conversion_refund_needed: Option<bool>,
        tx_hash: Option<String>,
        tx_type: Option<TokenTransactionType>,
    },
}

#[frb(mirror(ListPaymentsRequest))]
pub struct _ListPaymentsRequest {
    pub type_filter: Option<Vec<PaymentType>>,
    pub status_filter: Option<Vec<PaymentStatus>>,
    pub asset_filter: Option<AssetFilter>,
    pub payment_details_filter: Option<Vec<PaymentDetailsFilter>>,
    pub from_timestamp: Option<u64>,
    pub to_timestamp: Option<u64>,
    pub offset: Option<u32>,
    pub limit: Option<u32>,
    pub sort_ascending: Option<bool>,
}

#[frb(mirror(AssetFilter))]
pub enum _AssetFilter {
    Bitcoin,
    Token { token_identifier: Option<String> },
}

#[frb(mirror(ListPaymentsResponse))]
pub struct _ListPaymentsResponse {
    pub payments: Vec<Payment>,
}

#[frb(mirror(ListUnclaimedDepositsRequest))]
pub struct _ListUnclaimedDepositsRequest {}

#[frb(mirror(ListUnclaimedDepositsResponse))]
pub struct _ListUnclaimedDepositsResponse {
    pub deposits: Vec<DepositInfo>,
}

#[frb(mirror(LnurlPayInfo))]
pub struct _LnurlPayInfo {
    pub ln_address: Option<String>,
    pub comment: Option<String>,
    pub domain: Option<String>,
    pub metadata: Option<String>,
    pub processed_success_action: Option<SuccessActionProcessed>,
    pub raw_success_action: Option<SuccessAction>,
}

#[frb(mirror(LnurlPayRequest))]
pub struct _LnurlPayRequest {
    pub prepare_response: PrepareLnurlPayResponse,
    pub idempotency_key: Option<String>,
}

#[frb(mirror(LnurlPayResponse))]
pub struct _LnurlPayResponse {
    pub payment: Payment,
    pub success_action: Option<SuccessActionProcessed>,
}

#[frb(mirror(LnurlWithdrawInfo))]
pub struct _LnurlWithdrawInfo {
    pub withdraw_url: String,
}

#[frb(mirror(LnurlReceiveMetadata))]
pub struct _LnurlReceiveMetadata {
    pub nostr_zap_request: Option<String>,
    pub nostr_zap_receipt: Option<String>,
    pub sender_comment: Option<String>,
}

#[frb(mirror(LnurlWithdrawRequest))]
pub struct _LnurlWithdrawRequest {
    pub amount_sats: u64,
    pub withdraw_request: LnurlWithdrawRequestDetails,
    pub completion_timeout_secs: Option<u32>,
}

#[frb(mirror(LnurlWithdrawResponse))]
pub struct _LnurlWithdrawResponse {
    pub payment_request: String,
    pub payment: Option<Payment>,
}

#[frb(mirror(LnurlErrorDetails))]
pub struct _LnurlErrorDetails {
    pub reason: String,
}

#[frb(mirror(LnurlCallbackStatus))]
pub enum _LnurlCallbackStatus {
    Ok,
    ErrorStatus { error_details: LnurlErrorDetails },
}

#[frb(mirror(OnchainConfirmationSpeed))]
pub enum _OnchainConfirmationSpeed {
    Fast,
    Medium,
    Slow,
}

#[frb(mirror(FeePolicy))]
pub enum _FeePolicy {
    /// Fees are added on top of the specified amount (default behavior).
    FeesExcluded,
    /// Fees are deducted from the specified amount.
    FeesIncluded,
}

#[frb(mirror(PrepareLnurlPayRequest))]
pub struct _PrepareLnurlPayRequest {
    pub amount_sats: u64,
    pub pay_request: LnurlPayRequestDetails,
    pub comment: Option<String>,
    pub validate_success_action_url: Option<bool>,
    pub conversion_options: Option<ConversionOptions>,
    pub fee_policy: Option<FeePolicy>,
}

#[frb(mirror(PrepareLnurlPayResponse))]
pub struct _PrepareLnurlPayResponse {
    pub amount_sats: u64,
    pub comment: Option<String>,
    pub pay_request: LnurlPayRequestDetails,
    pub fee_sats: u64,
    pub invoice_details: Bolt11InvoiceDetails,
    pub success_action: Option<SuccessAction>,
    pub conversion_estimate: Option<ConversionEstimate>,
    pub fee_policy: FeePolicy,
}

#[frb(mirror(PrepareSendPaymentRequest))]
pub struct _PrepareSendPaymentRequest {
    pub payment_request: String,
    pub amount: Option<u128>,
    pub token_identifier: Option<String>,
    pub conversion_options: Option<ConversionOptions>,
    pub fee_policy: Option<FeePolicy>,
}

#[frb(mirror(PrepareSendPaymentResponse))]
pub struct _PrepareSendPaymentResponse {
    pub payment_method: SendPaymentMethod,
    pub amount: u128,
    pub token_identifier: Option<String>,
    pub conversion_estimate: Option<ConversionEstimate>,
    pub fee_policy: FeePolicy,
}

#[frb(mirror(ReceivePaymentMethod))]
pub enum _ReceivePaymentMethod {
    SparkAddress,
    SparkInvoice {
        amount: Option<u128>,
        token_identifier: Option<String>,
        expiry_time: Option<u64>,
        description: Option<String>,
        sender_public_key: Option<String>,
    },
    BitcoinAddress,
    Bolt11Invoice {
        description: String,
        amount_sats: Option<u64>,
        expiry_secs: Option<u32>,
    },
}

#[frb(mirror(ReceivePaymentRequest))]
pub struct _ReceivePaymentRequest {
    pub payment_method: ReceivePaymentMethod,
}

#[frb(mirror(ReceivePaymentResponse))]
pub struct _ReceivePaymentResponse {
    pub payment_request: String,
    pub fee: u128,
}

#[frb(mirror(RefundDepositRequest))]
pub struct _RefundDepositRequest {
    pub txid: String,
    pub vout: u32,
    pub destination_address: String,
    pub fee: Fee,
}

#[frb(mirror(RefundDepositResponse))]
pub struct _RefundDepositResponse {
    pub tx_id: String,
    pub tx_hex: String,
}

#[frb(mirror(SendOnchainFeeQuote))]
pub struct _SendOnchainFeeQuote {
    pub id: String,
    pub expires_at: u64,
    pub speed_fast: SendOnchainSpeedFeeQuote,
    pub speed_medium: SendOnchainSpeedFeeQuote,
    pub speed_slow: SendOnchainSpeedFeeQuote,
}

#[frb(mirror(SendOnchainSpeedFeeQuote))]
pub struct _SendOnchainSpeedFeeQuote {
    pub user_fee_sat: u64,
    pub l1_broadcast_fee_sat: u64,
}

#[frb(mirror(SendPaymentMethod))]
pub enum _SendPaymentMethod {
    BitcoinAddress {
        address: BitcoinAddressDetails,
        fee_quote: SendOnchainFeeQuote,
    },
    Bolt11Invoice {
        invoice_details: Bolt11InvoiceDetails,
        spark_transfer_fee_sats: Option<u64>,
        lightning_fee_sats: u64,
    },
    SparkAddress {
        address: String,
        fee: u128,
        token_identifier: Option<String>,
    },
    SparkInvoice {
        spark_invoice_details: SparkInvoiceDetails,
        fee: u128,
        token_identifier: Option<String>,
    },
}

#[frb(mirror(SendPaymentOptions))]
pub enum _SendPaymentOptions {
    BitcoinAddress {
        confirmation_speed: OnchainConfirmationSpeed,
    },
    Bolt11Invoice {
        prefer_spark: bool,
        completion_timeout_secs: Option<u32>,
    },
    SparkAddress {
        htlc_options: Option<SparkHtlcOptions>,
    },
}

#[frb(mirror(SparkHtlcOptions))]
pub struct _SparkHtlcOptions {
    pub payment_hash: String,
    pub expiry_duration_secs: u64,
}

#[frb(mirror(SendPaymentRequest))]
pub struct _SendPaymentRequest {
    pub prepare_response: PrepareSendPaymentResponse,
    pub options: Option<SendPaymentOptions>,
    pub idempotency_key: Option<String>,
}

#[frb(mirror(SendPaymentResponse))]
pub struct _SendPaymentResponse {
    pub payment: Payment,
}

#[frb(mirror(SignMessageRequest))]
pub struct _SignMessageRequest {
    pub message: String,
    pub compact: bool,
}

#[frb(mirror(SignMessageResponse))]
pub struct _SignMessageResponse {
    pub pubkey: String,
    pub signature: String,
}

#[frb(mirror(SuccessAction))]
pub enum _SuccessAction {
    Aes { data: AesSuccessActionData },
    Message { data: MessageSuccessActionData },
    Url { data: UrlSuccessActionData },
}

#[frb(mirror(SuccessActionProcessed))]
pub enum _SuccessActionProcessed {
    Aes { result: AesSuccessActionDataResult },
    Message { data: MessageSuccessActionData },
    Url { data: UrlSuccessActionData },
}

#[frb(mirror(SyncWalletRequest))]
pub struct _SyncWalletRequest {}

#[frb(mirror(SyncWalletResponse))]
pub struct _SyncWalletResponse {}

#[frb(mirror(AesSuccessActionData))]
pub struct _AesSuccessActionData {
    pub description: String,
    pub ciphertext: String,
    pub iv: String,
}

#[frb(mirror(AesSuccessActionDataResult))]
pub enum _AesSuccessActionDataResult {
    Decrypted { data: AesSuccessActionDataDecrypted },
    ErrorStatus { reason: String },
}

#[frb(mirror(AesSuccessActionDataDecrypted))]
pub struct _AesSuccessActionDataDecrypted {
    pub description: String,
    pub plaintext: String,
}

#[frb(mirror(MessageSuccessActionData))]
pub struct _MessageSuccessActionData {
    pub message: String,
}

#[frb(mirror(UrlSuccessActionData))]
pub struct _UrlSuccessActionData {
    pub description: String,
    pub url: String,
    pub matches_callback_domain: bool,
}

#[frb(mirror(Network))]
pub enum _Network {
    Mainnet,
    Regtest,
}

#[frb(mirror(Payment))]
pub struct _Payment {
    pub id: String,
    pub payment_type: PaymentType,
    pub status: PaymentStatus,
    pub amount: u128,
    pub fees: u128,
    pub timestamp: u64,
    pub method: PaymentMethod,
    pub details: Option<PaymentDetails>,
    pub conversion_details: Option<ConversionDetails>,
}

#[frb(mirror(ConversionDetails))]
pub struct _ConversionDetails {
    pub from: ConversionStep,
    pub to: ConversionStep,
}

#[frb(mirror(ConversionStep))]
pub struct _ConversionStep {
    pub payment_id: String,
    pub amount: u128,
    pub fee: u128,
    pub method: PaymentMethod,
    pub token_metadata: Option<TokenMetadata>,
}

#[frb(mirror(PaymentDetails))]
pub enum _PaymentDetails {
    Spark {
        invoice_details: Option<SparkInvoicePaymentDetails>,
        htlc_details: Option<SparkHtlcDetails>,
        conversion_info: Option<ConversionInfo>,
    },
    Token {
        metadata: TokenMetadata,
        tx_hash: String,
        tx_type: TokenTransactionType,
        invoice_details: Option<SparkInvoicePaymentDetails>,
        conversion_info: Option<ConversionInfo>,
    },
    Lightning {
        description: Option<String>,
        preimage: Option<String>,
        invoice: String,
        payment_hash: String,
        destination_pubkey: String,
        lnurl_pay_info: Option<LnurlPayInfo>,
        lnurl_withdraw_info: Option<LnurlWithdrawInfo>,
        lnurl_receive_metadata: Option<LnurlReceiveMetadata>,
    },
    Withdraw {
        tx_id: String,
    },
    Deposit {
        tx_id: String,
    },
}

#[frb(mirror(TokenTransactionType))]
pub enum _TokenTransactionType {
    Transfer,
    Mint,
    Burn,
}

#[frb(mirror(SparkInvoicePaymentDetails))]
pub struct _SparkInvoicePaymentDetails {
    pub description: Option<String>,
    pub invoice: String,
}

#[frb(mirror(SparkHtlcDetails))]
pub struct _SparkHtlcDetails {
    pub payment_hash: String,
    pub preimage: Option<String>,
    pub expiry_time: u64,
    pub status: SparkHtlcStatus,
}

#[frb(mirror(SparkHtlcStatus))]
pub enum _SparkHtlcStatus {
    WaitingForPreimage,
    PreimageShared,
    Returned,
}

#[frb(mirror(PaymentMetadata))]
pub struct _PaymentMetadata {
    pub lnurl_pay_info: Option<LnurlPayInfo>,
    pub lnurl_withdraw_info: Option<LnurlWithdrawInfo>,
    pub lnurl_description: Option<String>,
    pub conversion_info: Option<ConversionInfo>,
}

#[frb(mirror(PaymentMethod))]
pub enum _PaymentMethod {
    Lightning,
    Spark,
    Token,
    Deposit,
    Withdraw,
    Unknown,
}

#[frb(mirror(PaymentRequestSource))]
pub struct _PaymentRequestSource {
    pub bip_21_uri: Option<String>,
    pub bip_353_address: Option<String>,
}

#[frb(mirror(PaymentStatus))]
pub enum _PaymentStatus {
    Completed,
    Pending,
    Failed,
}

#[frb(mirror(PaymentType))]
pub enum _PaymentType {
    Send,
    Receive,
}

#[frb(mirror(UpdateDepositPayload))]
pub enum _UpdateDepositPayload {
    ClaimError {
        error: DepositClaimError,
    },
    Refund {
        refund_txid: String,
        refund_tx: String,
    },
}

#[frb(mirror(Amount))]
pub enum _Amount {
    Bitcoin {
        amount_msat: u64,
    },
    Currency {
        iso4217_code: String,
        fractional_amount: u64,
    },
}

#[frb(mirror(Bip21Details))]
pub struct _Bip21Details {
    pub amount_sat: Option<u64>,
    pub asset_id: Option<String>,
    pub uri: String,
    pub extras: Vec<Bip21Extra>,
    pub label: Option<String>,
    pub message: Option<String>,
    pub payment_methods: Vec<InputType>,
}

#[frb(mirror(Bip21Extra))]
pub struct _Bip21Extra {
    pub key: String,
    pub value: String,
}

#[frb(mirror(Bolt11Invoice))]
pub struct _Bolt11Invoice {
    pub bolt11: String,
    pub source: PaymentRequestSource,
}

#[frb(mirror(Bolt11RouteHint))]
pub struct _Bolt11RouteHint {
    pub hops: Vec<Bolt11RouteHintHop>,
}

#[frb(mirror(Bolt11RouteHintHop))]
pub struct _Bolt11RouteHintHop {
    pub src_node_id: String,
    pub short_channel_id: String,
    pub fees_base_msat: u32,
    pub fees_proportional_millionths: u32,
    pub cltv_expiry_delta: u16,
    pub htlc_minimum_msat: Option<u64>,
    pub htlc_maximum_msat: Option<u64>,
}

#[frb(mirror(Bolt12Invoice))]
pub struct _Bolt12Invoice {
    pub invoice: String,
    pub source: PaymentRequestSource,
}

#[frb(mirror(Bolt12InvoiceRequestDetails))]
pub struct _Bolt12InvoiceRequestDetails {
    // TODO: Fill fields
}

#[frb(mirror(Bolt12OfferBlindedPath))]
pub struct _Bolt12OfferBlindedPath {
    pub blinded_hops: Vec<String>,
}

#[frb(mirror(Bolt11InvoiceDetails))]
pub struct _Bolt11InvoiceDetails {
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

#[frb(mirror(Bolt12InvoiceDetails))]
pub struct _Bolt12InvoiceDetails {
    pub amount_msat: u64,
    pub invoice: Bolt12Invoice,
}

#[frb(mirror(Bolt12Offer))]
pub struct _Bolt12Offer {
    pub offer: String,
    pub source: PaymentRequestSource,
}

#[frb(mirror(Bolt12OfferDetails))]
pub struct _Bolt12OfferDetails {
    pub absolute_expiry: Option<u64>,
    pub chains: Vec<String>,
    pub description: Option<String>,
    pub issuer: Option<String>,
    pub min_amount: Option<Amount>,
    pub offer: Bolt12Offer,
    pub paths: Vec<Bolt12OfferBlindedPath>,
    pub signing_pubkey: Option<String>,
}

#[frb(mirror(SparkAddressDetails))]
pub struct _SparkAddressDetails {
    pub address: String,
    pub identity_public_key: String,
    pub network: BitcoinNetwork,
    pub source: PaymentRequestSource,
}

#[frb(mirror(SparkInvoiceDetails))]
pub struct _SparkInvoiceDetails {
    pub invoice: String,
    pub identity_public_key: String,
    pub network: BitcoinNetwork,
    pub amount: Option<u128>,
    pub token_identifier: Option<String>,
    pub expiry_time: Option<u64>,
    pub description: Option<String>,
    pub sender_public_key: Option<String>,
}

#[frb(mirror(SparkInvoicePaymentType))]
pub enum _SparkInvoicePaymentType {
    Sats,
    Tokens { token_identifier: Option<String> },
}

#[frb(mirror(LightningAddressDetails))]
pub struct _LightningAddressDetails {
    pub address: String,
    pub pay_request: LnurlPayRequestDetails,
}

#[frb(mirror(LnurlAuthRequestDetails))]
pub struct _LnurlAuthRequestDetails {
    pub k1: String,
    pub action: Option<String>,
    pub domain: String,
    pub url: String,
}

#[frb(mirror(LnurlPayRequestDetails))]
pub struct _LnurlPayRequestDetails {
    pub callback: String,
    pub min_sendable: u64,
    pub max_sendable: u64,
    pub metadata_str: String,
    pub comment_allowed: u16,
    pub domain: String,
    pub url: String,
    pub address: Option<String>,
    pub allows_nostr: Option<bool>,
    pub nostr_pubkey: Option<String>,
}

#[frb(mirror(LnurlWithdrawRequestDetails))]
pub struct _LnurlWithdrawRequestDetails {
    pub callback: String,
    pub k1: String,
    pub default_description: String,
    pub min_withdrawable: u64,
    pub max_withdrawable: u64,
}

#[frb(mirror(SilentPaymentAddressDetails))]
pub struct _SilentPaymentAddressDetails {
    pub address: String,
    pub network: BitcoinNetwork,
    pub source: PaymentRequestSource,
}

#[frb(mirror(CheckLightningAddressRequest))]
pub struct _CheckLightningAddressRequest {
    pub username: String,
}

#[frb(mirror(RegisterLightningAddressRequest))]
pub struct _RegisterLightningAddressRequest {
    pub username: String,
    pub description: Option<String>,
}

#[frb(mirror(LnurlInfo))]
pub struct _LnurlInfo {
    pub url: String,
    pub bech32: String,
}

#[frb(mirror(LightningAddressInfo))]
pub struct _LightningAddressInfo {
    pub description: String,
    pub lightning_address: String,
    pub lnurl: LnurlInfo,
    pub username: String,
}

#[frb(mirror(KeySetType))]
pub enum _KeySetType {
    Default,
    Taproot,
    NativeSegwit,
    WrappedSegwit,
    Legacy,
}

#[frb(mirror(ListFiatCurrenciesResponse))]
pub struct _ListFiatCurrenciesResponse {
    pub currencies: Vec<FiatCurrency>,
}

#[frb(mirror(ListFiatRatesResponse))]
pub struct _ListFiatRatesResponse {
    pub rates: Vec<Rate>,
}

#[frb(mirror(Rate))]
pub struct _Rate {
    pub coin: String,
    pub value: f64,
}

#[frb(mirror(FiatCurrency))]
pub struct _FiatCurrency {
    pub id: String,
    pub info: CurrencyInfo,
}

#[frb(mirror(CurrencyInfo))]
pub struct _CurrencyInfo {
    pub name: String,
    pub fraction_size: u32,
    pub spacing: Option<u32>,
    pub symbol: Option<Symbol>,
    pub uniq_symbol: Option<Symbol>,
    pub localized_name: Vec<LocalizedName>,
    pub locale_overrides: Vec<LocaleOverrides>,
}

#[frb(mirror(LocaleOverrides))]
pub struct _LocaleOverrides {
    pub locale: String,
    pub spacing: Option<u32>,
    pub symbol: Symbol,
}

#[frb(mirror(LocalizedName))]
pub struct _LocalizedName {
    pub locale: String,
    pub name: String,
}

#[frb(mirror(Symbol))]
pub struct _Symbol {
    pub grapheme: Option<String>,
    pub template: Option<String>,
    pub rtl: Option<bool>,
    pub position: Option<u32>,
}

#[frb(mirror(GetTokensMetadataRequest))]
pub struct _GetTokensMetadataRequest {
    pub token_identifiers: Vec<String>,
}

#[frb(mirror(GetTokensMetadataResponse))]
pub struct _GetTokensMetadataResponse {
    pub tokens_metadata: Vec<TokenMetadata>,
}

#[frb(mirror(RecordId))]
pub struct _RecordId {
    pub r#type: String,
    pub data_id: String,
}

#[frb(mirror(Record))]
pub struct _Record {
    pub id: RecordId,
    pub revision: u64,
    pub schema_version: String,
    pub data: HashMap<String, String>,
}

#[frb(mirror(IncomingChange))]
pub struct _IncomingChange {
    pub new_state: breez_sdk_spark::sync_storage::Record,
    pub old_state: Option<breez_sdk_spark::sync_storage::Record>,
    // pub pending_outgoing_changes: Vec<RecordChange>,
}

#[frb(mirror(OutgoingChange))]
pub struct _OutgoingChange {
    pub change: breez_sdk_spark::sync_storage::RecordChange,
    pub parent: Option<breez_sdk_spark::sync_storage::Record>,
}

#[frb(mirror(UnversionedRecordChange))]
pub struct _UnversionedRecordChange {
    pub id: RecordId,
    pub schema_version: String,
    pub updated_fields: HashMap<String, String>,
}

#[frb(mirror(RecordChange))]
pub struct _RecordChange {
    pub id: RecordId,
    pub schema_version: String,
    pub updated_fields: HashMap<String, String>,
    pub revision: u64,
}

#[frb(mirror(UserSettings))]
pub struct _UserSettings {
    pub spark_private_mode_enabled: bool,
}

#[frb(mirror(UpdateUserSettingsRequest))]
pub struct _UpdateUserSettingsRequest {
    pub spark_private_mode_enabled: Option<bool>,
}

#[frb(mirror(CreateIssuerTokenRequest))]
pub struct _CreateIssuerTokenRequest {
    pub name: String,
    pub ticker: String,
    pub decimals: u32,
    pub is_freezable: bool,
    pub max_supply: u128,
}

#[frb(mirror(MintIssuerTokenRequest))]
pub struct _MintIssuerTokenRequest {
    pub amount: u128,
}

#[frb(mirror(BurnIssuerTokenRequest))]
pub struct _BurnIssuerTokenRequest {
    pub amount: u128,
}

#[frb(mirror(FreezeIssuerTokenRequest))]
pub struct _FreezeIssuerTokenRequest {
    pub address: String,
}

#[frb(mirror(FreezeIssuerTokenResponse))]
pub struct _FreezeIssuerTokenResponse {
    pub impacted_output_ids: Vec<String>,
    pub impacted_token_amount: u128,
}

#[frb(mirror(UnfreezeIssuerTokenRequest))]
pub struct _UnfreezeIssuerTokenRequest {
    pub address: String,
}

#[frb(mirror(UnfreezeIssuerTokenResponse))]
pub struct _UnfreezeIssuerTokenResponse {
    pub impacted_output_ids: Vec<String>,
    pub impacted_token_amount: u128,
}

#[frb(mirror(RecommendedFees))]
pub struct _RecommendedFees {
    pub fastest_fee: u64,
    pub half_hour_fee: u64,
    pub hour_fee: u64,
    pub economy_fee: u64,
    pub minimum_fee: u64,
}

#[frb(mirror(ChainApiType))]
pub enum _ChainApiType {
    Esplora,
    MempoolSpace,
}

#[frb(mirror(ClaimHtlcPaymentRequest))]
pub struct _ClaimHtlcPaymentRequest {
    pub preimage: String,
}

#[frb(mirror(ClaimHtlcPaymentResponse))]
pub struct _ClaimHtlcPaymentResponse {
    pub payment: Payment,
}

#[frb(mirror(OptimizationProgress))]
pub struct _OptimizationProgress {
    pub is_running: bool,
    pub current_round: u32,
    pub total_rounds: u32,
}

#[frb(mirror(ConversionEstimate))]
pub struct _ConversionEstimate {
    pub options: ConversionOptions,
    pub amount: u128,
    pub fee: u128,
}

#[frb(mirror(ConversionPurpose))]
pub enum _ConversionPurpose {
    OngoingPayment {
        payment_request: String,
    },
    SelfTransfer,
}

#[frb(mirror(ConversionStatus))]
pub enum _ConversionStatus {
    Completed,
    RefundNeeded,
    Refunded,
}

#[frb(mirror(ConversionInfo))]
pub struct _ConversionInfo {
    pub pool_id: String,
    pub conversion_id: String,
    pub status: ConversionStatus,
    pub fee: Option<u128>,
    pub purpose: Option<ConversionPurpose>,
}

#[frb(mirror(ConversionOptions))]
pub struct _ConversionOptions {
    pub conversion_type: ConversionType,
    pub max_slippage_bps: Option<u32>,
    pub completion_timeout_secs: Option<u32>,
}

#[frb(mirror(ConversionType))]
pub enum _ConversionType {
    FromBitcoin,
    ToBitcoin { from_token_identifier: String },
}

#[frb(mirror(FetchConversionLimitsRequest))]
pub struct _FetchConversionLimitsRequest {
    pub conversion_type: ConversionType,
    pub token_identifier: Option<String>,
}

#[frb(mirror(FetchConversionLimitsResponse))]
pub struct _FetchConversionLimitsResponse {
    pub min_from_amount: Option<u128>,
    pub min_to_amount: Option<u128>,
}

#[frb(mirror(BuyBitcoinRequest))]
pub struct _BuyBitcoinRequest {
    pub locked_amount_sat: Option<u64>,
    pub redirect_url: Option<String>,
}

#[frb(mirror(BuyBitcoinResponse))]
pub struct _BuyBitcoinResponse {
    pub url: String,
}

#[frb(mirror(ServiceStatus))]
pub enum _ServiceStatus {
    Operational,
    Degraded,
    Partial,
    Unknown,
    Major,
}

#[frb(mirror(SparkStatus))]
pub struct _SparkStatus {
    pub status: ServiceStatus,
    pub last_updated: u64,
}
