use base64::Engine;
use bitcoin::{
    consensus::serialize,
    hashes::{Hash, sha256},
    hex::DisplayHex,
    secp256k1::{PublicKey, ecdsa::Signature},
};
pub use breez_sdk_common::input::parse as parse_input;
use breez_sdk_common::{
    fiat::FiatService,
    input::{BitcoinAddressDetails, Bolt11InvoiceDetails, ExternalInputParser, InputType},
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
use lnurl_models::sanitize_username;
use spark_wallet::{
    ExitSpeed, InvoiceDescription, SparkAddress, SparkWallet, TokenTransaction,
    TransferTokenOutput, WalletEvent, WalletTransfer,
};
use std::{str::FromStr, sync::Arc};
use tracing::{error, info, trace, warn};
use web_time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::{
    select,
    sync::{Mutex, mpsc, oneshot, watch},
    time::timeout,
};
use tokio_with_wasm::alias as tokio;
use web_time::Instant;
use x509_parser::parse_x509_certificate;

use crate::{
    BitcoinChainService, CheckLightningAddressRequest, CheckMessageRequest, CheckMessageResponse,
    ClaimDepositRequest, ClaimDepositResponse, DepositInfo, Fee, GetPaymentRequest,
    GetPaymentResponse, GetTokensMetadataRequest, GetTokensMetadataResponse, LightningAddressInfo,
    ListFiatCurrenciesResponse, ListFiatRatesResponse, ListUnclaimedDepositsRequest,
    ListUnclaimedDepositsResponse, LnurlPayInfo, LnurlPayRequest, LnurlPayResponse,
    LnurlWithdrawRequest, LnurlWithdrawResponse, Logger, Network, PaymentDetails, PaymentStatus,
    PrepareLnurlPayRequest, PrepareLnurlPayResponse, RefundDepositRequest, RefundDepositResponse,
    RegisterLightningAddressRequest, SendOnchainFeeQuote, SendPaymentOptions, SignMessageRequest,
    SignMessageResponse, WaitForPaymentIdentifier, WaitForPaymentRequest, WaitForPaymentResponse,
    error::SdkError,
    events::{EventEmitter, EventListener, SdkEvent},
    lnurl::LnurlServerClient,
    logger,
    models::{
        Config, GetInfoRequest, GetInfoResponse, ListPaymentsRequest, ListPaymentsResponse,
        Payment, PrepareSendPaymentRequest, PrepareSendPaymentResponse, ReceivePaymentMethod,
        ReceivePaymentRequest, ReceivePaymentResponse, SendPaymentMethod, SendPaymentRequest,
        SendPaymentResponse, SyncWalletRequest, SyncWalletResponse,
    },
    persist::{
        CachedAccountInfo, ObjectCacheRepository, PaymentMetadata, PaymentRequestMetadata,
        StaticDepositAddress, Storage, UpdateDepositPayload,
    },
    sync::SparkSyncService,
    utils::{
        deposit_chain_syncer::DepositChainSyncer,
        run_with_shutdown,
        token::{get_tokens_metadata_cached_or_query, token_transaction_to_payments},
        utxo_fetcher::{CachedUtxoFetcher, DetailedUtxo},
    },
};

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
const BREEZ_SYNC_SERVICE_URL: &str = "https://datasync.breez.technology";

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
const BREEZ_SYNC_SERVICE_URL: &str = "https://datasync.breez.technology:442";

#[derive(Clone, Debug)]
enum SyncType {
    Full,
    PaymentsOnly,
}

#[derive(Clone, Debug)]
struct SyncRequest {
    sync_type: SyncType,
    #[allow(clippy::type_complexity)]
    reply: Arc<Mutex<Option<oneshot::Sender<Result<(), SdkError>>>>>,
}

impl SyncRequest {
    fn full(reply: Option<oneshot::Sender<Result<(), SdkError>>>) -> Self {
        Self {
            sync_type: SyncType::Full,
            reply: Arc::new(Mutex::new(reply)),
        }
    }

    fn payments_only(reply: Option<oneshot::Sender<Result<(), SdkError>>>) -> Self {
        Self {
            sync_type: SyncType::PaymentsOnly,
            reply: Arc::new(Mutex::new(reply)),
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

/// `BreezSDK` is a wrapper around `SparkSDK` that provides a more structured API
/// with request/response objects and comprehensive error handling.
#[derive(Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct BreezSdk {
    config: Config,
    spark_wallet: Arc<SparkWallet>,
    storage: Arc<dyn Storage>,
    chain_service: Arc<dyn BitcoinChainService>,
    fiat_service: Arc<dyn FiatService>,
    lnurl_client: Arc<dyn RestClient>,
    lnurl_server_client: Option<Arc<dyn LnurlServerClient>>,
    event_emitter: Arc<EventEmitter>,
    shutdown_sender: watch::Sender<()>,
    sync_trigger: tokio::sync::broadcast::Sender<SyncRequest>,
    initial_synced_watcher: watch::Receiver<bool>,
    external_input_parsers: Vec<ExternalInputParser>,
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

#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn default_config(network: Network) -> Config {
    Config {
        api_key: None,
        network,
        sync_interval_secs: 60, // every 1 minute
        max_deposit_claim_fee: Some(Fee::Rate { sat_per_vbyte: 1 }),
        lnurl_domain: Some("breez.tips".to_string()),
        prefer_spark_over_lightning: false,
        external_input_parsers: None,
        use_default_external_input_parsers: true,
        real_time_sync_server_url: Some(BREEZ_SYNC_SERVICE_URL.to_string()),
    }
}

pub(crate) struct BreezSdkParams {
    pub config: Config,
    pub storage: Arc<dyn Storage>,
    pub chain_service: Arc<dyn BitcoinChainService>,
    pub fiat_service: Arc<dyn FiatService>,
    pub lnurl_client: Arc<dyn RestClient>,
    pub lnurl_server_client: Option<Arc<dyn LnurlServerClient>>,
    pub shutdown_sender: watch::Sender<()>,
    pub spark_wallet: Arc<SparkWallet>,
    pub event_emitter: Arc<EventEmitter>,
}

impl BreezSdk {
    /// Creates a new instance of the `BreezSdk`
    pub(crate) fn init_and_start(params: BreezSdkParams) -> Result<Self, SdkError> {
        // In Regtest we allow running without a Breez API key to facilitate local
        // integration tests. For non-regtest networks, a valid API key is required.
        if !matches!(params.config.network, Network::Regtest) {
            match &params.config.api_key {
                Some(api_key) => validate_breez_api_key(api_key)?,
                None => return Err(SdkError::Generic("Missing Breez API key".to_string())),
            }
        }
        let (initial_synced_sender, initial_synced_watcher) = watch::channel(false);
        let external_input_parsers = params.config.get_all_external_input_parsers();
        let sdk = Self {
            config: params.config,
            spark_wallet: params.spark_wallet,
            storage: params.storage,
            chain_service: params.chain_service,
            fiat_service: params.fiat_service,
            lnurl_client: params.lnurl_client,
            lnurl_server_client: params.lnurl_server_client,
            event_emitter: params.event_emitter,
            shutdown_sender: params.shutdown_sender,
            sync_trigger: tokio::sync::broadcast::channel(10).0,
            initial_synced_watcher,
            external_input_parsers,
        };

        sdk.start(initial_synced_sender);
        Ok(sdk)
    }

    /// Starts the SDK's background tasks
    ///
    /// This method initiates the following backround tasks:
    /// 1. `periodic_sync`: the wallet with the Spark network    
    /// 2. `monitor_deposits`: monitors for new deposits
    fn start(&self, initial_synced_sender: watch::Sender<bool>) {
        self.periodic_sync(initial_synced_sender);
        self.try_recover_lightning_address();
    }

    /// Refreshes the user's lightning address on the server on startup.
    fn try_recover_lightning_address(&self) {
        let sdk = self.clone();
        tokio::spawn(async move {
            if sdk.config.lnurl_domain.is_none() {
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

    fn periodic_sync(&self, initial_synced_sender: watch::Sender<bool>) {
        let sdk = self.clone();
        let mut shutdown_receiver = sdk.shutdown_sender.subscribe();
        let mut subscription = sdk.spark_wallet.subscribe_events();
        let sync_trigger_sender = sdk.sync_trigger.clone();
        let mut sync_trigger_receiver = sdk.sync_trigger.clone().subscribe();
        let mut last_sync_time = SystemTime::now();
        let sync_interval = u64::from(self.config.sync_interval_secs);
        tokio::spawn(async move {
            let balance_watcher =
                BalanceWatcher::new(sdk.spark_wallet.clone(), sdk.storage.clone());
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
                      if let Ok(sync_request) = sync_type_res   {
                          info!("Sync trigger changed: {:?}", &sync_request);
                          let cloned_sdk = sdk.clone();
                          let initial_synced_sender = initial_synced_sender.clone();
                          if let Some(true) = run_with_shutdown(shutdown_receiver.clone(), "Sync trigger changed", async move {
                          if let Err(e) = cloned_sdk.sync_wallet_internal(sync_request.sync_type.clone()).await {
                              error!("Failed to sync wallet: {e:?}");
                              let () = sync_request.reply(Some(e)).await;
                              return false
                          }
                          if matches!(sync_request.sync_type, SyncType::Full) {
                            let () = sync_request.reply(None).await;
                            if let Err(e) = initial_synced_sender.send(true) {
                              error!("Failed to send initial synced signal: {e:?}");
                            }
                            return true
                          }
                          false
                        }).await {
                          last_sync_time = SystemTime::now();
                        }
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
                if let Err(e) = self.sync_trigger.send(SyncRequest::full(None)) {
                    error!("Failed to sync wallet: {e:?}");
                }
            }
            WalletEvent::TransferClaimed(transfer) => {
                info!("Transfer claimed");
                if let Ok(payment) = transfer.try_into() {
                    self.event_emitter
                        .emit(&SdkEvent::PaymentSucceeded { payment })
                        .await;
                }
                if let Err(e) = self.sync_trigger.send(SyncRequest::payments_only(None)) {
                    error!("Failed to sync wallet: {e:?}");
                }
            }
        }
    }

    async fn sync_wallet_internal(&self, sync_type: SyncType) -> Result<(), SdkError> {
        let start_time = Instant::now();
        if let SyncType::Full = sync_type {
            // Sync with the Spark network
            if let Err(e) = self.spark_wallet.sync().await {
                error!("sync_wallet_internal: Failed to sync with Spark network: {e:?}");
            }
        }
        if let Err(e) = self.sync_wallet_state_to_storage().await {
            error!("sync_wallet_internal: Failed to sync wallet state to storage: {e:?}");
        }
        if let Err(e) = self.check_and_claim_static_deposits().await {
            error!("sync_wallet_internal: Failed to check and claim static deposits: {e:?}");
        }
        let elapsed = start_time.elapsed();
        info!("sync_wallet_internal: Wallet sync completed in {elapsed:?}");
        self.event_emitter.emit(&SdkEvent::Synced {}).await;
        Ok(())
    }

    /// Synchronizes wallet state to persistent storage, making sure we have the latest balances and payments.
    async fn sync_wallet_state_to_storage(&self) -> Result<(), SdkError> {
        update_balances(self.spark_wallet.clone(), self.storage.clone()).await?;

        let sync_service = SparkSyncService::new(self.spark_wallet.clone(), self.storage.clone());
        sync_service.sync_payments().await?;

        Ok(())
    }

    async fn check_and_claim_static_deposits(&self) -> Result<(), SdkError> {
        let to_claim = DepositChainSyncer::new(
            self.chain_service.clone(),
            self.storage.clone(),
            self.spark_wallet.clone(),
        )
        .sync()
        .await?;

        let mut claimed_deposits: Vec<DepositInfo> = Vec::new();
        let mut unclaimed_deposits: Vec<DepositInfo> = Vec::new();
        for detailed_utxo in to_claim {
            match self
                .claim_utxo(&detailed_utxo, self.config.max_deposit_claim_fee.clone())
                .await
            {
                Ok(_) => {
                    info!("Claimed utxo {}:{}", detailed_utxo.txid, detailed_utxo.vout);
                    self.storage
                        .delete_deposit(detailed_utxo.txid.to_string(), detailed_utxo.vout)
                        .await?;
                    claimed_deposits.push(detailed_utxo.into());
                }
                Err(e) => {
                    warn!(
                        "Failed to claim utxo {}:{}: {e}",
                        detailed_utxo.txid, detailed_utxo.vout
                    );
                    self.storage
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
            self.event_emitter
                .emit(&SdkEvent::UnclaimedDeposits { unclaimed_deposits })
                .await;
        }
        if !claimed_deposits.is_empty() {
            self.event_emitter
                .emit(&SdkEvent::ClaimedDeposits { claimed_deposits })
                .await;
        }
        Ok(())
    }

    async fn claim_utxo(
        &self,
        detailed_utxo: &DetailedUtxo,
        max_claim_fee: Option<Fee>,
    ) -> Result<WalletTransfer, SdkError> {
        info!(
            "Fetching static deposit claim quote for deposit tx {}:{} and amount: {}",
            detailed_utxo.txid, detailed_utxo.vout, detailed_utxo.value
        );
        let quote = self
            .spark_wallet
            .fetch_static_deposit_claim_quote(detailed_utxo.tx.clone(), Some(detailed_utxo.vout))
            .await?;
        let spark_requested_fee = detailed_utxo.value.saturating_sub(quote.credit_amount_sats);
        let Some(max_deposit_claim_fee) = max_claim_fee else {
            return Err(SdkError::DepositClaimFeeExceeded {
                tx: detailed_utxo.txid.to_string(),
                vout: detailed_utxo.vout,
                max_fee: None,
                actual_fee: spark_requested_fee,
            });
        };
        match max_deposit_claim_fee {
            Fee::Fixed { amount } => {
                info!(
                    "User max fee: {} spark requested fee: {}",
                    amount, spark_requested_fee
                );
                if spark_requested_fee > amount {
                    return Err(SdkError::DepositClaimFeeExceeded {
                        tx: detailed_utxo.txid.to_string(),
                        vout: detailed_utxo.vout,
                        max_fee: Some(max_deposit_claim_fee),
                        actual_fee: spark_requested_fee,
                    });
                }
            }
            Fee::Rate { sat_per_vbyte } => {
                // The claim tx size is 99 vbytes
                const CLAIM_TX_SIZE: u64 = 99;
                let user_max_fee = CLAIM_TX_SIZE.saturating_mul(sat_per_vbyte);
                info!(
                    "User max fee: {} spark requested fee: {}",
                    user_max_fee, spark_requested_fee
                );
                if spark_requested_fee > user_max_fee {
                    return Err(SdkError::DepositClaimFeeExceeded {
                        tx: detailed_utxo.txid.to_string(),
                        vout: detailed_utxo.vout,
                        max_fee: Some(max_deposit_claim_fee),
                        actual_fee: spark_requested_fee,
                    });
                }
            }
        }
        info!(
            "Claiming static deposit for utxo {}:{}",
            detailed_utxo.txid, detailed_utxo.vout
        );
        let transfer = self.spark_wallet.claim_static_deposit(quote).await?;
        info!(
            "Claimed static deposit transfer: {}",
            serde_json::to_string_pretty(&transfer)?
        );
        Ok(transfer)
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
        self.event_emitter.add_listener(listener).await
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
        self.event_emitter.remove_listener(id).await
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
        self.shutdown_sender
            .send(())
            .map_err(|_| SdkError::Generic("Failed to send shutdown signal".to_string()))?;

        self.shutdown_sender.closed().await;
        info!("Breez SDK disconnected");
        Ok(())
    }

    pub async fn parse(&self, input: &str) -> Result<InputType, SdkError> {
        Ok(parse_input(input, Some(self.external_input_parsers.clone())).await?)
    }

    /// Returns the balance of the wallet in satoshis
    #[allow(unused_variables)]
    pub async fn get_info(&self, request: GetInfoRequest) -> Result<GetInfoResponse, SdkError> {
        if request.ensure_synced.unwrap_or_default() {
            self.initial_synced_watcher
                .clone()
                .changed()
                .await
                .map_err(|_| {
                    SdkError::Generic("Failed to receive initial synced signal".to_string())
                })?;
        }
        let object_repository = ObjectCacheRepository::new(self.storage.clone());
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
        match request.payment_method {
            ReceivePaymentMethod::SparkAddress => Ok(ReceivePaymentResponse {
                fee: 0,
                payment_request: self
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
                let invoice = self.spark_wallet.create_spark_invoice(
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
                )?;
                Ok(ReceivePaymentResponse {
                    fee: 0,
                    payment_request: invoice,
                })
            }
            ReceivePaymentMethod::BitcoinAddress => {
                // TODO: allow passing amount

                let object_repository = ObjectCacheRepository::new(self.storage.clone());

                // First lookup in storage cache
                let static_deposit_address =
                    object_repository.fetch_static_deposit_address().await?;
                if let Some(static_deposit_address) = static_deposit_address {
                    return Ok(ReceivePaymentResponse {
                        payment_request: static_deposit_address.address.to_string(),
                        fee: 0,
                    });
                }

                // Then query existing addresses
                let deposit_addresses = self
                    .spark_wallet
                    .list_static_deposit_addresses(None)
                    .await?;

                // In case there are no addresses, generate a new one and cache it
                let address = match deposit_addresses.items.last() {
                    Some(address) => address.to_string(),
                    None => self
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
            } => Ok(ReceivePaymentResponse {
                payment_request: self
                    .spark_wallet
                    .create_lightning_invoice(
                        amount_sats.unwrap_or_default(),
                        Some(InvoiceDescription::Memo(description.clone())),
                        None,
                        self.config.prefer_spark_over_lightning,
                    )
                    .await?
                    .invoice,
                fee: 0,
            }),
        }
    }

    pub async fn prepare_lnurl_pay(
        &self,
        request: PrepareLnurlPayRequest,
    ) -> Result<PrepareLnurlPayResponse, SdkError> {
        let success_data = match validate_lnurl_pay(
            self.lnurl_client.as_ref(),
            request.amount_sats.saturating_mul(1_000),
            &None,
            &request.pay_request,
            self.config.network.into(),
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
            success_action: success_data.success_action,
        })
    }

    pub async fn lnurl_pay(&self, request: LnurlPayRequest) -> Result<LnurlPayResponse, SdkError> {
        let mut payment = Box::pin(self.send_payment_internal(
            SendPaymentRequest {
                prepare_response: PrepareSendPaymentResponse {
                    payment_method: SendPaymentMethod::Bolt11Invoice {
                        invoice_details: request.prepare_response.invoice_details,
                        spark_transfer_fee_sats: None,
                        lightning_fee_sats: request.prepare_response.fee_sats,
                    },
                    amount: request.prepare_response.amount_sats.into(),
                    token_identifier: None,
                },
                options: None,
            },
            true,
        ))
        .await?
        .payment;

        let success_action =
            process_success_action(&payment, request.prepare_response.success_action.as_ref())?;

        let lnurl_info = LnurlPayInfo {
            ln_address: request.prepare_response.pay_request.address,
            comment: request.prepare_response.comment,
            domain: Some(request.prepare_response.pay_request.domain),
            metadata: Some(request.prepare_response.pay_request.metadata_str),
            processed_success_action: success_action.clone(),
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

        self.storage
            .set_payment_metadata(
                payment.id.clone(),
                PaymentMetadata {
                    lnurl_pay_info: Some(lnurl_info),
                    lnurl_description,
                    ..Default::default()
                },
            )
            .await?;

        emit_final_payment_status(&self.event_emitter, payment.clone()).await;
        Ok(LnurlPayResponse {
            payment,
            success_action,
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
        let LnurlWithdrawRequest {
            amount_sats,
            withdraw_request,
            completion_timeout_secs,
        } = request;

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
                },
            })
            .await?
            .payment_request;

        // Store the LNURL withdraw metadata before executing the withdraw
        let cache = ObjectCacheRepository::new(self.storage.clone());
        cache
            .save_payment_request_metadata(&PaymentRequestMetadata {
                payment_request: payment_request.clone(),
                lnurl_withdraw_request_details: withdraw_request.clone(),
            })
            .await?;

        // Perform the LNURL withdraw using the generated invoice
        let withdraw_response = execute_lnurl_withdraw(
            self.lnurl_client.as_ref(),
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
        let fut = self.wait_for_payment(WaitForPaymentRequest {
            identifier: WaitForPaymentIdentifier::PaymentRequest(payment_request.clone()),
        });

        let payment = match timeout(Duration::from_secs(completion_timeout_secs.into()), fut).await
        {
            // Payment completed successfully
            Ok(Ok(res)) => Some(res.payment),
            // Error occurred while waiting for payment
            Ok(Err(e)) => return Err(SdkError::Generic(format!("Error waiting for payment: {e}"))),
            // Timeout occurred
            Err(_) => None,
        };

        Ok(LnurlWithdrawResponse {
            payment_request,
            payment,
        })
    }

    #[allow(clippy::too_many_lines)]
    pub async fn prepare_send_payment(
        &self,
        request: PrepareSendPaymentRequest,
    ) -> Result<PrepareSendPaymentResponse, SdkError> {
        let parsed_input = self.parse(&request.payment_request).await?;
        match &parsed_input {
            InputType::SparkAddress(spark_address_details) => Ok(PrepareSendPaymentResponse {
                payment_method: SendPaymentMethod::SparkAddress {
                    address: spark_address_details.address.clone(),
                    fee: 0,
                    token_identifier: request.token_identifier.clone(),
                },
                amount: request
                    .amount
                    .ok_or(SdkError::InvalidInput("Amount is required".to_string()))?,
                token_identifier: request.token_identifier,
            }),
            InputType::SparkInvoice(spark_invoice_details) => {
                if let Some(token_identifier) = &spark_invoice_details.token_identifier {
                    if let Some(requested_token_identifier) = &request.token_identifier
                        && requested_token_identifier != token_identifier
                    {
                        return Err(SdkError::InvalidInput(
                            "Requested token identifier does not match invoice token identifier"
                                .to_string(),
                        ));
                    }
                    return Err(SdkError::InvalidInput(
                        "Token identifier is required for tokens invoice".to_string(),
                    ));
                } else if request.token_identifier.is_some() {
                    return Err(SdkError::InvalidInput(
                            "Token identifier can't be provided for this payment request: non-tokens invoice".to_string(),
                        ));
                }

                if let Some(expiry_time) = spark_invoice_details.expiry_time
                    && SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_err(|_| SdkError::Generic("Failed to get current time".to_string()))?
                        > Duration::from_secs(expiry_time)
                {
                    return Err(SdkError::InvalidInput("Invoice has expired".to_string()));
                }

                if let Some(sender_public_key) = &spark_invoice_details.sender_public_key
                    && self.spark_wallet.get_identity_public_key().to_string() != *sender_public_key
                {
                    return Err(SdkError::InvalidInput(
                        format!(
                            "Invoice can only be paid by sender public key {sender_public_key}",
                        )
                        .to_string(),
                    ));
                }

                if let Some(invoice_amount) = spark_invoice_details.amount
                    && let Some(request_amount) = request.amount
                    && invoice_amount != request_amount
                {
                    return Err(SdkError::InvalidInput(
                        "Requested amount does not match invoice amount".to_string(),
                    ));
                }

                Ok(PrepareSendPaymentResponse {
                    payment_method: SendPaymentMethod::SparkInvoice {
                        spark_invoice_details: spark_invoice_details.clone(),
                        fee: 0,
                        token_identifier: request.token_identifier.clone(),
                    },
                    amount: spark_invoice_details
                        .amount
                        .or(request.amount)
                        .ok_or(SdkError::InvalidInput("Amount is required".to_string()))?,
                    token_identifier: request.token_identifier,
                })
            }
            InputType::Bolt11Invoice(detailed_bolt11_invoice) => {
                if request.token_identifier.is_some() {
                    return Err(SdkError::InvalidInput(
                        "Token identifier can't be provided for this payment request: non-spark address"
                            .to_string(),
                    ));
                }

                let spark_address = self
                    .spark_wallet
                    .extract_spark_address(&request.payment_request)?;

                let spark_transfer_fee_sats = if spark_address.is_some() {
                    Some(0)
                } else {
                    None
                };

                let lightning_fee_sats = self
                    .spark_wallet
                    .fetch_lightning_send_fee_estimate(
                        &request.payment_request,
                        request
                            .amount
                            .map(|a| Ok::<u64, SdkError>(a.try_into()?))
                            .transpose()?,
                    )
                    .await?;

                Ok(PrepareSendPaymentResponse {
                    payment_method: SendPaymentMethod::Bolt11Invoice {
                        invoice_details: detailed_bolt11_invoice.clone(),
                        spark_transfer_fee_sats,
                        lightning_fee_sats,
                    },
                    amount: request
                        .amount
                        .or(detailed_bolt11_invoice
                            .amount_msat
                            .map(|msat| u128::from(msat) / 1000))
                        .ok_or(SdkError::InvalidInput("Amount is required".to_string()))?,
                    token_identifier: None,
                })
            }
            InputType::BitcoinAddress(withdrawal_address) => {
                if request.token_identifier.is_some() {
                    return Err(SdkError::InvalidInput(
                        "Token identifier can't be provided for this payment request: non-spark address"
                            .to_string(),
                    ));
                }

                let fee_quote = self
                    .spark_wallet
                    .fetch_coop_exit_fee_quote(
                        &withdrawal_address.address,
                        Some(
                            request
                                .amount
                                .ok_or(SdkError::InvalidInput("Amount is required".to_string()))?
                                .try_into()?,
                        ),
                    )
                    .await?;
                Ok(PrepareSendPaymentResponse {
                    payment_method: SendPaymentMethod::BitcoinAddress {
                        address: withdrawal_address.clone(),
                        fee_quote: fee_quote.into(),
                    },
                    amount: request
                        .amount
                        .ok_or(SdkError::InvalidInput("Amount is required".to_string()))?,
                    token_identifier: None,
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
        Box::pin(self.send_payment_internal(request, false)).await
    }

    /// Synchronizes the wallet with the Spark network
    #[allow(unused_variables)]
    pub async fn sync_wallet(
        &self,
        request: SyncWalletRequest,
    ) -> Result<SyncWalletResponse, SdkError> {
        let (tx, rx) = oneshot::channel();

        if let Err(e) = self.sync_trigger.send(SyncRequest::full(Some(tx))) {
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
        let payments = self.storage.list_payments(request).await?;
        Ok(ListPaymentsResponse { payments })
    }

    pub async fn get_payment(
        &self,
        request: GetPaymentRequest,
    ) -> Result<GetPaymentResponse, SdkError> {
        let payment = self.storage.get_payment_by_id(request.payment_id).await?;
        Ok(GetPaymentResponse { payment })
    }

    pub async fn claim_deposit(
        &self,
        request: ClaimDepositRequest,
    ) -> Result<ClaimDepositResponse, SdkError> {
        let detailed_utxo =
            CachedUtxoFetcher::new(self.chain_service.clone(), self.storage.clone())
                .fetch_detailed_utxo(&request.txid, request.vout)
                .await?;

        let max_fee = request
            .max_fee
            .or(self.config.max_deposit_claim_fee.clone());
        match self.claim_utxo(&detailed_utxo, max_fee).await {
            Ok(transfer) => {
                self.storage
                    .delete_deposit(detailed_utxo.txid.to_string(), detailed_utxo.vout)
                    .await?;
                if let Err(e) = self.sync_trigger.send(SyncRequest::payments_only(None)) {
                    error!("Failed to execute sync after deposit claim: {e:?}");
                }
                Ok(ClaimDepositResponse {
                    payment: transfer.try_into()?,
                })
            }
            Err(e) => {
                error!("Failed to claim deposit: {e:?}");
                self.storage
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
        let detailed_utxo =
            CachedUtxoFetcher::new(self.chain_service.clone(), self.storage.clone())
                .fetch_detailed_utxo(&request.txid, request.vout)
                .await?;
        let tx = self
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
        self.storage
            .update_deposit(
                deposit.txid.clone(),
                deposit.vout,
                UpdateDepositPayload::Refund {
                    refund_tx: tx_hex.clone(),
                    refund_txid: tx_id.clone(),
                },
            )
            .await?;

        self.chain_service
            .broadcast_transaction(tx_hex.clone())
            .await?;
        Ok(RefundDepositResponse { tx_id, tx_hex })
    }

    #[allow(unused_variables)]
    pub async fn list_unclaimed_deposits(
        &self,
        request: ListUnclaimedDepositsRequest,
    ) -> Result<ListUnclaimedDepositsResponse, SdkError> {
        let deposits = self.storage.list_deposits().await?;
        Ok(ListUnclaimedDepositsResponse { deposits })
    }

    pub async fn check_lightning_address_available(
        &self,
        req: CheckLightningAddressRequest,
    ) -> Result<bool, SdkError> {
        let Some(client) = &self.lnurl_server_client else {
            return Err(SdkError::Generic(
                "LNURL server is not configured".to_string(),
            ));
        };

        let username = sanitize_username(&req.username);
        let available = client.check_username_available(&username).await?;
        Ok(available)
    }

    pub async fn get_lightning_address(&self) -> Result<Option<LightningAddressInfo>, SdkError> {
        let cache = ObjectCacheRepository::new(self.storage.clone());
        Ok(cache.fetch_lightning_address().await?)
    }

    pub async fn register_lightning_address(
        &self,
        request: RegisterLightningAddressRequest,
    ) -> Result<LightningAddressInfo, SdkError> {
        let cache = ObjectCacheRepository::new(self.storage.clone());
        let Some(client) = &self.lnurl_server_client else {
            return Err(SdkError::Generic(
                "LNURL server is not configured".to_string(),
            ));
        };

        let username = sanitize_username(&request.username);

        let description = match request.description {
            Some(description) => description,
            None => format!("Pay to {}@{}", username, client.domain()),
        };
        let params = crate::lnurl::RegisterLightningAddressRequest {
            username: username.clone(),
            description: description.clone(),
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

    pub async fn delete_lightning_address(&self) -> Result<(), SdkError> {
        let cache = ObjectCacheRepository::new(self.storage.clone());
        let Some(address_info) = cache.fetch_lightning_address().await? else {
            return Ok(());
        };

        let Some(client) = &self.lnurl_server_client else {
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
        let currencies = self.fiat_service.fetch_fiat_currencies().await?;
        Ok(ListFiatCurrenciesResponse { currencies })
    }

    /// List the latest rates of fiat currencies, sorted by name.
    pub async fn list_fiat_rates(&self) -> Result<ListFiatRatesResponse, SdkError> {
        let rates = self.fiat_service.fetch_fiat_rates().await?;
        Ok(ListFiatRatesResponse { rates })
    }

    pub async fn wait_for_payment(
        &self,
        request: WaitForPaymentRequest,
    ) -> Result<WaitForPaymentResponse, SdkError> {
        let (tx, mut rx) = mpsc::channel(20);
        let id = self
            .add_event_listener(Box::new(InternalEventListener::new(tx)))
            .await;

        // First check if we already have the payment in storage
        if let WaitForPaymentIdentifier::PaymentRequest(payment_request) = &request.identifier
            && let Some(payment) = self
                .storage
                .get_payment_by_invoice(payment_request.clone())
                .await?
        {
            self.remove_event_listener(&id).await;
            return Ok(WaitForPaymentResponse { payment });
        }

        // Otherwise, we wait for a matching payment event
        let payment_result = loop {
            let Some(event) = rx.recv().await else {
                break Err(SdkError::Generic("Event channel closed".to_string()));
            };

            let SdkEvent::PaymentSucceeded { payment } = event else {
                continue;
            };

            if is_payment_match(&payment, &request) {
                break Ok(payment);
            }
        };

        self.remove_event_listener(&id).await;
        Ok(WaitForPaymentResponse {
            payment: payment_result?,
        })
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
            &self.spark_wallet,
            &ObjectCacheRepository::new(self.storage.clone()),
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
        let pubkey = self.spark_wallet.get_identity_public_key().to_string();
        let signature = self.spark_wallet.sign_message(&request.message).await?;
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
            .spark_wallet
            .verify_message(&request.message, &signature, &pubkey)
            .await
            .is_ok();
        Ok(CheckMessageResponse { is_valid })
    }
}

// Separate impl block to avoid exposing private methods to uniffi.
impl BreezSdk {
    async fn send_payment_internal(
        &self,
        request: SendPaymentRequest,
        suppress_payment_event: bool,
    ) -> Result<SendPaymentResponse, SdkError> {
        let res = match &request.prepare_response.payment_method {
            SendPaymentMethod::SparkAddress {
                address,
                token_identifier,
                ..
            } => {
                self.send_spark_address(address, token_identifier.clone(), &request)
                    .await
            }
            SendPaymentMethod::SparkInvoice {
                spark_invoice_details,
                ..
            } => {
                self.send_spark_invoice(&spark_invoice_details.invoice, &request)
                    .await
            }
            SendPaymentMethod::Bolt11Invoice {
                invoice_details,
                spark_transfer_fee_sats,
                lightning_fee_sats,
            } => {
                self.send_bolt11_invoice(
                    invoice_details,
                    *spark_transfer_fee_sats,
                    *lightning_fee_sats,
                    &request,
                )
                .await
            }
            SendPaymentMethod::BitcoinAddress { address, fee_quote } => {
                self.send_bitcoin_address(address, fee_quote, &request)
                    .await
            }
        };
        if let Ok(response) = &res {
            //TODO: We get incomplete payments here from the ssp so better not to persist for now.
            // we trigger the sync here anyway to get the fresh payment.
            //self.storage.insert_payment(response.payment.clone()).await?;
            if !suppress_payment_event {
                emit_final_payment_status(&self.event_emitter, response.payment.clone()).await;
            }
            if let Err(e) = self.sync_trigger.send(SyncRequest::payments_only(None)) {
                error!("Failed to send sync trigger: {e:?}");
            }
        }
        res
    }

    async fn send_spark_address(
        &self,
        address: &str,
        token_identifier: Option<String>,
        request: &SendPaymentRequest,
    ) -> Result<SendPaymentResponse, SdkError> {
        let spark_address = address
            .parse::<SparkAddress>()
            .map_err(|_| SdkError::InvalidInput("Invalid spark address".to_string()))?;

        let payment = if let Some(identifier) = token_identifier {
            self.send_spark_token_address(
                identifier,
                request.prepare_response.amount,
                spark_address,
            )
            .await?
        } else {
            let transfer = self
                .spark_wallet
                .transfer(request.prepare_response.amount.try_into()?, &spark_address)
                .await?;
            transfer.try_into()?
        };

        Ok(SendPaymentResponse { payment })
    }

    async fn send_spark_token_address(
        &self,
        token_identifier: String,
        amount: u128,
        receiver_address: SparkAddress,
    ) -> Result<Payment, SdkError> {
        let token_transaction = self
            .spark_wallet
            .transfer_tokens(vec![TransferTokenOutput {
                token_id: token_identifier,
                amount,
                receiver_address: receiver_address.clone(),
                spark_invoice: None,
            }])
            .await?;

        self.map_and_persist_token_transaction(&token_transaction)
            .await
    }

    async fn send_spark_invoice(
        &self,
        invoice: &str,
        request: &SendPaymentRequest,
    ) -> Result<SendPaymentResponse, SdkError> {
        let payment = match self
            .spark_wallet
            .fulfill_spark_invoice(invoice, Some(request.prepare_response.amount))
            .await?
        {
            spark_wallet::FulfillSparkInvoiceResult::Transfer(wallet_transfer) => {
                (*wallet_transfer).try_into()?
            }
            spark_wallet::FulfillSparkInvoiceResult::TokenTransaction(token_transaction) => {
                self.map_and_persist_token_transaction(&token_transaction)
                    .await?
            }
        };

        Ok(SendPaymentResponse { payment })
    }

    async fn map_and_persist_token_transaction(
        &self,
        token_transaction: &TokenTransaction,
    ) -> Result<Payment, SdkError> {
        let object_repository = ObjectCacheRepository::new(self.storage.clone());
        let payments = token_transaction_to_payments(
            &self.spark_wallet,
            &object_repository,
            token_transaction,
            true,
        )
        .await?;
        for payment in &payments {
            self.storage.insert_payment(payment.clone()).await?;
        }

        payments
            .first()
            .ok_or(SdkError::Generic(
                "No payment created from token invoice".to_string(),
            ))
            .cloned()
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
            _ => (self.config.prefer_spark_over_lightning, None),
        };
        let fee_sats = match (prefer_spark, spark_transfer_fee_sats, lightning_fee_sats) {
            (true, Some(fee), _) => fee,
            _ => lightning_fee_sats,
        };

        let payment_response = self
            .spark_wallet
            .pay_lightning_invoice(
                &invoice_details.invoice.bolt11,
                amount_to_send
                    .map(|a| Ok::<u64, SdkError>(a.try_into()?))
                    .transpose()?,
                Some(fee_sats),
                prefer_spark,
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

        let fut = self.wait_for_payment(WaitForPaymentRequest {
            identifier: WaitForPaymentIdentifier::PaymentId(payment.id.clone()),
        });
        let payment = match timeout(Duration::from_secs(completion_timeout_secs.into()), fut).await
        {
            Ok(res) => res?.payment,
            // On timeout return the pending payment.
            Err(_) => payment,
        };

        Ok(SendPaymentResponse { payment })
    }

    async fn send_bitcoin_address(
        &self,
        address: &BitcoinAddressDetails,
        fee_quote: &SendOnchainFeeQuote,
        request: &SendPaymentRequest,
    ) -> Result<SendPaymentResponse, SdkError> {
        let exit_speed: ExitSpeed = match &request.options {
            Some(SendPaymentOptions::BitcoinAddress { confirmation_speed }) => {
                confirmation_speed.clone().into()
            }
            None => ExitSpeed::Fast,
            _ => {
                return Err(SdkError::InvalidInput("Invalid options".to_string()));
            }
        };
        let response = self
            .spark_wallet
            .withdraw(
                &address.address,
                Some(request.prepare_response.amount.try_into()?),
                exit_speed,
                fee_quote.clone().into(),
            )
            .await?;
        Ok(SendPaymentResponse {
            payment: response.try_into()?,
        })
    }

    // Pools the lightning send payment untill it is in completed state.
    fn poll_lightning_send_payment(&self, payment: &Payment, ssp_id: String) {
        const MAX_POLL_ATTEMPTS: u32 = 20;
        let payment_id = payment.id.clone();
        info!("Polling lightning send payment {}", payment_id);

        let spark_wallet = self.spark_wallet.clone();
        let sync_trigger = self.sync_trigger.clone();
        let event_emitter = self.event_emitter.clone();
        let payment = payment.clone();
        let payment_id = payment_id.to_string();
        let mut shutdown = self.shutdown_sender.subscribe();

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
                          emit_final_payment_status(&event_emitter, payment.clone()).await;
                          if let Err(e) = sync_trigger.send(SyncRequest::payments_only(None)) {
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
        let cache = ObjectCacheRepository::new(self.storage.clone());

        let Some(client) = &self.lnurl_server_client else {
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
}

fn is_payment_match(payment: &Payment, request: &WaitForPaymentRequest) -> bool {
    match &request.identifier {
        WaitForPaymentIdentifier::PaymentId(payment_id) => payment.id == *payment_id,
        WaitForPaymentIdentifier::PaymentRequest(payment_request) => {
            if let Some(details) = &payment.details {
                match details {
                    PaymentDetails::Lightning { invoice, .. } => {
                        invoice.to_lowercase() == payment_request.to_lowercase()
                    }
                    PaymentDetails::Spark {
                        invoice_details: invoice,
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

async fn emit_final_payment_status(event_emitter: &EventEmitter, payment: Payment) {
    match payment.status {
        PaymentStatus::Completed => {
            event_emitter
                .emit(&SdkEvent::PaymentSucceeded { payment })
                .await;
        }
        PaymentStatus::Failed => {
            event_emitter
                .emit(&SdkEvent::PaymentFailed { payment })
                .await;
        }
        PaymentStatus::Pending => (),
    }
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
