use base64::Engine;
use bitcoin::{
    consensus::serialize,
    hashes::{Hash, sha256},
    hex::DisplayHex,
};
pub use breez_sdk_common::input::parse as parse_input;
use breez_sdk_common::{
    fiat::FiatService,
    input::{BitcoinAddressDetails, Bolt11InvoiceDetails, InputType},
};
use breez_sdk_common::{
    lnurl::{
        error::LnurlError,
        pay::{
            AesSuccessActionDataResult, SuccessAction, SuccessActionProcessed,
            ValidatedCallbackResponse, validate_lnurl_pay,
        },
    },
    rest::RestClient,
};
use spark_wallet::{
    ExitSpeed, InvoiceDescription, ListTokenTransactionsRequest, SparkAddress, SparkWallet,
    TokenInputs, TransferTokenOutput, WalletEvent, WalletTransfer,
};
use std::{str::FromStr, sync::Arc};
use tracing::{error, info, trace};
use web_time::{Duration, SystemTime};

use tokio::{
    select,
    sync::{Mutex, mpsc, oneshot, watch},
    time::timeout,
};
use tokio_with_wasm::alias as tokio;
use web_time::Instant;
use x509_parser::parse_x509_certificate;

use crate::{
    BitcoinChainService, CheckLightningAddressRequest, ClaimDepositRequest, ClaimDepositResponse,
    DepositInfo, Fee, GetPaymentRequest, GetPaymentResponse, LightningAddressInfo,
    ListFiatCurrenciesResponse, ListFiatRatesResponse, ListUnclaimedDepositsRequest,
    ListUnclaimedDepositsResponse, LnurlPayInfo, LnurlPayRequest, LnurlPayResponse, Logger,
    Network, PaymentDetails, PaymentMethod, PaymentStatus, PaymentType, PrepareLnurlPayRequest,
    PrepareLnurlPayResponse, RefundDepositRequest, RefundDepositResponse,
    RegisterLightningAddressRequest, SendOnchainFeeQuote, SendPaymentOptions,
    WaitForPaymentIdentifier, WaitForPaymentRequest, WaitForPaymentResponse,
    adaptors::sparkscan::payments_from_address_transaction_and_ssp_request,
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
        CachedAccountInfo, CachedSyncInfo, ObjectCacheRepository, PaymentMetadata,
        StaticDepositAddress, Storage, UpdateDepositPayload,
    },
    utils::{
        deposit_chain_syncer::DepositChainSyncer,
        run_with_shutdown,
        utxo_fetcher::{CachedUtxoFetcher, DetailedUtxo},
    },
};

const SPARKSCAN_API_URL: &str = "https://api.sparkscan.io";
const PAYMENT_SYNC_BATCH_SIZE: u64 = 50;

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
    let db_path = std::path::PathBuf::from_str(&request.storage_dir)?;
    let path_suffix: String = match &request.seed {
        crate::Seed::Mnemonic {
            mnemonic,
            passphrase,
        } => {
            let str = format!("{mnemonic}:{passphrase:?}");
            sha256::Hash::hash(str.as_bytes())
                .to_string()
                .chars()
                .take(8)
                .collect()
        }
        crate::Seed::Entropy(vec) => sha256::Hash::hash(vec.as_slice())
            .to_string()
            .chars()
            .take(8)
            .collect(),
    };

    let storage_dir = db_path
        .join(request.config.network.to_string().to_lowercase())
        .join(path_suffix);

    let storage = default_storage(storage_dir.to_string_lossy().to_string())?;
    let builder = crate::SdkBuilder::new(request.config, request.seed, storage);
    let sdk = builder.build().await?;
    Ok(sdk)
}

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
#[cfg_attr(feature = "uniffi", uniffi::export)]
#[allow(clippy::needless_pass_by_value)]
pub fn default_storage(data_dir: String) -> Result<Arc<dyn Storage>, SdkError> {
    let db_path = std::path::PathBuf::from_str(&data_dir)?;

    let storage = crate::SqliteStorage::new(&db_path)?;
    Ok(Arc::new(storage))
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn default_config(network: Network) -> Config {
    Config {
        api_key: None,
        network,
        sync_interval_secs: 60, // every 1 minute
        max_deposit_claim_fee: None,
        lnurl_domain: Some("breez.tips".to_string()),
        prefer_spark_over_lightning: false,
        sparkscan_api_url: SPARKSCAN_API_URL.to_string(),
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
pub async fn parse(input: &str) -> Result<InputType, SdkError> {
    Ok(parse_input(input).await?)
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
}

impl BreezSdk {
    /// Creates a new instance of the `BreezSdk`
    pub(crate) fn init_and_start(params: BreezSdkParams) -> Result<Self, SdkError> {
        match &params.config.api_key {
            Some(api_key) => validate_breez_api_key(api_key)?,
            None => return Err(SdkError::Generic("Missing Breez API key".to_string())),
        }
        let (initial_synced_sender, initial_synced_watcher) = watch::channel(false);
        let sdk = Self {
            config: params.config,
            spark_wallet: params.spark_wallet,
            storage: params.storage,
            chain_service: params.chain_service,
            fiat_service: params.fiat_service,
            lnurl_client: params.lnurl_client,
            lnurl_server_client: params.lnurl_server_client,
            event_emitter: Arc::new(EventEmitter::new()),
            shutdown_sender: params.shutdown_sender,
            sync_trigger: tokio::sync::broadcast::channel(10).0,
            initial_synced_watcher,
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
            info!("sync_wallet_internal: Syncing with Spark network");
            self.spark_wallet.sync().await?;
            info!("sync_wallet_internal: Synced with Spark network completed");
        }
        self.sync_wallet_state_to_storage().await?;
        info!("sync_wallet_internal: Synced wallet state to storage completed");
        self.check_and_claim_static_deposits().await?;
        info!("sync_wallet_internal: Checked and claimed static deposits completed");
        let elapsed = start_time.elapsed();
        info!("sync_wallet_internal: Wallet sync completed in {elapsed:?}");
        self.event_emitter.emit(&SdkEvent::Synced {}).await;
        Ok(())
    }

    /// Synchronizes wallet state to persistent storage, making sure we have the latest balances and payments.
    async fn sync_wallet_state_to_storage(&self) -> Result<(), SdkError> {
        update_balances(self.spark_wallet.clone(), self.storage.clone()).await?;

        let object_repository = ObjectCacheRepository::new(self.storage.clone());
        self.sync_pending_payments().await?;
        self.sync_payments_to_storage(&object_repository).await?;

        Ok(())
    }

    async fn sync_payments_to_storage(
        &self,
        object_repository: &ObjectCacheRepository,
    ) -> Result<(), SdkError> {
        // Get the last payment id we processed from storage
        let cached_sync_info = object_repository
            .fetch_sync_info()
            .await?
            .unwrap_or_default();
        let last_synced_id = cached_sync_info.last_synced_payment_id;

        let spark_address = self.spark_wallet.get_spark_address()?.to_string();

        let mut payments_to_sync = Vec::new();

        // We'll keep querying in batches until we have all payments or we find the last synced payment
        let mut next_offset = 0_u64;
        let mut has_more = true;
        let mut found_last_synced = false;
        info!("Syncing payments to storage, offset = {next_offset}");
        while has_more && !found_last_synced {
            // Get batch of address transactions starting from current offset
            let response = sparkscan::Client::new(&self.config.sparkscan_api_url)
                .get_address_transactions_v1_address_address_transactions_get()
                .network(sparkscan::types::Network::from(self.config.network))
                .address(spark_address.to_string())
                .offset(next_offset)
                .limit(PAYMENT_SYNC_BATCH_SIZE)
                .send()
                .await?;
            let address_transactions = &response.data;

            let ssp_transfer_types = [
                sparkscan::types::AddressTransactionType::BitcoinDeposit,
                sparkscan::types::AddressTransactionType::BitcoinWithdrawal,
                sparkscan::types::AddressTransactionType::LightningPayment,
            ];
            let ssp_user_requests = self
                .spark_wallet
                .query_ssp_user_requests(
                    address_transactions
                        .iter()
                        .filter(|tx| ssp_transfer_types.contains(&tx.type_))
                        .map(|tx| tx.id.clone())
                        .collect(),
                )
                .await?;

            info!(
                "Syncing payments to storage, offset = {next_offset}, transactions = {}",
                address_transactions.len()
            );
            // Process transactions in this batch
            for transaction in address_transactions {
                // Create payment records
                let payments = payments_from_address_transaction_and_ssp_request(
                    transaction,
                    ssp_user_requests.get(&transaction.id),
                    &spark_address,
                )?;

                for payment in payments {
                    if payment.id == last_synced_id {
                        info!(
                            "Last synced payment id found ({last_synced_id}), stopping sync and proceeding to insert {} payments",
                            payments_to_sync.len()
                        );
                        found_last_synced = true;
                        break;
                    }
                    payments_to_sync.push(payment);
                }

                // If we found the last synced payment, stop processing this batch
                if found_last_synced {
                    break;
                }
            }

            // Check if we have more transfers to fetch
            next_offset = next_offset.saturating_add(u64::try_from(address_transactions.len())?);
            has_more = address_transactions.len() as u64 == PAYMENT_SYNC_BATCH_SIZE;
        }

        // Insert payment into storage from oldest to newest
        for payment in payments_to_sync.iter().rev() {
            self.storage.insert_payment(payment.clone()).await?;
            info!("Inserted payment: {payment:?}");
            object_repository
                .save_sync_info(&CachedSyncInfo {
                    last_synced_payment_id: payment.id.clone(),
                })
                .await?;
        }

        Ok(())
    }

    /// Syncs pending payments so that we have their latest status
    /// Uses the Spark SDK API (SparkWallet) to get the latest status of the payments
    async fn sync_pending_payments(&self) -> Result<(), SdkError> {
        // TODO: implement pending payment syncing using sparkscan API (including live updates)
        // Advantages:
        // - No need to maintain payment adapter code for both models
        // - Can use live updates from sparkscan API
        // Why it can't be done now:
        // - Sparkscan needs one of the following:
        //   - Batch transaction querying by id
        //   - Sorting by updated_at timestamp in address transactions query (simpler)

        let pending_payments = self
            .storage
            .list_payments(None, None, Some(PaymentStatus::Pending))
            .await?;

        let (pending_token_payments, pending_bitcoin_payments): (Vec<_>, Vec<_>) = pending_payments
            .iter()
            .partition(|p| p.method == PaymentMethod::Token);

        info!(
            "Syncing pending bitcoin payments: {}",
            pending_bitcoin_payments.len()
        );
        self.sync_pending_bitcoin_payments(&pending_bitcoin_payments)
            .await?;
        info!(
            "Syncing pending token payments: {}",
            pending_token_payments.len()
        );
        self.sync_pending_token_payments(&pending_token_payments)
            .await?;

        Ok(())
    }

    async fn sync_pending_bitcoin_payments(
        &self,
        pending_bitcoin_payments: &[&Payment],
    ) -> Result<(), SdkError> {
        if pending_bitcoin_payments.is_empty() {
            return Ok(());
        }

        let transfer_ids: Vec<_> = pending_bitcoin_payments
            .iter()
            .map(|p| p.id.clone())
            .collect();

        let transfers = self
            .spark_wallet
            .list_transfers(None, Some(transfer_ids.clone()))
            .await?
            .items;

        for transfer in transfers {
            let payment = Payment::try_from(transfer)?;
            info!("Inserting previously pending bitcoin payment: {payment:?}");
            self.storage.insert_payment(payment).await?;
        }

        Ok(())
    }

    async fn sync_pending_token_payments(
        &self,
        pending_token_payments: &[&Payment],
    ) -> Result<(), SdkError> {
        if pending_token_payments.is_empty() {
            return Ok(());
        }

        let hash_pending_token_payments_map = pending_token_payments.iter().try_fold(
            std::collections::HashMap::new(),
            |mut acc: std::collections::HashMap<&_, Vec<_>>, payment| {
                let details = payment
                    .details
                    .as_ref()
                    .ok_or_else(|| SdkError::Generic("Payment details missing".to_string()))?;

                if let PaymentDetails::Token { tx_hash, .. } = details {
                    acc.entry(tx_hash).or_default().push(payment);
                    Ok(acc)
                } else {
                    Err(SdkError::Generic(
                        "Payment is not a token payment".to_string(),
                    ))
                }
            },
        )?;

        let token_transactions = self
            .spark_wallet
            .list_token_transactions(ListTokenTransactionsRequest {
                token_transaction_hashes: hash_pending_token_payments_map
                    .keys()
                    .map(|k| (*k).to_string())
                    .collect(),
                ..Default::default()
            })
            .await?
            .items;

        for token_transaction in token_transactions {
            let is_transfer_transaction =
                matches!(token_transaction.inputs, TokenInputs::Transfer(..));
            let payment_status = PaymentStatus::from_token_transaction_status(
                token_transaction.status,
                is_transfer_transaction,
            );
            if payment_status != PaymentStatus::Pending {
                let payments_to_update = hash_pending_token_payments_map
                    .get(&token_transaction.hash)
                    .ok_or(SdkError::Generic("Payment not found".to_string()))?;
                for payment in payments_to_update {
                    // For now, updating the status is enough
                    let mut updated_payment = (**payment).clone();
                    updated_payment.status = payment_status;
                    info!("Inserting previously pending token payment: {updated_payment:?}");
                    self.storage.insert_payment(updated_payment).await?;
                }
            }
        }

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
                    error!(
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
                .emit(&SdkEvent::ClaimDepositsFailed { unclaimed_deposits })
                .await;
        }
        if !claimed_deposits.is_empty() {
            self.event_emitter
                .emit(&SdkEvent::ClaimDepositsSucceeded { claimed_deposits })
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
        if let Some(max_deposit_claim_fee) = max_claim_fee {
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
                            max_fee: max_deposit_claim_fee,
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
                            max_fee: max_deposit_claim_fee,
                            actual_fee: spark_requested_fee,
                        });
                    }
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
        match &request.payment_method {
            ReceivePaymentMethod::SparkAddress => Ok(ReceivePaymentResponse {
                fee_sats: 0,
                payment_request: self.spark_wallet.get_spark_address()?.to_string(),
            }),
            ReceivePaymentMethod::BitcoinAddress => {
                // TODO: allow passing amount

                let object_repository = ObjectCacheRepository::new(self.storage.clone());

                // First lookup in storage cache
                let static_deposit_address =
                    object_repository.fetch_static_deposit_address().await?;
                if let Some(static_deposit_address) = static_deposit_address {
                    return Ok(ReceivePaymentResponse {
                        payment_request: static_deposit_address.address.to_string(),
                        fee_sats: 0,
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
                    fee_sats: 0,
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
                fee_sats: 0,
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
            ValidatedCallbackResponse::EndpointError { data } => {
                return Err(LnurlError::EndpointError(data.reason).into());
            }
            ValidatedCallbackResponse::EndpointSuccess { data } => data,
        };

        let prepare_response = self
            .prepare_send_payment(PrepareSendPaymentRequest {
                payment_request: success_data.pr,
                amount: Some(request.amount_sats),
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
        let mut payment = self
            .send_payment_internal(
                SendPaymentRequest {
                    prepare_response: PrepareSendPaymentResponse {
                        payment_method: SendPaymentMethod::Bolt11Invoice {
                            invoice_details: request.prepare_response.invoice_details,
                            spark_transfer_fee_sats: None,
                            lightning_fee_sats: request.prepare_response.fee_sats,
                        },
                        amount: request.prepare_response.amount_sats,
                        token_identifier: None,
                    },
                    options: None,
                },
                true,
            )
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
                },
            )
            .await?;

        emit_final_payment_status(&self.event_emitter, payment.clone()).await;
        Ok(LnurlPayResponse {
            payment,
            success_action,
        })
    }

    #[allow(clippy::too_many_lines)]
    pub async fn prepare_send_payment(
        &self,
        request: PrepareSendPaymentRequest,
    ) -> Result<PrepareSendPaymentResponse, SdkError> {
        // First check for spark address
        if let Ok(spark_address) = request.payment_request.parse::<SparkAddress>() {
            let payment_request_amount = if let Some(invoice_fields) =
                &spark_address.spark_invoice_fields
                && let Some(payment_type) = &invoice_fields.payment_type
            {
                match payment_type {
                    spark_wallet::SparkAddressPaymentType::SatsPayment(sats_payment_details) => {
                        if request.token_identifier.is_some() {
                            return Err(SdkError::InvalidInput(
                                "Token identifier can't be provided for this payment request: spark sats payment".to_string(),
                            ));
                        }
                        if sats_payment_details.amount.is_some() && request.amount.is_some() {
                            return Err(SdkError::InvalidInput(
                                "Amount can't be provided for this payment request: spark invoice defines amount".to_string(),
                            ));
                        }
                        sats_payment_details.amount
                    }
                    spark_wallet::SparkAddressPaymentType::TokensPayment(
                        tokens_payment_details,
                    ) => {
                        if request.token_identifier.is_none() {
                            return Err(SdkError::InvalidInput(
                                "Token identifier is required for this payment request: spark tokens payment".to_string(),
                            ));
                        }
                        if tokens_payment_details.amount.is_some() && request.amount.is_some() {
                            return Err(SdkError::InvalidInput(
                                "Amount can't be provided for this payment request: spark invoice defines amount".to_string(),
                            ));
                        }
                        tokens_payment_details.amount
                    }
                }
            } else {
                None
            };

            return Ok(PrepareSendPaymentResponse {
                payment_method: SendPaymentMethod::SparkAddress {
                    address: spark_address.to_string(),
                    fee: 0,
                    token_identifier: request.token_identifier.clone(),
                },
                amount: payment_request_amount
                    .or(request.amount)
                    .ok_or(SdkError::InvalidInput("Amount is required".to_string()))?,
                token_identifier: request.token_identifier,
            });
        }

        if request.token_identifier.is_some() {
            return Err(SdkError::InvalidInput(
                "Token identifier can't be provided for this payment request: non-spark address"
                    .to_string(),
            ));
        }

        let amount_sats = request.amount;

        // Then check for other types of inputs
        let parsed_input = parse(&request.payment_request).await?;
        match &parsed_input {
            InputType::Bolt11Invoice(detailed_bolt11_invoice) => {
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
                    .fetch_lightning_send_fee_estimate(&request.payment_request, amount_sats)
                    .await?;

                Ok(PrepareSendPaymentResponse {
                    payment_method: SendPaymentMethod::Bolt11Invoice {
                        invoice_details: detailed_bolt11_invoice.clone(),
                        spark_transfer_fee_sats,
                        lightning_fee_sats,
                    },
                    amount: amount_sats
                        .or(detailed_bolt11_invoice.amount_msat.map(|msat| msat / 1000))
                        .ok_or(SdkError::InvalidInput("Amount is required".to_string()))?,
                    token_identifier: None,
                })
            }
            InputType::BitcoinAddress(withdrawal_address) => {
                let fee_quote = self
                    .spark_wallet
                    .fetch_coop_exit_fee_quote(
                        &withdrawal_address.address,
                        Some(
                            amount_sats
                                .ok_or(SdkError::InvalidInput("Amount is required".to_string()))?,
                        ),
                    )
                    .await?;
                Ok(PrepareSendPaymentResponse {
                    payment_method: SendPaymentMethod::BitcoinAddress {
                        address: withdrawal_address.clone(),
                        fee_quote: fee_quote.into(),
                    },
                    amount: amount_sats
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
        self.send_payment_internal(request, false).await
    }

    #[allow(clippy::too_many_lines)]
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
            self.send_spark_token_payment(
                identifier,
                request.prepare_response.amount.into(),
                spark_address,
            )
            .await?
        } else {
            let transfer = self
                .spark_wallet
                .transfer(request.prepare_response.amount, &spark_address)
                .await?;
            transfer.try_into()?
        };

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
                amount_to_send,
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
                Some(request.prepare_response.amount),
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
        let payments = self
            .storage
            .list_payments(request.offset, request.limit, None)
            .await?;
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

    pub async fn check_lightning_address_available(
        &self,
        req: CheckLightningAddressRequest,
    ) -> Result<bool, SdkError> {
        let Some(client) = &self.lnurl_server_client else {
            return Err(SdkError::Generic(
                "LNURL server is not configured".to_string(),
            ));
        };

        let available = client.check_username_available(&req.username).await?;
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

        let description = match request.description {
            Some(description) => description,
            None => format!("Pay to {}@{}", request.username, client.domain()),
        };
        let params = crate::lnurl::RegisterLightningAddressRequest {
            username: request.username.clone(),
            description: description.clone(),
        };

        let response = client.register_lightning_address(&params).await?;
        let address_info = LightningAddressInfo {
            lightning_address: response.lightning_address,
            description,
            lnurl: response.lnurl,
            username: request.username,
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
}

impl BreezSdk {
    async fn send_spark_token_payment(
        &self,
        token_identifier: String,
        amount: u128,
        receiver_address: SparkAddress,
    ) -> Result<Payment, SdkError> {
        // Get token metadata before sending the payment to make sure we get it from cache
        let metadata = self
            .spark_wallet
            .get_tokens_metadata(&[&token_identifier])
            .await?
            .first()
            .ok_or(SdkError::Generic("Token metadata not found".to_string()))?
            .clone();

        let tx_hash = self
            .spark_wallet
            .transfer_tokens(vec![TransferTokenOutput {
                token_id: token_identifier,
                amount,
                receiver_address: receiver_address.clone(),
            }])
            .await?;

        // Build and insert pending payment into storage as it may take some time for sparkscan to detect it
        let payment = Payment {
            id: format!("{tx_hash}:0"), // Transaction output index 0 is for the receiver
            payment_type: PaymentType::Send,
            status: PaymentStatus::Pending,
            amount: amount.try_into()?,
            fees: 0,
            timestamp: SystemTime::now()
                .duration_since(web_time::UNIX_EPOCH)
                .map_err(|_| SdkError::Generic("Failed to get current timestamp".to_string()))?
                .as_secs(),
            method: PaymentMethod::Token,
            details: Some(PaymentDetails::Token {
                metadata: metadata.into(),
                tx_hash,
            }),
        };
        self.storage.insert_payment(payment.clone()).await?;

        Ok(payment)
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
                    PaymentDetails::Spark
                    | PaymentDetails::Token { .. }
                    | PaymentDetails::Withdraw { tx_id: _ }
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
            SdkEvent::PaymentSucceeded { .. } | SdkEvent::ClaimDepositsSucceeded { .. } => {
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
