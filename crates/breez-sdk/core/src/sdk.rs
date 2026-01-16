use base64::Engine;
use bitcoin::{
    consensus::serialize,
    hashes::{Hash, sha256},
    hex::DisplayHex,
    secp256k1::{PublicKey, ecdsa::Signature},
};
use bitflags::bitflags;
use breez_sdk_common::{
    fiat::FiatService,
    lnurl::{self, withdraw::execute_lnurl_withdraw},
};
use breez_sdk_common::{
    lnurl::{
        error::LnurlError,
        pay::{
            AesSuccessActionDataResult, SuccessAction, SuccessActionProcessed, validate_lnurl_pay,
        },
    },
    rest::RestClient,
};
use flashnet::{
    ClawbackRequest, ClawbackResponse, ExecuteSwapRequest, FlashnetClient, FlashnetError,
    GetMinAmountsRequest, ListPoolsRequest, PoolSortOrder, SimulateSwapRequest,
};
use lnurl_models::sanitize_username;
use spark_wallet::{
    ExitSpeed, InvoiceDescription, ListTokenTransactionsRequest, ListTransfersRequest, Preimage,
    SparkAddress, SparkWallet, TransferId, TransferTokenOutput, WalletEvent, WalletTransfer,
};
use std::{collections::HashMap, str::FromStr, sync::Arc};
use tracing::{debug, error, info, trace, warn};
use web_time::{Duration, SystemTime};

use tokio::{
    select,
    sync::{Mutex, OnceCell, mpsc, oneshot, watch},
    time::timeout,
};
use tokio_with_wasm::alias as tokio;
use web_time::Instant;
use x509_parser::parse_x509_certificate;

use crate::{
    AssetFilter, BitcoinAddressDetails, BitcoinChainService, Bolt11InvoiceDetails,
    CheckLightningAddressRequest, CheckMessageRequest, CheckMessageResponse, ClaimDepositRequest,
    ClaimDepositResponse, ClaimHtlcPaymentRequest, ClaimHtlcPaymentResponse, ConversionEstimate,
    ConversionInfo, ConversionOptions, ConversionPurpose, ConversionStatus, ConversionType,
    DepositInfo, ExternalInputParser, FetchConversionLimitsRequest, FetchConversionLimitsResponse,
    GetPaymentRequest, GetPaymentResponse, GetTokensMetadataRequest, GetTokensMetadataResponse,
    InputType, LightningAddressInfo, ListFiatCurrenciesResponse, ListFiatRatesResponse,
    ListUnclaimedDepositsRequest, ListUnclaimedDepositsResponse, LnurlAuthRequestDetails,
    LnurlCallbackStatus, LnurlPayInfo, LnurlPayRequest, LnurlPayResponse, LnurlWithdrawInfo,
    LnurlWithdrawRequest, LnurlWithdrawResponse, Logger, MaxFee, Network, OnchainConfirmationSpeed,
    OptimizationConfig, OptimizationProgress, PaymentDetails, PaymentDetailsFilter, PaymentStatus,
    PaymentType, PrepareLnurlPayRequest, PrepareLnurlPayResponse, RefundDepositRequest,
    RefundDepositResponse, RegisterLightningAddressRequest, SendOnchainFeeQuote,
    SendPaymentOptions, SetLnurlMetadataItem, SignMessageRequest, SignMessageResponse,
    SparkHtlcOptions, SparkInvoiceDetails, TokenConversionPool, TokenConversionResponse,
    UpdateUserSettingsRequest, UserSettings, WaitForPaymentIdentifier,
    chain::RecommendedFees,
    error::SdkError,
    events::{EventEmitter, EventListener, InternalSyncedEvent, SdkEvent},
    issuer::TokenIssuer,
    lnurl::{ListMetadataRequest, LnurlServerClient, PublishZapReceiptRequest},
    logger,
    models::{
        Config, GetInfoRequest, GetInfoResponse, ListPaymentsRequest, ListPaymentsResponse,
        Payment, PrepareSendPaymentRequest, PrepareSendPaymentResponse, ReceivePaymentMethod,
        ReceivePaymentRequest, ReceivePaymentResponse, SendPaymentMethod, SendPaymentRequest,
        SendPaymentResponse, SyncWalletRequest, SyncWalletResponse,
    },
    nostr::NostrClient,
    persist::{
        CachedAccountInfo, ObjectCacheRepository, PaymentMetadata, StaticDepositAddress, Storage,
        UpdateDepositPayload,
    },
    sync::SparkSyncService,
    utils::{
        deposit_chain_syncer::DepositChainSyncer,
        run_with_shutdown,
        send_payment_validation::validate_prepare_send_payment_request,
        token::{
            get_tokens_metadata_cached_or_query, map_and_persist_token_transaction,
            token_transaction_to_payments,
        },
        utxo_fetcher::{CachedUtxoFetcher, DetailedUtxo},
    },
};

pub async fn parse_input(
    input: &str,
    external_input_parsers: Option<Vec<ExternalInputParser>>,
) -> Result<InputType, SdkError> {
    Ok(breez_sdk_common::input::parse(
        input,
        external_input_parsers.map(|parsers| parsers.into_iter().map(From::from).collect()),
    )
    .await?
    .into())
}

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
const BREEZ_SYNC_SERVICE_URL: &str = "https://datasync.breez.technology";

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
const BREEZ_SYNC_SERVICE_URL: &str = "https://datasync.breez.technology:442";

const CLAIM_TX_SIZE_VBYTES: u64 = 99;
const SYNC_PAGING_LIMIT: u32 = 100;
/// Default maximum slippage for conversions in basis points (0.5%)
const DEFAULT_TOKEN_CONVERSION_MAX_SLIPPAGE_BPS: u32 = 50;
/// Default timeout for conversion operations in seconds
const DEFAULT_TOKEN_CONVERSION_TIMEOUT_SECS: u32 = 30;

bitflags! {
    #[derive(Clone, Debug)]
    struct SyncType: u32 {
        const Wallet = 1 << 0;
        const WalletState = 1 << 1;
        const Deposits = 1 << 2;
        const LnurlMetadata = 1 << 3;
        const Full = Self::Wallet.0.0
            | Self::WalletState.0.0
            | Self::Deposits.0.0
            | Self::LnurlMetadata.0.0;
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SyncRequest {
    sync_type: SyncType,
    #[allow(clippy::type_complexity)]
    reply: Arc<Mutex<Option<oneshot::Sender<Result<(), SdkError>>>>>,
}

impl SyncRequest {
    fn new(reply: oneshot::Sender<Result<(), SdkError>>, sync_type: SyncType) -> Self {
        Self {
            sync_type,
            reply: Arc::new(Mutex::new(Some(reply))),
        }
    }

    fn full(reply: Option<oneshot::Sender<Result<(), SdkError>>>) -> Self {
        Self {
            sync_type: SyncType::Full,
            reply: Arc::new(Mutex::new(reply)),
        }
    }

    fn no_reply(sync_type: SyncType) -> Self {
        Self {
            sync_type,
            reply: Arc::new(Mutex::new(None)),
        }
    }

    async fn reply(&self, error: Option<SdkError>) {
        if let Some(reply) = self.reply.lock().await.take() {
            let _ = match error {
                Some(e) => reply.send(Err(e)),
                None => reply.send(Ok(())),
            };
        }
    }
}

#[derive(Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct SdkServices {
    pub(crate) config: Config,
    pub(crate) spark_wallet: Arc<SparkWallet>,
    pub(crate) storage: Arc<dyn Storage>,
    pub(crate) chain_service: Arc<dyn BitcoinChainService>,
    pub(crate) fiat_service: Arc<dyn FiatService>,
    pub(crate) lnurl_client: Arc<dyn RestClient>,
    pub(crate) lnurl_server_client: Option<Arc<dyn LnurlServerClient>>,
    pub(crate) lnurl_auth_signer: Arc<crate::signer::lnurl_auth::LnurlAuthSignerAdapter>,
    pub(crate) event_emitter: Arc<EventEmitter>,
    pub(crate) shutdown_sender: watch::Sender<()>,
    pub(crate) sync_trigger: tokio::sync::broadcast::Sender<SyncRequest>,
    pub(crate) zap_receipt_trigger: tokio::sync::broadcast::Sender<()>,
    pub(crate) conversion_refund_trigger: tokio::sync::broadcast::Sender<()>,
    pub(crate) initial_synced_watcher: watch::Receiver<bool>,
    pub(crate) external_input_parsers: Vec<ExternalInputParser>,
    pub(crate) spark_private_mode_initialized: Arc<OnceCell<()>>,
    pub(crate) nostr_client: Arc<NostrClient>,
    pub(crate) flashnet_client: Arc<FlashnetClient>,
}

/// `BreezSDK` is a wrapper around `SparkSDK` that provides a more structured API
/// with request/response objects and comprehensive error handling.
#[derive(Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct BreezSdk {
    pub(crate) services: Arc<SdkServices>,
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn init_logging(
    log_dir: Option<String>,
    app_logger: Option<Box<dyn Logger>>,
    log_filter: Option<String>,
) -> Result<(), SdkError> {
    logger::init_logging(log_dir, app_logger, log_filter)
}

/// Connects to the Spark network using the provided configuration and mnemonic.
///
/// # Arguments
///
/// * `request` - The connection request object
///
/// # Returns
///
/// Result containing either the initialized `BreezSdk` or an `SdkError`
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
pub async fn connect(request: crate::ConnectRequest) -> Result<BreezSdk, SdkError> {
    let builder = super::sdk_builder::SdkBuilder::new(request.config, request.seed)
        .with_default_storage(request.storage_dir);
    let sdk = builder.build().await?;
    Ok(sdk)
}

/// Connects to the Spark network using an external signer.
///
/// This method allows using a custom signer implementation instead of providing
/// a seed directly.
///
/// # Arguments
///
/// * `request` - The connection request object with external signer
///
/// # Returns
///
/// Result containing either the initialized `BreezSdk` or an `SdkError`
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
pub async fn connect_with_signer(
    request: crate::ConnectWithSignerRequest,
) -> Result<BreezSdk, SdkError> {
    let builder = super::sdk_builder::SdkBuilder::new_with_signer(request.config, request.signer)
        .with_default_storage(request.storage_dir);
    let sdk = builder.build().await?;
    Ok(sdk)
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn default_config(network: Network) -> Config {
    let lnurl_domain = match network {
        Network::Mainnet => Some("breez.tips".to_string()),
        Network::Regtest => None,
    };
    Config {
        api_key: None,
        network,
        sync_interval_secs: 60, // every 1 minute
        max_deposit_claim_fee: Some(MaxFee::Rate { sat_per_vbyte: 1 }),
        lnurl_domain,
        prefer_spark_over_lightning: false,
        external_input_parsers: None,
        use_default_external_input_parsers: true,
        real_time_sync_server_url: Some(BREEZ_SYNC_SERVICE_URL.to_string()),
        private_enabled_default: true,
        optimization_config: OptimizationConfig {
            auto_enabled: true,
            multiplicity: 1,
        },
    }
}

/// Creates a default external signer from a mnemonic.
///
/// This is a convenience factory method for creating a signer that can be used
/// with `connect_with_signer` or `SdkBuilder::new_with_signer`.
///
/// # Arguments
///
/// * `mnemonic` - BIP39 mnemonic phrase (12 or 24 words)
/// * `passphrase` - Optional passphrase for the mnemonic
/// * `network` - Network to use (Mainnet or Regtest)
/// * `key_set_config` - Optional key set configuration. If None, uses default configuration.
///
/// # Returns
///
/// Result containing the signer as `Arc<dyn ExternalSigner>`
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn default_external_signer(
    mnemonic: String,
    passphrase: Option<String>,
    network: Network,
    key_set_config: Option<crate::models::KeySetConfig>,
) -> Result<Arc<dyn crate::signer::ExternalSigner>, SdkError> {
    use crate::signer::DefaultExternalSigner;

    let config = key_set_config.unwrap_or_default();
    let signer = DefaultExternalSigner::new(
        mnemonic,
        passphrase,
        network,
        config.key_set_type,
        config.use_address_index,
        config.account_number,
    )?;

    Ok(Arc::new(signer))
}

pub(crate) struct SdkServicesParams {
    pub config: Config,
    pub storage: Arc<dyn Storage>,
    pub chain_service: Arc<dyn BitcoinChainService>,
    pub fiat_service: Arc<dyn FiatService>,
    pub lnurl_client: Arc<dyn RestClient>,
    pub lnurl_server_client: Option<Arc<dyn LnurlServerClient>>,
    pub lnurl_auth_signer: Arc<crate::signer::lnurl_auth::LnurlAuthSignerAdapter>,
    pub shutdown_sender: watch::Sender<()>,
    pub spark_wallet: Arc<SparkWallet>,
    pub event_emitter: Arc<EventEmitter>,
    pub nostr_client: Arc<NostrClient>,
    pub flashnet_client: Arc<FlashnetClient>,
}

impl BreezSdk {
    /// Creates a new instance of the `BreezSdk`
    pub(crate) fn init_and_start(sp: SdkServicesParams) -> Result<Self, SdkError> {
        // In Regtest we allow running without a Breez API key to facilitate local
        // integration tests. For non-regtest networks, a valid API key is required.
        if !matches!(sp.config.network, Network::Regtest) {
            match &sp.config.api_key {
                Some(api_key) => validate_breez_api_key(api_key)?,
                None => return Err(SdkError::Generic("Missing Breez API key".to_string())),
            }
        }
        let (initial_synced_sender, initial_synced_watcher) = watch::channel(false);
        let external_input_parsers = sp.config.get_all_external_input_parsers();

        let services = Arc::new(SdkServices {
            config: sp.config,
            spark_wallet: sp.spark_wallet,
            storage: sp.storage,
            chain_service: sp.chain_service,
            fiat_service: sp.fiat_service,
            lnurl_client: sp.lnurl_client,
            lnurl_server_client: sp.lnurl_server_client,
            lnurl_auth_signer: sp.lnurl_auth_signer,
            event_emitter: sp.event_emitter,
            shutdown_sender: sp.shutdown_sender,
            sync_trigger: tokio::sync::broadcast::channel(10).0,
            zap_receipt_trigger: tokio::sync::broadcast::channel(10).0,
            conversion_refund_trigger: tokio::sync::broadcast::channel(10).0,
            initial_synced_watcher,
            external_input_parsers,
            spark_private_mode_initialized: Arc::new(OnceCell::new()),
            nostr_client: sp.nostr_client,
            flashnet_client: sp.flashnet_client,
        });

        let sdk = Self { services };
        sdk.start(initial_synced_sender);
        Ok(sdk)
    }

    /// Starts the SDK's background tasks
    ///
    /// This method initiates the following backround tasks:
    /// 1. `spawn_spark_private_mode_initialization`: initializes the spark private mode on startup
    /// 2. `periodic_sync`: syncs the wallet with the Spark network    
    /// 3. `try_recover_lightning_address`: recovers the lightning address on startup
    /// 4. `spawn_zap_receipt_publisher`: publishes zap receipts for payments with zap requests
    /// 5. `spawm_conversion_refunder`: refunds failed conversions
    fn start(&self, initial_synced_sender: watch::Sender<bool>) {
        self.spawn_spark_private_mode_initialization();
        self.periodic_sync(initial_synced_sender);
        self.try_recover_lightning_address();
        self.spawn_zap_receipt_publisher();
        self.spawn_conversion_refunder();
    }

    fn spawn_spark_private_mode_initialization(&self) {
        let sdk = self.clone();
        tokio::spawn(async move {
            if let Err(e) = sdk.ensure_spark_private_mode_initialized().await {
                error!("Failed to initialize spark private mode: {e:?}");
            }
        });
    }

    /// Refreshes the user's lightning address on the server on startup.
    fn try_recover_lightning_address(&self) {
        let sdk = self.clone();
        tokio::spawn(async move {
            if sdk.services.config.lnurl_domain.is_none() {
                return;
            }

            match sdk.recover_lightning_address().await {
                Ok(None) => info!("no lightning address to recover on startup"),
                Ok(Some(value)) => info!(
                    "recovered lightning address on startup: lnurl: {}, address: {}",
                    value.lnurl, value.lightning_address
                ),
                Err(e) => error!("Failed to recover lightning address on startup: {e:?}"),
            }
        });
    }

    /// Background task that publishes zap receipts for payments with zap requests.
    /// Triggered on startup and after syncing lnurl metadata.
    fn spawn_zap_receipt_publisher(&self) {
        let sdk = self.clone();
        let mut shutdown_receiver = sdk.services.shutdown_sender.subscribe();
        let mut trigger_receiver = sdk.services.zap_receipt_trigger.clone().subscribe();

        tokio::spawn(async move {
            if let Err(e) = Self::process_pending_zap_receipts(&sdk).await {
                error!("Failed to process pending zap receipts on startup: {e:?}");
            }

            loop {
                tokio::select! {
                    _ = shutdown_receiver.changed() => {
                        info!("Zap receipt publisher shutdown signal received");
                        return;
                    }
                    _ = trigger_receiver.recv() => {
                        if let Err(e) = Self::process_pending_zap_receipts(&sdk).await {
                            error!("Failed to process pending zap receipts: {e:?}");
                        }
                    }
                }
            }
        });
    }

    /// Background task that periodically checks for failed conversions and refunds them.
    /// Triggered on startup and then every 150 seconds.
    fn spawn_conversion_refunder(&self) {
        let sdk = self.clone();
        let mut shutdown_receiver = sdk.services.shutdown_sender.subscribe();
        let mut trigger_receiver = sdk.services.conversion_refund_trigger.clone().subscribe();

        tokio::spawn(async move {
            loop {
                if let Err(e) = sdk.refund_failed_conversions().await {
                    error!("Failed to refund failed conversions: {e:?}");
                }

                select! {
                    _ = shutdown_receiver.changed() => {
                        info!("Conversion refunder shutdown signal received");
                        return;
                    }
                    _ = trigger_receiver.recv() => {
                        debug!("Conversion refunder triggered");
                    }
                    () = tokio::time::sleep(Duration::from_secs(150)) => {}
                }
            }
        });
    }

    async fn process_pending_zap_receipts(&self) -> Result<(), SdkError> {
        let Some(lnurl_server_client) = self.services.lnurl_server_client.clone() else {
            return Ok(());
        };

        let mut offset = 0;
        let limit = 100;
        loop {
            let payments = self
                .services
                .storage
                .list_payments(ListPaymentsRequest {
                    offset: Some(offset),
                    limit: Some(limit),
                    status_filter: Some(vec![PaymentStatus::Completed]),
                    type_filter: Some(vec![PaymentType::Receive]),
                    asset_filter: Some(AssetFilter::Bitcoin),
                    ..Default::default()
                })
                .await?;
            if payments.is_empty() {
                break;
            }

            let len = u32::try_from(payments.len())?;
            for payment in payments {
                let Some(PaymentDetails::Lightning {
                    ref lnurl_receive_metadata,
                    ref payment_hash,
                    ..
                }) = payment.details
                else {
                    continue;
                };

                let Some(lnurl_receive_metadata) = lnurl_receive_metadata else {
                    continue;
                };

                let Some(zap_request) = &lnurl_receive_metadata.nostr_zap_request else {
                    continue;
                };

                if lnurl_receive_metadata.nostr_zap_receipt.is_some() {
                    continue;
                }

                // Create the zap receipt using NostrClient
                let zap_receipt = match self
                    .services
                    .nostr_client
                    .create_zap_receipt(zap_request, &payment)
                    .await
                {
                    Ok(receipt) => receipt,
                    Err(e) => {
                        error!(
                            "Failed to create zap receipt for payment {}: {e:?}",
                            payment.id
                        );
                        continue;
                    }
                };

                // Publish the zap receipt via the server
                let zap_receipt = match lnurl_server_client
                    .publish_zap_receipt(&PublishZapReceiptRequest {
                        payment_hash: payment_hash.clone(),
                        zap_receipt: zap_receipt.clone(),
                    })
                    .await
                {
                    Ok(zap_receipt) => zap_receipt,
                    Err(e) => {
                        error!(
                            "Failed to publish zap receipt for payment {}: {}",
                            payment.id, e
                        );
                        continue;
                    }
                };

                if let Err(e) = self
                    .services
                    .storage
                    .set_lnurl_metadata(vec![SetLnurlMetadataItem {
                        sender_comment: lnurl_receive_metadata.sender_comment.clone(),
                        nostr_zap_request: Some(zap_request.clone()),
                        nostr_zap_receipt: Some(zap_receipt),
                        payment_hash: payment_hash.clone(),
                    }])
                    .await
                {
                    error!(
                        "Failed to store zap receipt for payment {}: {}",
                        payment.id, e
                    );
                }
            }

            if len < limit {
                break;
            }

            offset = offset.saturating_add(len);
        }

        Ok(())
    }

    fn periodic_sync(&self, initial_synced_sender: watch::Sender<bool>) {
        let sdk = self.clone();
        let mut shutdown_receiver = sdk.services.shutdown_sender.subscribe();
        let mut subscription = sdk.services.spark_wallet.subscribe_events();
        let sync_trigger_sender = sdk.services.sync_trigger.clone();
        let mut sync_trigger_receiver = sdk.services.sync_trigger.clone().subscribe();
        let mut last_sync_time = SystemTime::now();
        let sync_interval = u64::from(self.services.config.sync_interval_secs);
        tokio::spawn(async move {
            let balance_watcher = BalanceWatcher::new(
                sdk.services.spark_wallet.clone(),
                sdk.services.storage.clone(),
            );
            let balance_watcher_id = sdk.add_event_listener(Box::new(balance_watcher)).await;
            loop {
                tokio::select! {
                    _ = shutdown_receiver.changed() => {
                        if !sdk.remove_event_listener(&balance_watcher_id).await {
                            error!("Failed to remove balance watcher listener");
                        }
                        info!("Deposit tracking loop shutdown signal received");
                        return;
                    }
                    event = subscription.recv() => {
                        match event {
                            Ok(event) => {
                                info!("Received event: {event}");
                                trace!("Received event: {:?}", event);
                                sdk.handle_wallet_event(event).await;
                            }
                            Err(e) => {
                                error!("Failed to receive event: {e:?}");
                            }
                        }
                    }
                    sync_type_res = sync_trigger_receiver.recv() => {
                        let Ok(sync_request) = sync_type_res else {
                            continue;
                        };
                        info!("Sync trigger changed: {:?}", &sync_request);
                        let cloned_sdk = sdk.clone();
                        let initial_synced_sender = initial_synced_sender.clone();
                        if let Some(true) = Box::pin(run_with_shutdown(shutdown_receiver.clone(), "Sync trigger changed", async move {
                            if let Err(e) = cloned_sdk.sync_wallet_internal(sync_request.sync_type.clone()).await {
                                error!("Failed to sync wallet: {e:?}");
                                let () = sync_request.reply(Some(e)).await;
                                return false;
                            }
                            // Notify that the requested sync is complete
                            let () = sync_request.reply(None).await;
                            // If this was a full sync, notify the initial synced watcher
                            if sync_request.sync_type.contains(SyncType::Full) {
                                if let Err(e) = initial_synced_sender.send(true) {
                                    error!("Failed to send initial synced signal: {e:?}");
                                }
                                return true;
                            }

                            false
                        })).await {
                            last_sync_time = SystemTime::now();
                        }
                    }
                    // Ensure we sync at least the configured interval
                    () = tokio::time::sleep(Duration::from_secs(10)) => {
                        let now = SystemTime::now();
                        if let Ok(elapsed) = now.duration_since(last_sync_time) && elapsed.as_secs() >= sync_interval
                            && let Err(e) = sync_trigger_sender.send(SyncRequest::full(None)) {
                            error!("Failed to trigger periodic sync: {e:?}");
                        }
                    }
                }
            }
        });
    }

    async fn handle_wallet_event(&self, event: WalletEvent) {
        match event {
            WalletEvent::DepositConfirmed(_) => {
                info!("Deposit confirmed");
            }
            WalletEvent::StreamConnected => {
                info!("Stream connected");
            }
            WalletEvent::StreamDisconnected => {
                info!("Stream disconnected");
            }
            WalletEvent::Synced => {
                info!("Synced");
                if let Err(e) = self.services.sync_trigger.send(SyncRequest::full(None)) {
                    error!("Failed to sync wallet: {e:?}");
                }
            }
            WalletEvent::TransferClaimed(transfer) => {
                info!("Transfer claimed");
                if let Ok(mut payment) = Payment::try_from(transfer) {
                    // Insert the payment into storage to make it immediately available for listing
                    if let Err(e) = self.services.storage.insert_payment(payment.clone()).await {
                        error!("Failed to insert succeeded payment: {e:?}");
                    }

                    // Ensure potential lnurl metadata is synced before emitting the event.
                    // Note this is already synced at TransferClaimStarting, but it might not have completed yet, so that could race.
                    self.sync_single_lnurl_metadata(&mut payment).await;

                    self.services
                        .event_emitter
                        .emit(&SdkEvent::PaymentSucceeded { payment })
                        .await;
                }
                if let Err(e) = self
                    .services
                    .sync_trigger
                    .send(SyncRequest::no_reply(SyncType::WalletState))
                {
                    error!("Failed to sync wallet: {e:?}");
                }
            }
            WalletEvent::TransferClaimStarting(transfer) => {
                info!("Transfer claim starting");
                if let Ok(mut payment) = Payment::try_from(transfer) {
                    // Insert the payment into storage to make it immediately available for listing
                    if let Err(e) = self.services.storage.insert_payment(payment.clone()).await {
                        error!("Failed to insert pending payment: {e:?}");
                    }

                    // Ensure potential lnurl metadata is synced before emitting the event
                    self.sync_single_lnurl_metadata(&mut payment).await;

                    self.services
                        .event_emitter
                        .emit(&SdkEvent::PaymentPending { payment })
                        .await;
                }
                if let Err(e) = self
                    .services
                    .sync_trigger
                    .send(SyncRequest::no_reply(SyncType::WalletState))
                {
                    error!("Failed to sync wallet: {e:?}");
                }
            }
            WalletEvent::Optimization(event) => {
                info!("Optimization event: {:?}", event);
            }
        }
    }

    async fn sync_single_lnurl_metadata(&self, payment: &mut Payment) {
        if payment.payment_type != PaymentType::Receive {
            return;
        }

        let Some(PaymentDetails::Lightning {
            invoice,
            lnurl_receive_metadata,
            ..
        }) = &mut payment.details
        else {
            return;
        };

        if lnurl_receive_metadata.is_some() {
            // Already have lnurl metadata
            return;
        }

        let Ok(input) = parse_input(invoice, None).await else {
            error!(
                "Failed to parse invoice for lnurl metadata sync: {}",
                invoice
            );
            return;
        };

        let InputType::Bolt11Invoice(details) = input else {
            error!(
                "Input is not a Bolt11 invoice for lnurl metadata sync: {}",
                invoice
            );
            return;
        };

        // If there is a description hash, we assume this is a lnurl payment.
        if details.description_hash.is_none() {
            return;
        }

        // Let's check whether the lnurl receive metadata was already synced, then return early
        if let Ok(db_payment) = self
            .services
            .storage
            .get_payment_by_id(payment.id.clone())
            .await
            && let Some(PaymentDetails::Lightning {
                lnurl_receive_metadata: db_lnurl_receive_metadata,
                ..
            }) = db_payment.details
        {
            *lnurl_receive_metadata = db_lnurl_receive_metadata;
            return;
        }

        // Just sync all lnurl metadata here, no need to be picky.
        let (tx, rx) = oneshot::channel();
        if let Err(e) = self
            .services
            .sync_trigger
            .send(SyncRequest::new(tx, SyncType::LnurlMetadata))
        {
            error!("Failed to trigger lnurl metadata sync: {e}");
            return;
        }

        if let Err(e) = rx.await {
            error!("Failed to sync lnurl metadata for invoice {}: {e}", invoice);
            return;
        }

        let db_payment = match self
            .services
            .storage
            .get_payment_by_id(payment.id.clone())
            .await
        {
            Ok(p) => p,
            Err(e) => {
                debug!("Payment not found in storage for invoice {}: {e}", invoice);
                return;
            }
        };

        let Some(PaymentDetails::Lightning {
            lnurl_receive_metadata: db_lnurl_receive_metadata,
            ..
        }) = db_payment.details
        else {
            debug!(
                "No lnurl receive metadata in storage for invoice {}",
                invoice
            );
            return;
        };
        *lnurl_receive_metadata = db_lnurl_receive_metadata;
    }

    #[allow(clippy::too_many_lines)]
    async fn sync_wallet_internal(&self, sync_type: SyncType) -> Result<(), SdkError> {
        let start_time = Instant::now();

        let sync_wallet = async {
            let wallet_synced = if sync_type.contains(SyncType::Wallet) {
                debug!("sync_wallet_internal: Starting Wallet sync");
                let wallet_start = Instant::now();
                match self.services.spark_wallet.sync().await {
                    Ok(()) => {
                        debug!(
                            "sync_wallet_internal: Wallet sync completed in {:?}",
                            wallet_start.elapsed()
                        );
                        true
                    }
                    Err(e) => {
                        error!(
                            "sync_wallet_internal: Spark wallet sync failed in {:?}: {e:?}",
                            wallet_start.elapsed()
                        );
                        false
                    }
                }
            } else {
                trace!("sync_wallet_internal: Skipping Wallet sync");
                false
            };

            let wallet_state_synced = if sync_type.contains(SyncType::WalletState) {
                debug!("sync_wallet_internal: Starting WalletState sync");
                let wallet_state_start = Instant::now();
                match self.sync_wallet_state_to_storage().await {
                    Ok(()) => {
                        debug!(
                            "sync_wallet_internal: WalletState sync completed in {:?}",
                            wallet_state_start.elapsed()
                        );
                        true
                    }
                    Err(e) => {
                        error!(
                            "sync_wallet_internal: Failed to sync wallet state to storage in {:?}: {e:?}",
                            wallet_state_start.elapsed()
                        );
                        false
                    }
                }
            } else {
                trace!("sync_wallet_internal: Skipping WalletState sync");
                false
            };

            (wallet_synced, wallet_state_synced)
        };

        let sync_lnurl = async {
            if sync_type.contains(SyncType::LnurlMetadata) {
                debug!("sync_wallet_internal: Starting LnurlMetadata sync");
                let lnurl_start = Instant::now();
                match self.sync_lnurl_metadata().await {
                    Ok(()) => {
                        debug!(
                            "sync_wallet_internal: LnurlMetadata sync completed in {:?}",
                            lnurl_start.elapsed()
                        );
                        true
                    }
                    Err(e) => {
                        error!(
                            "sync_wallet_internal: Failed to sync lnurl metadata in {:?}: {e:?}",
                            lnurl_start.elapsed()
                        );
                        false
                    }
                }
            } else {
                trace!("sync_wallet_internal: Skipping LnurlMetadata sync");
                false
            }
        };

        let sync_deposits = async {
            if sync_type.contains(SyncType::Deposits) {
                debug!("sync_wallet_internal: Starting Deposits sync");
                let deposits_start = Instant::now();
                match self.check_and_claim_static_deposits().await {
                    Ok(()) => {
                        debug!(
                            "sync_wallet_internal: Deposits sync completed in {:?}",
                            deposits_start.elapsed()
                        );
                        true
                    }
                    Err(e) => {
                        error!(
                            "sync_wallet_internal: Failed to check and claim static deposits in {:?}: {e:?}",
                            deposits_start.elapsed()
                        );
                        false
                    }
                }
            } else {
                trace!("sync_wallet_internal: Skipping Deposits sync");
                false
            }
        };

        let ((wallet, wallet_state), lnurl_metadata, deposits) =
            tokio::join!(sync_wallet, sync_lnurl, sync_deposits);

        let elapsed = start_time.elapsed();
        let event = InternalSyncedEvent {
            wallet,
            wallet_state,
            lnurl_metadata,
            deposits,
            storage_incoming: None,
        };
        info!("sync_wallet_internal: Wallet sync completed in {elapsed:?}: {event:?}");
        self.services.event_emitter.emit_synced(&event).await;
        Ok(())
    }

    /// Synchronizes wallet state to persistent storage, making sure we have the latest balances and payments.
    async fn sync_wallet_state_to_storage(&self) -> Result<(), SdkError> {
        update_balances(
            self.services.spark_wallet.clone(),
            self.services.storage.clone(),
        )
        .await?;

        let initial_sync_complete = *self.services.initial_synced_watcher.borrow();
        let sync_service = SparkSyncService::new(
            self.services.spark_wallet.clone(),
            self.services.storage.clone(),
            self.services.event_emitter.clone(),
        );
        sync_service.sync_payments(initial_sync_complete).await?;

        Ok(())
    }

    async fn check_and_claim_static_deposits(&self) -> Result<(), SdkError> {
        self.ensure_spark_private_mode_initialized().await?;
        let to_claim = DepositChainSyncer::new(
            self.services.chain_service.clone(),
            self.services.storage.clone(),
            self.services.spark_wallet.clone(),
        )
        .sync()
        .await?;

        let mut claimed_deposits: Vec<DepositInfo> = Vec::new();
        let mut unclaimed_deposits: Vec<DepositInfo> = Vec::new();
        for detailed_utxo in to_claim {
            match self
                .claim_utxo(
                    &detailed_utxo,
                    self.services.config.max_deposit_claim_fee.clone(),
                )
                .await
            {
                Ok(_) => {
                    info!("Claimed utxo {}:{}", detailed_utxo.txid, detailed_utxo.vout);
                    self.services
                        .storage
                        .delete_deposit(detailed_utxo.txid.to_string(), detailed_utxo.vout)
                        .await?;
                    claimed_deposits.push(detailed_utxo.into());
                }
                Err(e) => {
                    warn!(
                        "Failed to claim utxo {}:{}: {e}",
                        detailed_utxo.txid, detailed_utxo.vout
                    );
                    self.services
                        .storage
                        .update_deposit(
                            detailed_utxo.txid.to_string(),
                            detailed_utxo.vout,
                            UpdateDepositPayload::ClaimError {
                                error: e.clone().into(),
                            },
                        )
                        .await?;
                    let mut unclaimed_deposit: DepositInfo = detailed_utxo.clone().into();
                    unclaimed_deposit.claim_error = Some(e.into());
                    unclaimed_deposits.push(unclaimed_deposit);
                }
            }
        }

        info!("background claim completed, unclaimed deposits: {unclaimed_deposits:?}");

        if !unclaimed_deposits.is_empty() {
            self.services
                .event_emitter
                .emit(&SdkEvent::UnclaimedDeposits { unclaimed_deposits })
                .await;
        }
        if !claimed_deposits.is_empty() {
            self.services
                .event_emitter
                .emit(&SdkEvent::ClaimedDeposits { claimed_deposits })
                .await;
        }
        Ok(())
    }

    async fn sync_lnurl_metadata(&self) -> Result<(), SdkError> {
        let Some(lnurl_server_client) = self.services.lnurl_server_client.clone() else {
            return Ok(());
        };

        let cache = ObjectCacheRepository::new(Arc::clone(&self.services.storage));
        let mut updated_after = cache.fetch_lnurl_metadata_updated_after().await?;

        loop {
            debug!("Syncing lnurl metadata from updated_after {updated_after}");
            let metadata = lnurl_server_client
                .list_metadata(&ListMetadataRequest {
                    offset: None,
                    limit: Some(SYNC_PAGING_LIMIT),
                    updated_after: Some(updated_after),
                })
                .await?;

            if metadata.metadata.is_empty() {
                debug!("No more lnurl metadata on offset {updated_after}");
                break;
            }

            let len = u32::try_from(metadata.metadata.len())?;
            let last_updated_at = metadata.metadata.last().map(|m| m.updated_at);
            self.services
                .storage
                .set_lnurl_metadata(metadata.metadata.into_iter().map(From::from).collect())
                .await?;

            debug!(
                "Synchronized {} lnurl metadata at updated_after {updated_after}",
                len
            );
            updated_after = last_updated_at.unwrap_or(updated_after);
            cache
                .save_lnurl_metadata_updated_after(updated_after)
                .await?;

            let _ = self.services.zap_receipt_trigger.send(());
            if len < SYNC_PAGING_LIMIT {
                // No more invoices to fetch
                break;
            }
        }

        Ok(())
    }

    /// Checks for payments that need conversion refunds and initiates the manual refund process.
    /// This occurs when a Spark transfer or token transaction is sent using the Flashnet client,
    /// but the execution fails and no automatic refund is initiated.
    async fn refund_failed_conversions(&self) -> Result<(), SdkError> {
        debug!("Checking for failed conversions needing refunds");
        let payments = self
            .services
            .storage
            .list_payments(ListPaymentsRequest {
                payment_details_filter: Some(vec![
                    PaymentDetailsFilter::Spark {
                        htlc_status: None,
                        conversion_refund_needed: Some(true),
                    },
                    PaymentDetailsFilter::Token {
                        conversion_refund_needed: Some(true),
                        tx_hash: None,
                    },
                ]),
                ..Default::default()
            })
            .await?;
        debug!(
            "Found {} payments needing conversion refunds",
            payments.len()
        );
        for payment in payments {
            if let Err(e) = self.refund_conversion(&payment).await {
                error!(
                    "Failed to refund conversion for payment {}: {e:?}",
                    payment.id
                );
            }
        }

        Ok(())
    }

    /// Initiates a refund for a conversion payment that requires a manual refund.
    async fn refund_conversion(&self, payment: &Payment) -> Result<(), SdkError> {
        let (clawback_id, conversion_info) = match &payment.details {
            Some(PaymentDetails::Spark {
                conversion_info, ..
            }) => (payment.id.clone(), conversion_info),
            Some(PaymentDetails::Token {
                tx_hash,
                conversion_info,
                ..
            }) => (tx_hash.clone(), conversion_info),
            _ => {
                return Err(SdkError::Generic(
                    "Payment is not a Spark or Conversion".to_string(),
                ));
            }
        };
        let Some(ConversionInfo {
            pool_id,
            conversion_id,
            status: ConversionStatus::RefundNeeded,
            fee,
            purpose,
        }) = conversion_info
        else {
            return Err(SdkError::Generic(
                "Conversion does not have a refund pending status".to_string(),
            ));
        };
        debug!(
            "Conversion refund needed for payment {}: pool_id {pool_id}, conversion_id {conversion_id}",
            payment.id
        );
        let Ok(pool_id) = PublicKey::from_str(pool_id) else {
            return Err(SdkError::Generic(format!("Invalid pool_id: {pool_id}")));
        };
        match self
            .services
            .flashnet_client
            .clawback(ClawbackRequest {
                pool_id,
                transfer_id: clawback_id,
            })
            .await
        {
            Ok(ClawbackResponse {
                accepted: true,
                spark_status_tracking_id,
                ..
            }) => {
                debug!(
                    "Clawback initiated for payment {}: tracking_id: {}",
                    payment.id, spark_status_tracking_id
                );
                // Update the payment metadata to reflect the refund status
                self.merge_payment_metadata(
                    payment.id.clone(),
                    PaymentMetadata {
                        conversion_info: Some(ConversionInfo {
                            pool_id: pool_id.to_string(),
                            conversion_id: conversion_id.clone(),
                            status: ConversionStatus::Refunded,
                            fee: *fee,
                            purpose: purpose.clone(),
                        }),
                        ..Default::default()
                    },
                )
                .await?;
                // Add payment metadata for the not yet received refund payment
                let cache = ObjectCacheRepository::new(self.services.storage.clone());
                cache
                    .save_payment_metadata(
                        &spark_status_tracking_id,
                        &PaymentMetadata {
                            conversion_info: Some(ConversionInfo {
                                pool_id: pool_id.to_string(),
                                conversion_id: conversion_id.clone(),
                                status: ConversionStatus::Refunded,
                                fee: Some(0),
                                purpose: None,
                            }),
                            ..Default::default()
                        },
                    )
                    .await?;
                Ok(())
            }
            Ok(ClawbackResponse {
                accepted: false,
                request_id,
                error,
                ..
            }) => Err(SdkError::Generic(format!(
                "Clawback not accepted: request_id: {request_id:?}, error: {error:?}"
            ))),
            Err(e) => Err(SdkError::Generic(format!(
                "Failed to initiate clawback: {e}"
            ))),
        }
    }

    async fn claim_utxo(
        &self,
        detailed_utxo: &DetailedUtxo,
        max_claim_fee: Option<MaxFee>,
    ) -> Result<WalletTransfer, SdkError> {
        info!(
            "Fetching static deposit claim quote for deposit tx {}:{} and amount: {}",
            detailed_utxo.txid, detailed_utxo.vout, detailed_utxo.value
        );
        let quote = self
            .services
            .spark_wallet
            .fetch_static_deposit_claim_quote(detailed_utxo.tx.clone(), Some(detailed_utxo.vout))
            .await?;

        let spark_requested_fee_sats = detailed_utxo.value.saturating_sub(quote.credit_amount_sats);

        let spark_requested_fee_rate = spark_requested_fee_sats.div_ceil(CLAIM_TX_SIZE_VBYTES);

        let Some(max_deposit_claim_fee) = max_claim_fee else {
            return Err(SdkError::MaxDepositClaimFeeExceeded {
                tx: detailed_utxo.txid.to_string(),
                vout: detailed_utxo.vout,
                max_fee: None,
                required_fee_sats: spark_requested_fee_sats,
                required_fee_rate_sat_per_vbyte: spark_requested_fee_rate,
            });
        };
        let max_fee = max_deposit_claim_fee
            .to_fee(self.services.chain_service.as_ref())
            .await?;
        let max_fee_sats = max_fee.to_sats(CLAIM_TX_SIZE_VBYTES);
        info!(
            "User max fee: {} spark requested fee: {}",
            max_fee_sats, spark_requested_fee_sats
        );
        if spark_requested_fee_sats > max_fee_sats {
            return Err(SdkError::MaxDepositClaimFeeExceeded {
                tx: detailed_utxo.txid.to_string(),
                vout: detailed_utxo.vout,
                max_fee: Some(max_fee),
                required_fee_sats: spark_requested_fee_sats,
                required_fee_rate_sat_per_vbyte: spark_requested_fee_rate,
            });
        }

        info!(
            "Claiming static deposit for utxo {}:{}",
            detailed_utxo.txid, detailed_utxo.vout
        );
        let transfer = self
            .services
            .spark_wallet
            .claim_static_deposit(quote)
            .await?;
        info!(
            "Claimed static deposit transfer for utxo {}:{}, value {}",
            detailed_utxo.txid, detailed_utxo.vout, transfer.total_value_sat,
        );
        Ok(transfer)
    }

    async fn ensure_spark_private_mode_initialized(&self) -> Result<(), SdkError> {
        self.services
            .spark_private_mode_initialized
            .get_or_try_init(|| async {
                // Check if already initialized in storage
                let object_repository = ObjectCacheRepository::new(self.services.storage.clone());
                let is_initialized = object_repository
                    .fetch_spark_private_mode_initialized()
                    .await?;

                if !is_initialized {
                    // Initialize if not already done
                    self.initialize_spark_private_mode().await?;
                }
                Ok::<_, SdkError>(())
            })
            .await?;
        Ok(())
    }

    async fn initialize_spark_private_mode(&self) -> Result<(), SdkError> {
        if !self.services.config.private_enabled_default {
            ObjectCacheRepository::new(self.services.storage.clone())
                .save_spark_private_mode_initialized()
                .await?;
            info!("Spark private mode initialized: no changes needed");
            return Ok(());
        }

        // Enable spark private mode
        self.update_user_settings(UpdateUserSettingsRequest {
            spark_private_mode_enabled: Some(true),
        })
        .await?;
        ObjectCacheRepository::new(self.services.storage.clone())
            .save_spark_private_mode_initialized()
            .await?;
        info!("Spark private mode initialized: enabled");
        Ok(())
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
#[allow(clippy::needless_pass_by_value)]
impl BreezSdk {
    /// Registers a listener to receive SDK events
    ///
    /// # Arguments
    ///
    /// * `listener` - An implementation of the `EventListener` trait
    ///
    /// # Returns
    ///
    /// A unique identifier for the listener, which can be used to remove it later
    pub async fn add_event_listener(&self, listener: Box<dyn EventListener>) -> String {
        self.services.event_emitter.add_listener(listener).await
    }

    /// Removes a previously registered event listener
    ///
    /// # Arguments
    ///
    /// * `id` - The listener ID returned from `add_event_listener`
    ///
    /// # Returns
    ///
    /// `true` if the listener was found and removed, `false` otherwise
    pub async fn remove_event_listener(&self, id: &str) -> bool {
        self.services.event_emitter.remove_listener(id).await
    }

    /// Stops the SDK's background tasks
    ///
    /// This method stops the background tasks started by the `start()` method.
    /// It should be called before your application terminates to ensure proper cleanup.
    ///
    /// # Returns
    ///
    /// Result containing either success or an `SdkError` if the background task couldn't be stopped
    pub async fn disconnect(&self) -> Result<(), SdkError> {
        info!("Disconnecting Breez SDK");
        self.services
            .shutdown_sender
            .send(())
            .map_err(|_| SdkError::Generic("Failed to send shutdown signal".to_string()))?;

        self.services.shutdown_sender.closed().await;
        info!("Breez SDK disconnected");
        Ok(())
    }

    pub async fn parse(&self, input: &str) -> Result<InputType, SdkError> {
        parse_input(input, Some(self.services.external_input_parsers.clone())).await
    }

    /// Returns the balance of the wallet in satoshis
    #[allow(unused_variables)]
    pub async fn get_info(&self, request: GetInfoRequest) -> Result<GetInfoResponse, SdkError> {
        if request.ensure_synced.unwrap_or_default() {
            self.services
                .initial_synced_watcher
                .clone()
                .changed()
                .await
                .map_err(|_| {
                    SdkError::Generic("Failed to receive initial synced signal".to_string())
                })?;
        }
        let object_repository = ObjectCacheRepository::new(self.services.storage.clone());
        let account_info = object_repository
            .fetch_account_info()
            .await?
            .unwrap_or_default();
        Ok(GetInfoResponse {
            balance_sats: account_info.balance_sats,
            token_balances: account_info.token_balances,
        })
    }

    pub async fn receive_payment(
        &self,
        request: ReceivePaymentRequest,
    ) -> Result<ReceivePaymentResponse, SdkError> {
        self.ensure_spark_private_mode_initialized().await?;
        match request.payment_method {
            ReceivePaymentMethod::SparkAddress => Ok(ReceivePaymentResponse {
                fee: 0,
                payment_request: self
                    .services
                    .spark_wallet
                    .get_spark_address()?
                    .to_address_string()
                    .map_err(|e| {
                        SdkError::Generic(format!("Failed to convert Spark address to string: {e}"))
                    })?,
            }),
            ReceivePaymentMethod::SparkInvoice {
                amount,
                token_identifier,
                expiry_time,
                description,
                sender_public_key,
            } => {
                let invoice = self
                    .services
                    .spark_wallet
                    .create_spark_invoice(
                        amount,
                        token_identifier.clone(),
                        expiry_time
                            .map(|time| {
                                SystemTime::UNIX_EPOCH
                                    .checked_add(Duration::from_secs(time))
                                    .ok_or(SdkError::Generic("Invalid expiry time".to_string()))
                            })
                            .transpose()?,
                        description,
                        sender_public_key.map(|key| PublicKey::from_str(&key).unwrap()),
                    )
                    .await?;
                Ok(ReceivePaymentResponse {
                    fee: 0,
                    payment_request: invoice,
                })
            }
            ReceivePaymentMethod::BitcoinAddress => {
                // TODO: allow passing amount

                let object_repository = ObjectCacheRepository::new(self.services.storage.clone());

                // First lookup in storage cache
                let static_deposit_address =
                    object_repository.fetch_static_deposit_address().await?;
                if let Some(static_deposit_address) = static_deposit_address {
                    return Ok(ReceivePaymentResponse {
                        payment_request: static_deposit_address.address.clone(),
                        fee: 0,
                    });
                }

                // Then query existing addresses
                let deposit_addresses = self
                    .services
                    .spark_wallet
                    .list_static_deposit_addresses(None)
                    .await?;

                // In case there are no addresses, generate a new one and cache it
                let address = match deposit_addresses.items.last() {
                    Some(address) => address.to_string(),
                    None => self
                        .services
                        .spark_wallet
                        .generate_deposit_address(true)
                        .await?
                        .to_string(),
                };

                object_repository
                    .save_static_deposit_address(&StaticDepositAddress {
                        address: address.clone(),
                    })
                    .await?;

                Ok(ReceivePaymentResponse {
                    payment_request: address,
                    fee: 0,
                })
            }
            ReceivePaymentMethod::Bolt11Invoice {
                description,
                amount_sats,
                expiry_secs,
            } => Ok(ReceivePaymentResponse {
                payment_request: self
                    .services
                    .spark_wallet
                    .create_lightning_invoice(
                        amount_sats.unwrap_or_default(),
                        Some(InvoiceDescription::Memo(description.clone())),
                        None,
                        expiry_secs,
                        self.services.config.prefer_spark_over_lightning,
                    )
                    .await?
                    .invoice,
                fee: 0,
            }),
        }
    }

    pub async fn claim_htlc_payment(
        &self,
        request: ClaimHtlcPaymentRequest,
    ) -> Result<ClaimHtlcPaymentResponse, SdkError> {
        let preimage = Preimage::from_hex(&request.preimage)
            .map_err(|_| SdkError::InvalidInput("Invalid preimage".to_string()))?;
        let payment_hash = preimage.compute_hash();

        // Check if there is a claimable HTLC with the given payment hash
        let claimable_htlc_transfers = self
            .services
            .spark_wallet
            .list_claimable_htlc_transfers(None)
            .await?;
        if !claimable_htlc_transfers
            .iter()
            .filter_map(|t| t.htlc_preimage_request.as_ref())
            .any(|p| p.payment_hash == payment_hash)
        {
            return Err(SdkError::InvalidInput(
                "No claimable HTLC with the given payment hash".to_string(),
            ));
        }

        let transfer = self.services.spark_wallet.claim_htlc(&preimage).await?;
        let payment: Payment = transfer.try_into()?;

        // Insert the payment into storage to make it immediately available for listing
        self.services
            .storage
            .insert_payment(payment.clone())
            .await?;

        Ok(ClaimHtlcPaymentResponse { payment })
    }

    pub async fn prepare_lnurl_pay(
        &self,
        request: PrepareLnurlPayRequest,
    ) -> Result<PrepareLnurlPayResponse, SdkError> {
        let success_data = match validate_lnurl_pay(
            self.services.lnurl_client.as_ref(),
            request.amount_sats.saturating_mul(1_000),
            &None,
            &request.pay_request.clone().into(),
            self.services.config.network.into(),
            request.validate_success_action_url,
        )
        .await?
        {
            lnurl::pay::ValidatedCallbackResponse::EndpointError { data } => {
                return Err(LnurlError::EndpointError(data.reason).into());
            }
            lnurl::pay::ValidatedCallbackResponse::EndpointSuccess { data } => data,
        };

        let prepare_response = self
            .prepare_send_payment(PrepareSendPaymentRequest {
                payment_request: success_data.pr,
                amount: Some(request.amount_sats.into()),
                token_identifier: None,
                conversion_options: None,
            })
            .await?;

        let SendPaymentMethod::Bolt11Invoice {
            invoice_details,
            lightning_fee_sats,
            ..
        } = prepare_response.payment_method
        else {
            return Err(SdkError::Generic(
                "Expected Bolt11Invoice payment method".to_string(),
            ));
        };

        Ok(PrepareLnurlPayResponse {
            amount_sats: request.amount_sats,
            comment: request.comment,
            pay_request: request.pay_request,
            invoice_details,
            fee_sats: lightning_fee_sats,
            success_action: success_data.success_action.map(From::from),
        })
    }

    pub async fn lnurl_pay(&self, request: LnurlPayRequest) -> Result<LnurlPayResponse, SdkError> {
        self.ensure_spark_private_mode_initialized().await?;
        let mut payment = Box::pin(self.maybe_convert_token_send_payment(
            SendPaymentRequest {
                prepare_response: PrepareSendPaymentResponse {
                    payment_method: SendPaymentMethod::Bolt11Invoice {
                        invoice_details: request.prepare_response.invoice_details,
                        spark_transfer_fee_sats: None,
                        lightning_fee_sats: request.prepare_response.fee_sats,
                    },
                    amount: request.prepare_response.amount_sats.into(),
                    token_identifier: None,
                    conversion_estimate: None,
                },
                options: None,
                idempotency_key: request.idempotency_key,
            },
            true,
        ))
        .await?
        .payment;

        let success_action = process_success_action(
            &payment,
            request
                .prepare_response
                .success_action
                .clone()
                .map(Into::into)
                .as_ref(),
        )?;

        let lnurl_info = LnurlPayInfo {
            ln_address: request.prepare_response.pay_request.address,
            comment: request.prepare_response.comment,
            domain: Some(request.prepare_response.pay_request.domain),
            metadata: Some(request.prepare_response.pay_request.metadata_str),
            processed_success_action: success_action.clone().map(From::from),
            raw_success_action: request.prepare_response.success_action,
        };
        let Some(PaymentDetails::Lightning {
            lnurl_pay_info,
            description,
            ..
        }) = &mut payment.details
        else {
            return Err(SdkError::Generic(
                "Expected Lightning payment details".to_string(),
            ));
        };
        *lnurl_pay_info = Some(lnurl_info.clone());

        let lnurl_description = lnurl_info.extract_description();
        description.clone_from(&lnurl_description);

        self.services
            .storage
            .set_payment_metadata(
                payment.id.clone(),
                PaymentMetadata {
                    lnurl_pay_info: Some(lnurl_info),
                    lnurl_description,
                    ..Default::default()
                },
            )
            .await?;

        self.services
            .event_emitter
            .emit(&SdkEvent::from_payment(payment.clone()))
            .await;
        Ok(LnurlPayResponse {
            payment,
            success_action: success_action.map(From::from),
        })
    }

    /// Performs an LNURL withdraw operation for the amount of satoshis to
    /// withdraw and the LNURL withdraw request details. The LNURL withdraw request
    /// details can be obtained from calling [`BreezSdk::parse`].
    ///
    /// The method generates a Lightning invoice for the withdraw amount, stores
    /// the LNURL withdraw metadata, and performs the LNURL withdraw using  the generated
    /// invoice.
    ///
    /// If the `completion_timeout_secs` parameter is provided and greater than 0, the
    /// method will wait for the payment to be completed within that period. If the
    /// withdraw is completed within the timeout, the `payment` field in the response
    /// will be set with the payment details. If the `completion_timeout_secs`
    /// parameter is not provided or set to 0, the method will not wait for the payment
    /// to be completed. If the withdraw is not completed within the
    /// timeout, the `payment` field will be empty.
    ///
    /// # Arguments
    ///
    /// * `request` - The LNURL withdraw request
    ///
    /// # Returns
    ///
    /// Result containing either:
    /// * `LnurlWithdrawResponse` - The payment details if the withdraw request was successful
    /// * `SdkError` - If there was an error during the withdraw process
    pub async fn lnurl_withdraw(
        &self,
        request: LnurlWithdrawRequest,
    ) -> Result<LnurlWithdrawResponse, SdkError> {
        self.ensure_spark_private_mode_initialized().await?;
        let LnurlWithdrawRequest {
            amount_sats,
            withdraw_request,
            completion_timeout_secs,
        } = request;
        let withdraw_request: breez_sdk_common::lnurl::withdraw::LnurlWithdrawRequestDetails =
            withdraw_request.into();
        if !withdraw_request.is_amount_valid(amount_sats) {
            return Err(SdkError::InvalidInput(
                "Amount must be within min/max LNURL withdrawable limits".to_string(),
            ));
        }

        // Generate a Lightning invoice for the withdraw
        let payment_request = self
            .receive_payment(ReceivePaymentRequest {
                payment_method: ReceivePaymentMethod::Bolt11Invoice {
                    description: withdraw_request.default_description.clone(),
                    amount_sats: Some(amount_sats),
                    expiry_secs: None,
                },
            })
            .await?
            .payment_request;

        // Store the LNURL withdraw metadata before executing the withdraw
        let cache = ObjectCacheRepository::new(self.services.storage.clone());
        cache
            .save_payment_metadata(
                &payment_request,
                &PaymentMetadata {
                    lnurl_withdraw_info: Some(LnurlWithdrawInfo {
                        withdraw_url: withdraw_request.callback.clone(),
                    }),
                    lnurl_description: Some(withdraw_request.default_description.clone()),
                    ..Default::default()
                },
            )
            .await?;

        // Perform the LNURL withdraw using the generated invoice
        let withdraw_response = execute_lnurl_withdraw(
            self.services.lnurl_client.as_ref(),
            &withdraw_request,
            &payment_request,
        )
        .await?;
        if let lnurl::withdraw::ValidatedCallbackResponse::EndpointError { data } =
            withdraw_response
        {
            return Err(LnurlError::EndpointError(data.reason).into());
        }

        let completion_timeout_secs = match completion_timeout_secs {
            Some(secs) if secs > 0 => secs,
            _ => {
                return Ok(LnurlWithdrawResponse {
                    payment_request,
                    payment: None,
                });
            }
        };

        // Wait for the payment to be completed
        let payment = self
            .wait_for_payment(
                WaitForPaymentIdentifier::PaymentRequest(payment_request.clone()),
                completion_timeout_secs,
            )
            .await
            .ok();
        Ok(LnurlWithdrawResponse {
            payment_request,
            payment,
        })
    }

    /// Performs LNURL-auth with the service.
    ///
    /// This method implements the LNURL-auth protocol as specified in LUD-04 and LUD-05.
    /// It derives a domain-specific linking key, signs the challenge, and sends the
    /// authentication request to the service.
    ///
    /// # Arguments
    ///
    /// * `request_data` - The parsed LNURL-auth request details obtained from [`parse`]
    ///
    /// # Returns
    ///
    /// * `Ok(LnurlCallbackStatus::Ok)` - Authentication was successful
    /// * `Ok(LnurlCallbackStatus::ErrorStatus{reason})` - Service returned an error
    /// * `Err(SdkError)` - An error occurred during the authentication process
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use breez_sdk_spark::{BreezSdk, InputType};
    /// # async fn example(sdk: BreezSdk) -> Result<(), Box<dyn std::error::Error>> {
    /// // 1. Parse the LNURL-auth string
    /// let input = sdk.parse("lnurl1...").await?;
    /// let auth_request = match input {
    ///     InputType::LnurlAuth(data) => data,
    ///     _ => return Err("Not an auth request".into()),
    /// };
    ///
    /// // 2. Show user the domain and get confirmation
    /// println!("Authenticate with {}?", auth_request.domain);
    ///
    /// // 3. Perform authentication
    /// let status = sdk.lnurl_auth(auth_request).await?;
    /// match status {
    ///     breez_sdk_spark::LnurlCallbackStatus::Ok => println!("Success!"),
    ///     breez_sdk_spark::LnurlCallbackStatus::ErrorStatus { error_details } => {
    ///         println!("Error: {}", error_details.reason)
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # See Also
    ///
    /// * LUD-04: <https://github.com/lnurl/luds/blob/luds/04.md>
    /// * LUD-05: <https://github.com/lnurl/luds/blob/luds/05.md>
    pub async fn lnurl_auth(
        &self,
        request_data: LnurlAuthRequestDetails,
    ) -> Result<LnurlCallbackStatus, SdkError> {
        let request: breez_sdk_common::lnurl::auth::LnurlAuthRequestDetails = request_data.into();
        let status = breez_sdk_common::lnurl::auth::perform_lnurl_auth(
            self.services.lnurl_client.as_ref(),
            &request,
            self.services.lnurl_auth_signer.as_ref(),
        )
        .await
        .map_err(|e| match e {
            LnurlError::ServiceConnectivity(msg) => SdkError::NetworkError(msg.to_string()),
            LnurlError::InvalidUri(msg) => SdkError::InvalidInput(msg),
            _ => SdkError::Generic(e.to_string()),
        })?;
        Ok(status.into())
    }

    #[allow(clippy::too_many_lines)]
    pub async fn prepare_send_payment(
        &self,
        request: PrepareSendPaymentRequest,
    ) -> Result<PrepareSendPaymentResponse, SdkError> {
        let parsed_input = self.parse(&request.payment_request).await?;

        validate_prepare_send_payment_request(
            &parsed_input,
            &request,
            &self
                .services
                .spark_wallet
                .get_identity_public_key()
                .to_string(),
        )?;

        match &parsed_input {
            InputType::SparkAddress(spark_address_details) => {
                let amount = request
                    .amount
                    .ok_or(SdkError::InvalidInput("Amount is required".to_string()))?;
                let conversion_estimate = self
                    .estimate_conversion(
                        request.conversion_options.as_ref(),
                        request.token_identifier.as_ref(),
                        amount,
                    )
                    .await?;

                Ok(PrepareSendPaymentResponse {
                    payment_method: SendPaymentMethod::SparkAddress {
                        address: spark_address_details.address.clone(),
                        fee: 0,
                        token_identifier: request.token_identifier.clone(),
                    },
                    amount,
                    token_identifier: request.token_identifier,
                    conversion_estimate,
                })
            }
            InputType::SparkInvoice(spark_invoice_details) => {
                let amount = spark_invoice_details
                    .amount
                    .or(request.amount)
                    .ok_or(SdkError::InvalidInput("Amount is required".to_string()))?;
                let conversion_estimate = self
                    .estimate_conversion(
                        request.conversion_options.as_ref(),
                        request.token_identifier.as_ref(),
                        amount,
                    )
                    .await?;

                Ok(PrepareSendPaymentResponse {
                    payment_method: SendPaymentMethod::SparkInvoice {
                        spark_invoice_details: spark_invoice_details.clone(),
                        fee: 0,
                        token_identifier: request.token_identifier.clone(),
                    },
                    amount,
                    token_identifier: request.token_identifier,
                    conversion_estimate,
                })
            }
            InputType::Bolt11Invoice(detailed_bolt11_invoice) => {
                let spark_address: Option<SparkAddress> = self
                    .services
                    .spark_wallet
                    .extract_spark_address(&request.payment_request)?;

                let spark_transfer_fee_sats = if spark_address.is_some() {
                    Some(0)
                } else {
                    None
                };

                let amount = request
                    .amount
                    .or(detailed_bolt11_invoice
                        .amount_msat
                        .map(|msat| u128::from(msat).saturating_div(1000)))
                    .ok_or(SdkError::InvalidInput("Amount is required".to_string()))?;
                let lightning_fee_sats = self
                    .services
                    .spark_wallet
                    .fetch_lightning_send_fee_estimate(
                        &request.payment_request,
                        request
                            .amount
                            .map(|a| Ok::<u64, SdkError>(a.try_into()?))
                            .transpose()?,
                    )
                    .await?;
                let conversion_estimate = self
                    .estimate_conversion(
                        request.conversion_options.as_ref(),
                        request.token_identifier.as_ref(),
                        amount.saturating_add(u128::from(lightning_fee_sats)),
                    )
                    .await?;

                Ok(PrepareSendPaymentResponse {
                    payment_method: SendPaymentMethod::Bolt11Invoice {
                        invoice_details: detailed_bolt11_invoice.clone(),
                        spark_transfer_fee_sats,
                        lightning_fee_sats,
                    },
                    amount,
                    token_identifier: request.token_identifier,
                    conversion_estimate,
                })
            }
            InputType::BitcoinAddress(withdrawal_address) => {
                let amount = request
                    .amount
                    .ok_or(SdkError::InvalidInput("Amount is required".to_string()))?;
                let fee_quote: SendOnchainFeeQuote = self
                    .services
                    .spark_wallet
                    .fetch_coop_exit_fee_quote(
                        &withdrawal_address.address,
                        Some(amount.try_into()?),
                    )
                    .await?
                    .into();
                let conversion_estimate = self
                    .estimate_conversion(
                        request.conversion_options.as_ref(),
                        request.token_identifier.as_ref(),
                        amount.saturating_add(u128::from(fee_quote.speed_fast.total_fee_sat())),
                    )
                    .await?;
                Ok(PrepareSendPaymentResponse {
                    payment_method: SendPaymentMethod::BitcoinAddress {
                        address: withdrawal_address.clone(),
                        fee_quote,
                    },
                    amount,
                    token_identifier: None,
                    conversion_estimate,
                })
            }
            _ => Err(SdkError::InvalidInput(
                "Unsupported payment method".to_string(),
            )),
        }
    }

    pub async fn send_payment(
        &self,
        request: SendPaymentRequest,
    ) -> Result<SendPaymentResponse, SdkError> {
        self.ensure_spark_private_mode_initialized().await?;
        Box::pin(self.maybe_convert_token_send_payment(request, false)).await
    }

    pub async fn fetch_conversion_limits(
        &self,
        request: FetchConversionLimitsRequest,
    ) -> Result<FetchConversionLimitsResponse, SdkError> {
        let (asset_in_address, asset_out_address) = request
            .conversion_type
            .as_asset_addresses(request.token_identifier.as_ref())?;
        let min_amounts = self
            .services
            .flashnet_client
            .get_min_amounts(GetMinAmountsRequest {
                asset_in_address,
                asset_out_address,
            })
            .await?;
        Ok(FetchConversionLimitsResponse {
            min_from_amount: min_amounts.asset_in_min,
            min_to_amount: min_amounts.asset_out_min,
        })
    }

    /// Synchronizes the wallet with the Spark network
    #[allow(unused_variables)]
    pub async fn sync_wallet(
        &self,
        request: SyncWalletRequest,
    ) -> Result<SyncWalletResponse, SdkError> {
        let (tx, rx) = oneshot::channel();

        if let Err(e) = self.services.sync_trigger.send(SyncRequest::full(Some(tx))) {
            error!("Failed to send sync trigger: {e:?}");
        }
        let _ = rx.await.map_err(|e| {
            error!("Failed to receive sync trigger: {e:?}");
            SdkError::Generic(format!("sync trigger failed: {e:?}"))
        })?;
        Ok(SyncWalletResponse {})
    }

    /// Lists payments from the storage with pagination
    ///
    /// This method provides direct access to the payment history stored in the database.
    /// It returns payments in reverse chronological order (newest first).
    ///
    /// # Arguments
    ///
    /// * `request` - Contains pagination parameters (offset and limit)
    ///
    /// # Returns
    ///
    /// * `Ok(ListPaymentsResponse)` - Contains the list of payments if successful
    /// * `Err(SdkError)` - If there was an error accessing the storage
    ///
    pub async fn list_payments(
        &self,
        request: ListPaymentsRequest,
    ) -> Result<ListPaymentsResponse, SdkError> {
        let payments = self.services.storage.list_payments(request).await?;
        Ok(ListPaymentsResponse { payments })
    }

    pub async fn get_payment(
        &self,
        request: GetPaymentRequest,
    ) -> Result<GetPaymentResponse, SdkError> {
        let payment = self
            .services
            .storage
            .get_payment_by_id(request.payment_id)
            .await?;
        Ok(GetPaymentResponse { payment })
    }

    pub async fn claim_deposit(
        &self,
        request: ClaimDepositRequest,
    ) -> Result<ClaimDepositResponse, SdkError> {
        self.ensure_spark_private_mode_initialized().await?;
        let detailed_utxo = CachedUtxoFetcher::new(
            self.services.chain_service.clone(),
            self.services.storage.clone(),
        )
        .fetch_detailed_utxo(&request.txid, request.vout)
        .await?;

        let max_fee = request
            .max_fee
            .or(self.services.config.max_deposit_claim_fee.clone());
        match self.claim_utxo(&detailed_utxo, max_fee).await {
            Ok(transfer) => {
                self.services
                    .storage
                    .delete_deposit(detailed_utxo.txid.to_string(), detailed_utxo.vout)
                    .await?;
                if let Err(e) = self
                    .services
                    .sync_trigger
                    .send(SyncRequest::no_reply(SyncType::WalletState))
                {
                    error!("Failed to execute sync after deposit claim: {e:?}");
                }
                Ok(ClaimDepositResponse {
                    payment: transfer.try_into()?,
                })
            }
            Err(e) => {
                error!("Failed to claim deposit: {e:?}");
                self.services
                    .storage
                    .update_deposit(
                        detailed_utxo.txid.to_string(),
                        detailed_utxo.vout,
                        UpdateDepositPayload::ClaimError {
                            error: e.clone().into(),
                        },
                    )
                    .await?;
                Err(e)
            }
        }
    }

    pub async fn refund_deposit(
        &self,
        request: RefundDepositRequest,
    ) -> Result<RefundDepositResponse, SdkError> {
        let detailed_utxo = CachedUtxoFetcher::new(
            self.services.chain_service.clone(),
            self.services.storage.clone(),
        )
        .fetch_detailed_utxo(&request.txid, request.vout)
        .await?;
        let tx = self
            .services
            .spark_wallet
            .refund_static_deposit(
                detailed_utxo.clone().tx,
                Some(detailed_utxo.vout),
                &request.destination_address,
                request.fee.into(),
            )
            .await?;
        let deposit: DepositInfo = detailed_utxo.into();
        let tx_hex = serialize(&tx).as_hex().to_string();
        let tx_id = tx.compute_txid().as_raw_hash().to_string();

        // Store the refund transaction details separately
        self.services
            .storage
            .update_deposit(
                deposit.txid.clone(),
                deposit.vout,
                UpdateDepositPayload::Refund {
                    refund_tx: tx_hex.clone(),
                    refund_txid: tx_id.clone(),
                },
            )
            .await?;

        self.services
            .chain_service
            .broadcast_transaction(tx_hex.clone())
            .await?;
        Ok(RefundDepositResponse { tx_id, tx_hex })
    }

    #[allow(unused_variables)]
    pub async fn list_unclaimed_deposits(
        &self,
        request: ListUnclaimedDepositsRequest,
    ) -> Result<ListUnclaimedDepositsResponse, SdkError> {
        let deposits = self.services.storage.list_deposits().await?;
        Ok(ListUnclaimedDepositsResponse { deposits })
    }

    pub async fn check_lightning_address_available(
        &self,
        req: CheckLightningAddressRequest,
    ) -> Result<bool, SdkError> {
        let Some(client) = &self.services.lnurl_server_client else {
            return Err(SdkError::Generic(
                "LNURL server is not configured".to_string(),
            ));
        };

        let username = sanitize_username(&req.username);
        let available = client.check_username_available(&username).await?;
        Ok(available)
    }

    pub async fn get_lightning_address(&self) -> Result<Option<LightningAddressInfo>, SdkError> {
        let cache = ObjectCacheRepository::new(self.services.storage.clone());
        Ok(cache.fetch_lightning_address().await?)
    }

    pub async fn register_lightning_address(
        &self,
        request: RegisterLightningAddressRequest,
    ) -> Result<LightningAddressInfo, SdkError> {
        // Ensure spark private mode is initialized before registering
        self.ensure_spark_private_mode_initialized().await?;

        self.register_lightning_address_internal(request).await
    }

    pub async fn delete_lightning_address(&self) -> Result<(), SdkError> {
        let cache = ObjectCacheRepository::new(self.services.storage.clone());
        let Some(address_info) = cache.fetch_lightning_address().await? else {
            return Ok(());
        };

        let Some(client) = &self.services.lnurl_server_client else {
            return Err(SdkError::Generic(
                "LNURL server is not configured".to_string(),
            ));
        };

        let params = crate::lnurl::UnregisterLightningAddressRequest {
            username: address_info.username,
        };

        client.unregister_lightning_address(&params).await?;
        cache.delete_lightning_address().await?;
        Ok(())
    }

    /// List fiat currencies for which there is a known exchange rate,
    /// sorted by the canonical name of the currency.
    pub async fn list_fiat_currencies(&self) -> Result<ListFiatCurrenciesResponse, SdkError> {
        let currencies = self
            .services
            .fiat_service
            .fetch_fiat_currencies()
            .await?
            .into_iter()
            .map(From::from)
            .collect();
        Ok(ListFiatCurrenciesResponse { currencies })
    }

    /// List the latest rates of fiat currencies, sorted by name.
    pub async fn list_fiat_rates(&self) -> Result<ListFiatRatesResponse, SdkError> {
        let rates = self
            .services
            .fiat_service
            .fetch_fiat_rates()
            .await?
            .into_iter()
            .map(From::from)
            .collect();
        Ok(ListFiatRatesResponse { rates })
    }

    /// Get the recommended BTC fees based on the configured chain service.
    pub async fn recommended_fees(&self) -> Result<RecommendedFees, SdkError> {
        Ok(self.services.chain_service.recommended_fees().await?)
    }

    /// Returns the metadata for the given token identifiers.
    ///
    /// Results are not guaranteed to be in the same order as the input token identifiers.    
    ///
    /// If the metadata is not found locally in cache, it will be queried from
    /// the Spark network and then cached.
    pub async fn get_tokens_metadata(
        &self,
        request: GetTokensMetadataRequest,
    ) -> Result<GetTokensMetadataResponse, SdkError> {
        let metadata = get_tokens_metadata_cached_or_query(
            &self.services.spark_wallet,
            &ObjectCacheRepository::new(self.services.storage.clone()),
            &request
                .token_identifiers
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
        )
        .await?;
        Ok(GetTokensMetadataResponse {
            tokens_metadata: metadata,
        })
    }

    /// Signs a message with the wallet's identity key. The message is SHA256
    /// hashed before signing. The returned signature will be hex encoded in
    /// DER format by default, or compact format if specified.
    pub async fn sign_message(
        &self,
        request: SignMessageRequest,
    ) -> Result<SignMessageResponse, SdkError> {
        let pubkey = self
            .services
            .spark_wallet
            .get_identity_public_key()
            .to_string();
        let signature = self
            .services
            .spark_wallet
            .sign_message(&request.message)
            .await?;
        let signature_hex = if request.compact {
            signature.serialize_compact().to_lower_hex_string()
        } else {
            signature.serialize_der().to_lower_hex_string()
        };

        Ok(SignMessageResponse {
            pubkey,
            signature: signature_hex,
        })
    }

    /// Verifies a message signature against the provided public key. The message
    /// is SHA256 hashed before verification. The signature can be hex encoded
    /// in either DER or compact format.
    pub async fn check_message(
        &self,
        request: CheckMessageRequest,
    ) -> Result<CheckMessageResponse, SdkError> {
        let pubkey = PublicKey::from_str(&request.pubkey)
            .map_err(|_| SdkError::InvalidInput("Invalid public key".to_string()))?;
        let signature_bytes = hex::decode(&request.signature)
            .map_err(|_| SdkError::InvalidInput("Not a valid hex encoded signature".to_string()))?;
        let signature = Signature::from_der(&signature_bytes)
            .or_else(|_| Signature::from_compact(&signature_bytes))
            .map_err(|_| {
                SdkError::InvalidInput("Not a valid DER or compact encoded signature".to_string())
            })?;

        let is_valid = self
            .services
            .spark_wallet
            .verify_message(&request.message, &signature, &pubkey)
            .await
            .is_ok();
        Ok(CheckMessageResponse { is_valid })
    }

    /// Returns the user settings for the wallet.
    ///
    /// Some settings are fetched from the Spark network so network requests are performed.
    pub async fn get_user_settings(&self) -> Result<UserSettings, SdkError> {
        // Ensure spark private mode is initialized to avoid race conditions with the initialization task.
        self.ensure_spark_private_mode_initialized().await?;

        let spark_user_settings = self.services.spark_wallet.query_wallet_settings().await?;

        // We may in the future have user settings that are stored locally and synced using real-time sync.

        Ok(UserSettings {
            spark_private_mode_enabled: spark_user_settings.private_enabled,
        })
    }

    /// Updates the user settings for the wallet.
    ///
    /// Some settings are updated on the Spark network so network requests may be performed.
    pub async fn update_user_settings(
        &self,
        request: UpdateUserSettingsRequest,
    ) -> Result<(), SdkError> {
        if let Some(spark_private_mode_enabled) = request.spark_private_mode_enabled {
            self.services
                .spark_wallet
                .update_wallet_settings(spark_private_mode_enabled)
                .await?;

            // Reregister the lightning address if spark private mode changed.
            let lightning_address = match self.get_lightning_address().await {
                Ok(lightning_address) => lightning_address,
                Err(e) => {
                    error!("Failed to get lightning address during user settings update: {e:?}");
                    return Ok(());
                }
            };
            let Some(lightning_address) = lightning_address else {
                return Ok(());
            };
            if let Err(e) = self
                .register_lightning_address_internal(RegisterLightningAddressRequest {
                    username: lightning_address.username,
                    description: Some(lightning_address.description),
                })
                .await
            {
                error!("Failed to reregister lightning address during user settings update: {e:?}");
            }
        }
        Ok(())
    }

    /// Returns an instance of the [`TokenIssuer`] for managing token issuance.
    pub fn get_token_issuer(&self) -> TokenIssuer {
        TokenIssuer::new(
            self.services.spark_wallet.clone(),
            self.services.storage.clone(),
        )
    }

    /// Starts leaf optimization in the background.
    ///
    /// This method spawns the optimization work in a background task and returns
    /// immediately. Progress is reported via events.
    /// If optimization is already running, no new task will be started.
    pub fn start_leaf_optimization(&self) {
        self.services.spark_wallet.start_leaf_optimization();
    }

    /// Cancels the ongoing leaf optimization.
    ///
    /// This method cancels the ongoing optimization and waits for it to fully stop.
    /// The current round will complete before stopping. This method blocks
    /// until the optimization has fully stopped and leaves reserved for optimization
    /// are available again.
    ///
    /// If no optimization is running, this method returns immediately.
    pub async fn cancel_leaf_optimization(&self) -> Result<(), SdkError> {
        self.services
            .spark_wallet
            .cancel_leaf_optimization()
            .await?;
        Ok(())
    }

    /// Returns the current optimization progress snapshot.
    pub fn get_leaf_optimization_progress(&self) -> OptimizationProgress {
        self.services
            .spark_wallet
            .get_leaf_optimization_progress()
            .into()
    }
}

// Separate impl block to avoid exposing private methods to uniffi.
impl BreezSdk {
    async fn maybe_convert_token_send_payment(
        &self,
        request: SendPaymentRequest,
        mut suppress_payment_event: bool,
    ) -> Result<SendPaymentResponse, SdkError> {
        // Check the idempotency key is valid and payment doesn't already exist
        if request.idempotency_key.is_some() && request.prepare_response.token_identifier.is_some()
        {
            return Err(SdkError::InvalidInput(
                "Idempotency key is not supported for token payments".to_string(),
            ));
        }
        if let Some(idempotency_key) = &request.idempotency_key {
            // If an idempotency key is provided, check if a payment with that id already exists
            if let Ok(payment) = self
                .services
                .storage
                .get_payment_by_id(idempotency_key.clone())
                .await
            {
                return Ok(SendPaymentResponse { payment });
            }
        }
        // Perform the send payment, with conversion if requested
        let res = if let Some(ConversionEstimate {
            options: conversion_options,
            ..
        }) = &request.prepare_response.conversion_estimate
        {
            Box::pin(self.convert_token_send_payment_internal(
                conversion_options,
                &request,
                &mut suppress_payment_event,
            ))
            .await
        } else {
            Box::pin(self.send_payment_internal(&request)).await
        };
        // Emit payment status event and trigger wallet state sync
        if let Ok(response) = &res {
            if !suppress_payment_event {
                self.services
                    .event_emitter
                    .emit(&SdkEvent::from_payment(response.payment.clone()))
                    .await;
            }
            if let Err(e) = self
                .services
                .sync_trigger
                .send(SyncRequest::no_reply(SyncType::WalletState))
            {
                error!("Failed to send sync trigger: {e:?}");
            }
        }
        res
    }

    #[allow(clippy::too_many_lines)]
    async fn convert_token_send_payment_internal(
        &self,
        conversion_options: &ConversionOptions,
        request: &SendPaymentRequest,
        suppress_payment_event: &mut bool,
    ) -> Result<SendPaymentResponse, SdkError> {
        // Perform a conversion before sending the payment
        let (conversion_response, conversion_purpose) =
            match &request.prepare_response.payment_method {
                SendPaymentMethod::SparkAddress { address, .. } => {
                    let spark_address = address
                        .parse::<SparkAddress>()
                        .map_err(|_| SdkError::InvalidInput("Invalid spark address".to_string()))?;
                    let conversion_purpose = if spark_address.identity_public_key
                        == self.services.spark_wallet.get_identity_public_key()
                    {
                        ConversionPurpose::SelfTransfer
                    } else {
                        ConversionPurpose::OngoingPayment {
                            payment_request: address.clone(),
                        }
                    };
                    let res = self
                        .convert_token(
                            conversion_options,
                            &conversion_purpose,
                            request.prepare_response.token_identifier.as_ref(),
                            request.prepare_response.amount,
                        )
                        .await?;
                    (res, conversion_purpose)
                }
                SendPaymentMethod::SparkInvoice {
                    spark_invoice_details:
                        SparkInvoiceDetails {
                            identity_public_key,
                            invoice,
                            ..
                        },
                    ..
                } => {
                    let own_identity_public_key = self
                        .services
                        .spark_wallet
                        .get_identity_public_key()
                        .to_string();
                    let conversion_purpose = if identity_public_key == &own_identity_public_key {
                        ConversionPurpose::SelfTransfer
                    } else {
                        ConversionPurpose::OngoingPayment {
                            payment_request: invoice.clone(),
                        }
                    };
                    let res = self
                        .convert_token(
                            conversion_options,
                            &conversion_purpose,
                            request.prepare_response.token_identifier.as_ref(),
                            request.prepare_response.amount,
                        )
                        .await?;
                    (res, conversion_purpose)
                }
                SendPaymentMethod::Bolt11Invoice {
                    spark_transfer_fee_sats,
                    lightning_fee_sats,
                    invoice_details,
                    ..
                } => {
                    let conversion_purpose = ConversionPurpose::OngoingPayment {
                        payment_request: invoice_details.invoice.bolt11.clone(),
                    };
                    let res = self
                        .convert_token_for_bolt11_invoice(
                            conversion_options,
                            *spark_transfer_fee_sats,
                            *lightning_fee_sats,
                            request,
                            &conversion_purpose,
                        )
                        .await?;
                    (res, conversion_purpose)
                }
                SendPaymentMethod::BitcoinAddress { address, fee_quote } => {
                    let conversion_purpose = ConversionPurpose::OngoingPayment {
                        payment_request: address.address.clone(),
                    };
                    let res = self
                        .convert_token_for_bitcoin_address(
                            conversion_options,
                            fee_quote,
                            request,
                            &conversion_purpose,
                        )
                        .await?;
                    (res, conversion_purpose)
                }
            };
        // Trigger a wallet state sync if converting from Bitcoin to token
        if matches!(
            conversion_options.conversion_type,
            ConversionType::FromBitcoin
        ) {
            let _ = self
                .services
                .sync_trigger
                .send(SyncRequest::no_reply(SyncType::WalletState));
        }
        // Wait for the received conversion payment to complete
        let payment = self
            .wait_for_payment(
                WaitForPaymentIdentifier::PaymentId(
                    conversion_response.received_payment_id.clone(),
                ),
                conversion_options
                    .completion_timeout_secs
                    .unwrap_or(DEFAULT_TOKEN_CONVERSION_TIMEOUT_SECS),
            )
            .await
            .map_err(|e| {
                SdkError::Generic(format!("Timeout waiting for conversion to complete: {e}"))
            })?;
        // For self-payments, we can skip sending the actual payment
        if conversion_purpose == ConversionPurpose::SelfTransfer {
            *suppress_payment_event = true;
            return Ok(SendPaymentResponse { payment });
        }
        // Now send the actual payment
        let response = Box::pin(self.send_payment_internal(request)).await?;
        // Merge payment metadata to link the payments
        self.merge_payment_metadata(
            conversion_response.sent_payment_id,
            PaymentMetadata {
                parent_payment_id: Some(response.payment.id.clone()),
                ..Default::default()
            },
        )
        .await?;
        self.merge_payment_metadata(
            conversion_response.received_payment_id,
            PaymentMetadata {
                parent_payment_id: Some(response.payment.id.clone()),
                ..Default::default()
            },
        )
        .await?;

        Ok(response)
    }

    async fn send_payment_internal(
        &self,
        request: &SendPaymentRequest,
    ) -> Result<SendPaymentResponse, SdkError> {
        match &request.prepare_response.payment_method {
            SendPaymentMethod::SparkAddress {
                address,
                token_identifier,
                ..
            } => {
                self.send_spark_address(
                    address,
                    token_identifier.clone(),
                    request.prepare_response.amount,
                    request.options.as_ref(),
                    request.idempotency_key.clone(),
                )
                .await
            }
            SendPaymentMethod::SparkInvoice {
                spark_invoice_details,
                ..
            } => {
                self.send_spark_invoice(&spark_invoice_details.invoice, request)
                    .await
            }
            SendPaymentMethod::Bolt11Invoice {
                invoice_details,
                spark_transfer_fee_sats,
                lightning_fee_sats,
                ..
            } => {
                Box::pin(self.send_bolt11_invoice(
                    invoice_details,
                    *spark_transfer_fee_sats,
                    *lightning_fee_sats,
                    request,
                ))
                .await
            }
            SendPaymentMethod::BitcoinAddress { address, fee_quote } => {
                self.send_bitcoin_address(address, fee_quote, request).await
            }
        }
    }

    async fn send_spark_address(
        &self,
        address: &str,
        token_identifier: Option<String>,
        amount: u128,
        options: Option<&SendPaymentOptions>,
        idempotency_key: Option<String>,
    ) -> Result<SendPaymentResponse, SdkError> {
        let spark_address = address
            .parse::<SparkAddress>()
            .map_err(|_| SdkError::InvalidInput("Invalid spark address".to_string()))?;

        // If HTLC options are provided, send an HTLC transfer
        if let Some(SendPaymentOptions::SparkAddress { htlc_options }) = options
            && let Some(htlc_options) = htlc_options
        {
            if token_identifier.is_some() {
                return Err(SdkError::InvalidInput(
                    "Can't provide both token identifier and HTLC options".to_string(),
                ));
            }

            return self
                .send_spark_htlc(
                    &spark_address,
                    amount.try_into()?,
                    htlc_options,
                    idempotency_key,
                )
                .await;
        }

        let payment = if let Some(identifier) = token_identifier {
            self.send_spark_token_address(identifier, amount, spark_address)
                .await?
        } else {
            let transfer_id = idempotency_key
                .as_ref()
                .map(|key| TransferId::from_str(key))
                .transpose()?;
            let transfer = self
                .services
                .spark_wallet
                .transfer(amount.try_into()?, &spark_address, transfer_id)
                .await?;
            transfer.try_into()?
        };

        // Insert the payment into storage to make it immediately available for listing
        self.services
            .storage
            .insert_payment(payment.clone())
            .await?;

        Ok(SendPaymentResponse { payment })
    }

    async fn send_spark_htlc(
        &self,
        address: &SparkAddress,
        amount_sat: u64,
        htlc_options: &SparkHtlcOptions,
        idempotency_key: Option<String>,
    ) -> Result<SendPaymentResponse, SdkError> {
        let payment_hash = sha256::Hash::from_str(&htlc_options.payment_hash)
            .map_err(|_| SdkError::InvalidInput("Invalid payment hash".to_string()))?;

        if htlc_options.expiry_duration_secs == 0 {
            return Err(SdkError::InvalidInput(
                "Expiry duration must be greater than 0".to_string(),
            ));
        }
        let expiry_duration = Duration::from_secs(htlc_options.expiry_duration_secs);

        let transfer_id = idempotency_key
            .as_ref()
            .map(|key| TransferId::from_str(key))
            .transpose()?;
        let transfer = self
            .services
            .spark_wallet
            .create_htlc(
                amount_sat,
                address,
                &payment_hash,
                expiry_duration,
                transfer_id,
            )
            .await?;

        let payment: Payment = transfer.try_into()?;

        // Insert the payment into storage to make it immediately available for listing
        self.services
            .storage
            .insert_payment(payment.clone())
            .await?;

        Ok(SendPaymentResponse { payment })
    }

    async fn send_spark_token_address(
        &self,
        token_identifier: String,
        amount: u128,
        receiver_address: SparkAddress,
    ) -> Result<Payment, SdkError> {
        let token_transaction = self
            .services
            .spark_wallet
            .transfer_tokens(
                vec![TransferTokenOutput {
                    token_id: token_identifier,
                    amount,
                    receiver_address: receiver_address.clone(),
                    spark_invoice: None,
                }],
                None,
                None,
            )
            .await?;

        map_and_persist_token_transaction(
            &self.services.spark_wallet,
            &self.services.storage,
            &token_transaction,
        )
        .await
    }

    async fn send_spark_invoice(
        &self,
        invoice: &str,
        request: &SendPaymentRequest,
    ) -> Result<SendPaymentResponse, SdkError> {
        let transfer_id = request
            .idempotency_key
            .as_ref()
            .map(|key| TransferId::from_str(key))
            .transpose()?;

        let payment = match self
            .services
            .spark_wallet
            .fulfill_spark_invoice(invoice, Some(request.prepare_response.amount), transfer_id)
            .await?
        {
            spark_wallet::FulfillSparkInvoiceResult::Transfer(wallet_transfer) => {
                (*wallet_transfer).try_into()?
            }
            spark_wallet::FulfillSparkInvoiceResult::TokenTransaction(token_transaction) => {
                map_and_persist_token_transaction(
                    &self.services.spark_wallet,
                    &self.services.storage,
                    &token_transaction,
                )
                .await?
            }
        };

        // Insert the payment into storage to make it immediately available for listing
        self.services
            .storage
            .insert_payment(payment.clone())
            .await?;

        Ok(SendPaymentResponse { payment })
    }

    async fn send_bolt11_invoice(
        &self,
        invoice_details: &Bolt11InvoiceDetails,
        spark_transfer_fee_sats: Option<u64>,
        lightning_fee_sats: u64,
        request: &SendPaymentRequest,
    ) -> Result<SendPaymentResponse, SdkError> {
        let amount_to_send = match invoice_details.amount_msat {
            // We are not sending amount in case the invoice contains it.
            Some(_) => None,
            // We are sending amount for zero amount invoice
            None => Some(request.prepare_response.amount),
        };
        let (prefer_spark, completion_timeout_secs) = match request.options {
            Some(SendPaymentOptions::Bolt11Invoice {
                prefer_spark,
                completion_timeout_secs,
            }) => (prefer_spark, completion_timeout_secs),
            _ => (self.services.config.prefer_spark_over_lightning, None),
        };
        let fee_sats = match (prefer_spark, spark_transfer_fee_sats, lightning_fee_sats) {
            (true, Some(fee), _) => fee,
            _ => lightning_fee_sats,
        };
        let transfer_id = request
            .idempotency_key
            .as_ref()
            .map(|idempotency_key| TransferId::from_str(idempotency_key))
            .transpose()?;

        let payment_response = self
            .services
            .spark_wallet
            .pay_lightning_invoice(
                &invoice_details.invoice.bolt11,
                amount_to_send
                    .map(|a| Ok::<u64, SdkError>(a.try_into()?))
                    .transpose()?,
                Some(fee_sats),
                prefer_spark,
                transfer_id,
            )
            .await?;
        let payment = match payment_response.lightning_payment {
            Some(lightning_payment) => {
                let ssp_id = lightning_payment.id.clone();
                let payment = Payment::from_lightning(
                    lightning_payment,
                    request.prepare_response.amount,
                    payment_response.transfer.id.to_string(),
                )?;
                self.poll_lightning_send_payment(&payment, ssp_id);
                payment
            }
            None => payment_response.transfer.try_into()?,
        };

        let Some(completion_timeout_secs) = completion_timeout_secs else {
            return Ok(SendPaymentResponse { payment });
        };

        if completion_timeout_secs == 0 {
            return Ok(SendPaymentResponse { payment });
        }

        let payment = self
            .wait_for_payment(
                WaitForPaymentIdentifier::PaymentId(payment.id.clone()),
                completion_timeout_secs,
            )
            .await
            .unwrap_or(payment);

        // Insert the payment into storage to make it immediately available for listing
        self.services
            .storage
            .insert_payment(payment.clone())
            .await?;

        Ok(SendPaymentResponse { payment })
    }

    async fn send_bitcoin_address(
        &self,
        address: &BitcoinAddressDetails,
        fee_quote: &SendOnchainFeeQuote,
        request: &SendPaymentRequest,
    ) -> Result<SendPaymentResponse, SdkError> {
        let exit_speed = match &request.options {
            Some(SendPaymentOptions::BitcoinAddress { confirmation_speed }) => {
                confirmation_speed.clone().into()
            }
            None => ExitSpeed::Fast,
            _ => {
                return Err(SdkError::InvalidInput("Invalid options".to_string()));
            }
        };
        let transfer_id = request
            .idempotency_key
            .as_ref()
            .map(|idempotency_key| TransferId::from_str(idempotency_key))
            .transpose()?;
        let response = self
            .services
            .spark_wallet
            .withdraw(
                &address.address,
                Some(request.prepare_response.amount.try_into()?),
                exit_speed,
                fee_quote.clone().into(),
                transfer_id,
            )
            .await?;

        let payment: Payment = response.try_into()?;

        self.services
            .storage
            .insert_payment(payment.clone())
            .await?;

        Ok(SendPaymentResponse { payment })
    }

    async fn wait_for_payment(
        &self,
        identifier: WaitForPaymentIdentifier,
        completion_timeout_secs: u32,
    ) -> Result<Payment, SdkError> {
        let (tx, mut rx) = mpsc::channel(20);
        let id = self
            .add_event_listener(Box::new(InternalEventListener::new(tx)))
            .await;

        // First check if we already have the completed payment in storage
        let payment = match &identifier {
            WaitForPaymentIdentifier::PaymentId(payment_id) => self
                .services
                .storage
                .get_payment_by_id(payment_id.clone())
                .await
                .ok(),
            WaitForPaymentIdentifier::PaymentRequest(payment_request) => {
                self.services
                    .storage
                    .get_payment_by_invoice(payment_request.clone())
                    .await?
            }
        };
        if let Some(payment) = payment
            && payment.status == PaymentStatus::Completed
        {
            self.remove_event_listener(&id).await;
            return Ok(payment);
        }

        let timeout_res = timeout(Duration::from_secs(completion_timeout_secs.into()), async {
            loop {
                let Some(event) = rx.recv().await else {
                    return Err(SdkError::Generic("Event channel closed".to_string()));
                };

                let SdkEvent::PaymentSucceeded { payment } = event else {
                    continue;
                };

                if is_payment_match(&payment, &identifier) {
                    return Ok(payment);
                }
            }
        })
        .await
        .map_err(|_| SdkError::Generic("Timeout waiting for payment".to_string()));

        self.remove_event_listener(&id).await;
        timeout_res?
    }

    async fn merge_payment_metadata(
        &self,
        payment_id: String,
        mut metadata: PaymentMetadata,
    ) -> Result<(), SdkError> {
        if let Some(details) = self
            .services
            .storage
            .get_payment_by_id(payment_id.clone())
            .await
            .ok()
            .and_then(|p| p.details)
        {
            match details {
                PaymentDetails::Lightning {
                    lnurl_pay_info,
                    lnurl_withdraw_info,
                    ..
                } => {
                    metadata.lnurl_pay_info = metadata.lnurl_pay_info.or(lnurl_pay_info);
                    metadata.lnurl_withdraw_info =
                        metadata.lnurl_withdraw_info.or(lnurl_withdraw_info);
                }
                PaymentDetails::Spark {
                    conversion_info, ..
                }
                | PaymentDetails::Token {
                    conversion_info, ..
                } => {
                    metadata.conversion_info = metadata.conversion_info.or(conversion_info);
                }
                _ => {}
            }
        }
        self.services
            .storage
            .set_payment_metadata(payment_id, metadata)
            .await?;
        Ok(())
    }

    // Pools the lightning send payment untill it is in completed state.
    fn poll_lightning_send_payment(&self, payment: &Payment, ssp_id: String) {
        const MAX_POLL_ATTEMPTS: u32 = 20;
        let payment_id = payment.id.clone();
        info!("Polling lightning send payment {}", payment_id);

        let spark_wallet = self.services.spark_wallet.clone();
        let sync_trigger = self.services.sync_trigger.clone();
        let event_emitter = self.services.event_emitter.clone();
        let payment = payment.clone();
        let payment_id = payment_id.clone();
        let mut shutdown = self.services.shutdown_sender.subscribe();

        tokio::spawn(async move {
            for i in 0..MAX_POLL_ATTEMPTS {
                info!(
                    "Polling lightning send payment {} attempt {}",
                    payment_id, i
                );
                select! {
                    _ = shutdown.changed() => {
                        info!("Shutdown signal received");
                        return;
                    },
                    p = spark_wallet.fetch_lightning_send_payment(&ssp_id) => {
                        if let Ok(Some(p)) = p && let Ok(payment) = Payment::from_lightning(p.clone(), payment.amount, payment.id.clone()) {
                            info!("Polling payment status = {} {:?}", payment.status, p.status);
                            if payment.status != PaymentStatus::Pending {
                                info!("Polling payment completed status = {}", payment.status);
                                event_emitter.emit(&SdkEvent::from_payment(payment.clone())).await;
                                if let Err(e) = sync_trigger.send(SyncRequest::no_reply(SyncType::WalletState)) {
                                    error!("Failed to send sync trigger: {e:?}");
                                }
                                return;
                            }
                        }

                        let sleep_time = if i < 5 {
                            Duration::from_secs(1)
                        } else {
                            Duration::from_secs(i.into())
                        };
                        tokio::time::sleep(sleep_time).await;
                    }
                }
            }
        });
    }

    /// Attempts to recover a lightning address from the lnurl server.
    async fn recover_lightning_address(&self) -> Result<Option<LightningAddressInfo>, SdkError> {
        let cache = ObjectCacheRepository::new(self.services.storage.clone());

        let Some(client) = &self.services.lnurl_server_client else {
            return Err(SdkError::Generic(
                "LNURL server is not configured".to_string(),
            ));
        };
        let resp = client.recover_lightning_address().await?;

        let result = if let Some(resp) = resp {
            let address_info = resp.into();
            cache.save_lightning_address(&address_info).await?;
            Some(address_info)
        } else {
            cache.delete_lightning_address().await?;
            None
        };

        Ok(result)
    }

    async fn register_lightning_address_internal(
        &self,
        request: RegisterLightningAddressRequest,
    ) -> Result<LightningAddressInfo, SdkError> {
        let cache = ObjectCacheRepository::new(self.services.storage.clone());
        let Some(client) = &self.services.lnurl_server_client else {
            return Err(SdkError::Generic(
                "LNURL server is not configured".to_string(),
            ));
        };

        let username = sanitize_username(&request.username);

        let description = match request.description {
            Some(description) => description,
            None => format!("Pay to {}@{}", username, client.domain()),
        };

        // Query settings directly from spark wallet to avoid recursion through get_user_settings()
        let spark_user_settings = self.services.spark_wallet.query_wallet_settings().await?;
        let nostr_pubkey = if spark_user_settings.private_enabled {
            Some(self.services.nostr_client.nostr_pubkey())
        } else {
            None
        };

        let params = crate::lnurl::RegisterLightningAddressRequest {
            username: username.clone(),
            description: description.clone(),
            nostr_pubkey,
        };

        let response = client.register_lightning_address(&params).await?;
        let address_info = LightningAddressInfo {
            lightning_address: response.lightning_address,
            description,
            lnurl: response.lnurl,
            username,
        };
        cache.save_lightning_address(&address_info).await?;
        Ok(address_info)
    }

    async fn convert_token_for_bolt11_invoice(
        &self,
        conversion_options: &ConversionOptions,
        spark_transfer_fee_sats: Option<u64>,
        lightning_fee_sats: u64,
        request: &SendPaymentRequest,
        conversion_purpose: &ConversionPurpose,
    ) -> Result<TokenConversionResponse, SdkError> {
        // Determine the fee to be used based on preference
        let fee_sats = match request.options {
            Some(SendPaymentOptions::Bolt11Invoice { prefer_spark, .. }) => {
                match (prefer_spark, spark_transfer_fee_sats) {
                    (true, Some(fee)) => fee,
                    _ => lightning_fee_sats,
                }
            }
            _ => lightning_fee_sats,
        };
        // The absolute minimum amount out is the lightning invoice amount plus fee
        let min_amount_out = request
            .prepare_response
            .amount
            .saturating_add(u128::from(fee_sats));

        self.convert_token(
            conversion_options,
            conversion_purpose,
            request.prepare_response.token_identifier.as_ref(),
            min_amount_out,
        )
        .await
    }

    async fn convert_token_for_bitcoin_address(
        &self,
        conversion_options: &ConversionOptions,
        fee_quote: &SendOnchainFeeQuote,
        request: &SendPaymentRequest,
        conversion_purpose: &ConversionPurpose,
    ) -> Result<TokenConversionResponse, SdkError> {
        // Determine the fee to be used based on confirmation speed
        let fee_sats = if let Some(SendPaymentOptions::BitcoinAddress { confirmation_speed }) =
            &request.options
        {
            match confirmation_speed {
                OnchainConfirmationSpeed::Slow => fee_quote.speed_slow.total_fee_sat(),
                OnchainConfirmationSpeed::Medium => fee_quote.speed_medium.total_fee_sat(),
                OnchainConfirmationSpeed::Fast => fee_quote.speed_fast.total_fee_sat(),
            }
        } else {
            fee_quote.speed_fast.total_fee_sat()
        };
        // The absolute minimum amount out is the amount plus fee
        let min_amount_out = request
            .prepare_response
            .amount
            .saturating_add(u128::from(fee_sats));

        self.convert_token(
            conversion_options,
            conversion_purpose,
            request.prepare_response.token_identifier.as_ref(),
            min_amount_out,
        )
        .await
    }

    #[allow(clippy::too_many_lines)]
    async fn convert_token(
        &self,
        conversion_options: &ConversionOptions,
        conversion_purpose: &ConversionPurpose,
        token_identifier: Option<&String>,
        min_amount_out: u128,
    ) -> Result<TokenConversionResponse, SdkError> {
        let conversion_pool = self
            .get_conversion_pool(conversion_options, token_identifier, min_amount_out)
            .await?;
        let conversion_estimate = self
            .estimate_conversion_internal(&conversion_pool, conversion_options, min_amount_out)
            .await?
            .ok_or(SdkError::Generic(
                "No conversion estimate available".to_string(),
            ))?;
        // Execute the conversion
        let pool_id = conversion_pool.pool.lp_public_key;
        let response_res = self
            .services
            .flashnet_client
            .execute_swap(ExecuteSwapRequest {
                asset_in_address: conversion_pool.asset_in_address.clone(),
                asset_out_address: conversion_pool.asset_out_address.clone(),
                pool_id,
                amount_in: conversion_estimate.amount,
                max_slippage_bps: conversion_options
                    .max_slippage_bps
                    .unwrap_or(DEFAULT_TOKEN_CONVERSION_MAX_SLIPPAGE_BPS),
                min_amount_out,
                integrator_fee_rate_bps: None,
                integrator_public_key: None,
            })
            .await;
        match response_res {
            Ok(response) => {
                info!(
                    "Conversion executed: accepted {}, error {:?}",
                    response.accepted, response.error
                );
                let (sent_payment_id, received_payment_id) = self
                    .update_payment_conversion_info(
                        &pool_id,
                        response.transfer_id,
                        response.outbound_transfer_id,
                        response.refund_transfer_id,
                        response.fee_amount,
                        conversion_purpose,
                    )
                    .await?;
                if let Some(received_payment_id) = received_payment_id
                    && response.accepted
                {
                    Ok(TokenConversionResponse {
                        sent_payment_id,
                        received_payment_id,
                    })
                } else {
                    let error_message = response
                        .error
                        .unwrap_or("Conversion not accepted".to_string());
                    Err(SdkError::Generic(format!(
                        "Convert token failed, refund in progress: {error_message}",
                    )))
                }
            }
            Err(e) => {
                error!("Convert token failed: {e:?}");
                if let FlashnetError::Execution {
                    transaction_identifier: Some(transaction_identifier),
                    source,
                } = &e
                {
                    let _ = self
                        .update_payment_conversion_info(
                            &pool_id,
                            transaction_identifier.clone(),
                            None,
                            None,
                            None,
                            conversion_purpose,
                        )
                        .await;
                    let _ = self.services.conversion_refund_trigger.send(());
                    Err(SdkError::Generic(format!(
                        "Convert token failed, refund pending: {}",
                        *source.clone()
                    )))
                } else {
                    Err(e.into())
                }
            }
        }
    }

    async fn get_conversion_pool(
        &self,
        conversion_options: &ConversionOptions,
        token_identifier: Option<&String>,
        amount_out: u128,
    ) -> Result<TokenConversionPool, SdkError> {
        let conversion_type = &conversion_options.conversion_type;
        let (asset_in_address, asset_out_address) =
            conversion_type.as_asset_addresses(token_identifier)?;

        // List available pools for the asset pair
        let a_in_pools_fut = self.services.flashnet_client.list_pools(ListPoolsRequest {
            asset_a_address: Some(asset_in_address.clone()),
            asset_b_address: Some(asset_out_address.clone()),
            sort: Some(PoolSortOrder::Volume24hDesc),
            ..Default::default()
        });
        let b_in_pools_fut = self.services.flashnet_client.list_pools(ListPoolsRequest {
            asset_a_address: Some(asset_out_address.clone()),
            asset_b_address: Some(asset_in_address.clone()),
            sort: Some(PoolSortOrder::Volume24hDesc),
            ..Default::default()
        });
        let (a_in_pools_res, b_in_pools_res) = tokio::join!(a_in_pools_fut, b_in_pools_fut);
        let mut pools = a_in_pools_res.map_or(HashMap::new(), |res| {
            res.pools
                .into_iter()
                .map(|pool| (pool.lp_public_key, pool))
                .collect::<HashMap<_, _>>()
        });
        if let Ok(res) = b_in_pools_res {
            pools.extend(res.pools.into_iter().map(|pool| (pool.lp_public_key, pool)));
        }
        let pools = pools.into_values().collect::<Vec<_>>();
        if pools.is_empty() {
            warn!(
                "No conversion pools available: in address {asset_in_address}, out address {asset_out_address}",
            );
            return Err(SdkError::Generic(
                "No conversion pools available".to_string(),
            ));
        }

        // Extract max_slippage_bps with default fallback
        let max_slippage_bps = conversion_options
            .max_slippage_bps
            .unwrap_or(DEFAULT_TOKEN_CONVERSION_MAX_SLIPPAGE_BPS);

        // Select the best pool using multi-factor scoring
        let pool = flashnet::select_best_pool(
            &pools,
            &asset_in_address,
            amount_out,
            max_slippage_bps,
            self.services.config.network.into(),
        )?;

        Ok(TokenConversionPool {
            asset_in_address,
            asset_out_address,
            pool,
        })
    }

    async fn estimate_conversion(
        &self,
        conversion_options: Option<&ConversionOptions>,
        token_identifier: Option<&String>,
        amount_out: u128,
    ) -> Result<Option<ConversionEstimate>, SdkError> {
        let Some(conversion_options) = conversion_options else {
            return Ok(None);
        };
        let conversion_pool = self
            .get_conversion_pool(conversion_options, token_identifier, amount_out)
            .await?;

        self.estimate_conversion_internal(&conversion_pool, conversion_options, amount_out)
            .await
    }

    async fn estimate_conversion_internal(
        &self,
        conversion_pool: &TokenConversionPool,
        conversion_options: &ConversionOptions,
        amount_out: u128,
    ) -> Result<Option<ConversionEstimate>, SdkError> {
        let TokenConversionPool {
            asset_in_address,
            asset_out_address,
            pool,
        } = conversion_pool;
        // Calculate the required amount in for the desired amount out
        let amount_in = pool.calculate_amount_in(
            asset_in_address,
            amount_out,
            conversion_options
                .max_slippage_bps
                .unwrap_or(DEFAULT_TOKEN_CONVERSION_MAX_SLIPPAGE_BPS),
            self.services.config.network.into(),
        )?;
        // Simulate the swap to validate the conversion
        let response = self
            .services
            .flashnet_client
            .simulate_swap(SimulateSwapRequest {
                asset_in_address: asset_in_address.clone(),
                asset_out_address: asset_out_address.clone(),
                pool_id: pool.lp_public_key,
                amount_in,
                integrator_bps: None,
            })
            .await?;
        if response.amount_out < amount_out {
            return Err(SdkError::Generic(format!(
                "Validation returned {} but expected at least {amount_out}",
                response.amount_out
            )));
        }
        Ok(response.fee_paid_asset_in.map(|fee| ConversionEstimate {
            options: conversion_options.clone(),
            amount: amount_in,
            fee,
        }))
    }

    /// Fetches a payment by its conversion identifier.
    /// The identifier can be either a spark transfer id or a token transaction hash.
    async fn fetch_payment_by_conversion_identifier(
        &self,
        identifier: &str,
        tx_inputs_are_ours: bool,
    ) -> Result<Payment, SdkError> {
        debug!("Fetching conversion payment for identifier: {}", identifier);
        let payment = if let Ok(transfer_id) = TransferId::from_str(identifier) {
            let transfers = self
                .services
                .spark_wallet
                .list_transfers(ListTransfersRequest {
                    transfer_ids: vec![transfer_id],
                    ..Default::default()
                })
                .await?;
            let transfer = transfers
                .items
                .first()
                .cloned()
                .ok_or_else(|| SdkError::Generic("Transfer not found".to_string()))?;
            transfer.try_into()
        } else {
            let token_transactions = self
                .services
                .spark_wallet
                .list_token_transactions(ListTokenTransactionsRequest {
                    token_transaction_hashes: vec![identifier.to_string()],
                    ..Default::default()
                })
                .await?;
            let token_transaction = token_transactions
                .items
                .first()
                .ok_or_else(|| SdkError::Generic("Token transaction not found".to_string()))?;
            let object_repository = ObjectCacheRepository::new(self.services.storage.clone());
            let payments = token_transaction_to_payments(
                &self.services.spark_wallet,
                &object_repository,
                token_transaction,
                tx_inputs_are_ours,
            )
            .await?;
            payments.first().cloned().ok_or_else(|| {
                SdkError::Generic("Payment not found for token transaction".to_string())
            })
        };
        payment
            .inspect(|p| debug!("Found payment: {p:?}"))
            .inspect_err(|e| debug!("No payment found: {e}"))
    }

    /// Updates the payment with the conversion info.
    ///
    /// Arguments:
    /// * `pool_id` - The pool id used for the conversion.
    /// * `outbound_identifier` - The outbound spark transfer id or token transaction hash.
    /// * `inbound_identifier` - The inbound spark transfer id or token transaction hash if the conversion was successful.
    /// * `refund_identifier` - The inbound refund spark transfer id or token transaction hash if the conversion was refunded.
    /// * `fee` - The fee paid for the conversion.
    ///
    /// Returns:
    /// * The sent payment id of the conversion.
    /// * The received payment id of the conversion.
    async fn update_payment_conversion_info(
        &self,
        pool_id: &PublicKey,
        outbound_identifier: String,
        inbound_identifier: Option<String>,
        refund_identifier: Option<String>,
        fee: Option<u128>,
        purpose: &ConversionPurpose,
    ) -> Result<(String, Option<String>), SdkError> {
        debug!(
            "Updating payment conversion info for pool_id: {pool_id}, outbound_identifier: {outbound_identifier}, inbound_identifier: {inbound_identifier:?}, refund_identifier: {refund_identifier:?}"
        );
        let cache = ObjectCacheRepository::new(self.services.storage.clone());
        let status = match (&inbound_identifier, &refund_identifier) {
            (Some(_), _) => ConversionStatus::Completed,
            (None, Some(_)) => ConversionStatus::Refunded,
            _ => ConversionStatus::RefundNeeded,
        };
        let pool_id_str = pool_id.to_string();
        let conversion_id = uuid::Uuid::now_v7().to_string();

        // Update the sent payment metadata
        let sent_payment = self
            .fetch_payment_by_conversion_identifier(&outbound_identifier, true)
            .await?;
        let sent_payment_id = sent_payment.id.clone();
        self.services
            .storage
            .set_payment_metadata(
                sent_payment_id.clone(),
                PaymentMetadata {
                    conversion_info: Some(ConversionInfo {
                        pool_id: pool_id_str.clone(),
                        conversion_id: conversion_id.clone(),
                        status: status.clone(),
                        fee,
                        purpose: None,
                    }),
                    ..Default::default()
                },
            )
            .await?;

        // Update the received payment metadata if available
        let received_payment_id = if let Some(identifier) = &inbound_identifier {
            let metadata = PaymentMetadata {
                conversion_info: Some(ConversionInfo {
                    pool_id: pool_id_str.clone(),
                    conversion_id: conversion_id.clone(),
                    status: status.clone(),
                    fee: None,
                    purpose: Some(purpose.clone()),
                }),
                ..Default::default()
            };
            if let Ok(payment) = self
                .fetch_payment_by_conversion_identifier(identifier, false)
                .await
            {
                self.services
                    .storage
                    .set_payment_metadata(payment.id.clone(), metadata)
                    .await?;
                Some(payment.id)
            } else {
                cache.save_payment_metadata(identifier, &metadata).await?;
                Some(identifier.clone())
            }
        } else {
            None
        };

        // Update the refund payment metadata if available
        if let Some(identifier) = &refund_identifier {
            let metadata = PaymentMetadata {
                conversion_info: Some(ConversionInfo {
                    pool_id: pool_id_str,
                    conversion_id,
                    status,
                    fee: None,
                    purpose: None,
                }),
                ..Default::default()
            };
            if let Ok(payment) = self
                .fetch_payment_by_conversion_identifier(identifier, false)
                .await
            {
                self.services
                    .storage
                    .set_payment_metadata(payment.id.clone(), metadata)
                    .await?;
            } else {
                cache.save_payment_metadata(identifier, &metadata).await?;
            }
        }

        self.services.storage.insert_payment(sent_payment).await?;

        Ok((sent_payment_id, received_payment_id))
    }
}

fn is_payment_match(payment: &Payment, identifier: &WaitForPaymentIdentifier) -> bool {
    match identifier {
        WaitForPaymentIdentifier::PaymentId(payment_id) => payment.id == *payment_id,
        WaitForPaymentIdentifier::PaymentRequest(payment_request) => {
            if let Some(details) = &payment.details {
                match details {
                    PaymentDetails::Lightning { invoice, .. } => {
                        invoice.to_lowercase() == payment_request.to_lowercase()
                    }
                    PaymentDetails::Spark {
                        invoice_details: invoice,
                        ..
                    }
                    | PaymentDetails::Token {
                        invoice_details: invoice,
                        ..
                    } => {
                        if let Some(invoice) = invoice {
                            invoice.invoice.to_lowercase() == payment_request.to_lowercase()
                        } else {
                            false
                        }
                    }
                    PaymentDetails::Withdraw { tx_id: _ }
                    | PaymentDetails::Deposit { tx_id: _ } => false,
                }
            } else {
                false
            }
        }
    }
}

struct BalanceWatcher {
    spark_wallet: Arc<SparkWallet>,
    storage: Arc<dyn Storage>,
}

impl BalanceWatcher {
    fn new(spark_wallet: Arc<SparkWallet>, storage: Arc<dyn Storage>) -> Self {
        Self {
            spark_wallet,
            storage,
        }
    }
}

#[macros::async_trait]
impl EventListener for BalanceWatcher {
    async fn on_event(&self, event: SdkEvent) {
        match event {
            SdkEvent::PaymentSucceeded { .. } | SdkEvent::ClaimedDeposits { .. } => {
                match update_balances(self.spark_wallet.clone(), self.storage.clone()).await {
                    Ok(()) => info!("Balance updated successfully"),
                    Err(e) => error!("Failed to update balance: {e:?}"),
                }
            }
            _ => {}
        }
    }
}

async fn update_balances(
    spark_wallet: Arc<SparkWallet>,
    storage: Arc<dyn Storage>,
) -> Result<(), SdkError> {
    let balance_sats = spark_wallet.get_balance().await?;
    let token_balances = spark_wallet
        .get_token_balances()
        .await?
        .into_iter()
        .map(|(k, v)| (k, v.into()))
        .collect();
    let object_repository = ObjectCacheRepository::new(storage.clone());

    object_repository
        .save_account_info(&CachedAccountInfo {
            balance_sats,
            token_balances,
        })
        .await?;
    let identity_public_key = spark_wallet.get_identity_public_key();
    info!(
        "Balance updated successfully {} for identity {}",
        balance_sats, identity_public_key
    );
    Ok(())
}

struct InternalEventListener {
    tx: mpsc::Sender<SdkEvent>,
}

impl InternalEventListener {
    #[allow(unused)]
    pub fn new(tx: mpsc::Sender<SdkEvent>) -> Self {
        Self { tx }
    }
}

#[macros::async_trait]
impl EventListener for InternalEventListener {
    async fn on_event(&self, event: SdkEvent) {
        let _ = self.tx.send(event).await;
    }
}

fn process_success_action(
    payment: &Payment,
    success_action: Option<&SuccessAction>,
) -> Result<Option<SuccessActionProcessed>, LnurlError> {
    let Some(success_action) = success_action else {
        return Ok(None);
    };

    let data = match success_action {
        SuccessAction::Aes { data } => data,
        SuccessAction::Message { data } => {
            return Ok(Some(SuccessActionProcessed::Message { data: data.clone() }));
        }
        SuccessAction::Url { data } => {
            return Ok(Some(SuccessActionProcessed::Url { data: data.clone() }));
        }
    };

    let Some(PaymentDetails::Lightning { preimage, .. }) = &payment.details else {
        return Err(LnurlError::general(format!(
            "Invalid payment type: expected type `PaymentDetails::Lightning`, got payment details {:?}.",
            payment.details
        )));
    };

    let Some(preimage) = preimage else {
        return Ok(None);
    };

    let preimage =
        sha256::Hash::from_str(preimage).map_err(|_| LnurlError::general("Invalid preimage"))?;
    let preimage = preimage.as_byte_array();
    let result: AesSuccessActionDataResult = match (data, preimage).try_into() {
        Ok(data) => AesSuccessActionDataResult::Decrypted { data },
        Err(e) => AesSuccessActionDataResult::ErrorStatus {
            reason: e.to_string(),
        },
    };

    Ok(Some(SuccessActionProcessed::Aes { result }))
}

fn validate_breez_api_key(api_key: &str) -> Result<(), SdkError> {
    let api_key_decoded = base64::engine::general_purpose::STANDARD
        .decode(api_key.as_bytes())
        .map_err(|err| {
            SdkError::Generic(format!(
                "Could not base64 decode the Breez API key: {err:?}"
            ))
        })?;
    let (_rem, cert) = parse_x509_certificate(&api_key_decoded).map_err(|err| {
        SdkError::Generic(format!("Invalid certificate for Breez API key: {err:?}"))
    })?;

    let issuer = cert
        .issuer()
        .iter_common_name()
        .next()
        .and_then(|cn| cn.as_str().ok());
    match issuer {
        Some(common_name) => {
            if !common_name.starts_with("Breez") {
                return Err(SdkError::Generic(
                    "Invalid certificate found for Breez API key: issuer mismatch. Please confirm that the certificate's origin is trusted"
                        .to_string()
                ));
            }
        }
        _ => {
            return Err(SdkError::Generic(
                "Could not parse Breez API key certificate: issuer is invalid or not found."
                    .to_string(),
            ));
        }
    }

    Ok(())
}
