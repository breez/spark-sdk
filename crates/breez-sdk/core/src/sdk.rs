use base64::Engine;
use bitcoin::{
    consensus::serialize,
    hashes::{Hash, sha256},
    hex::DisplayHex,
};
use breez_sdk_common::input::InputType;
pub use breez_sdk_common::input::parse as parse_input;
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
    DefaultSigner, ExitSpeed, InvoiceDescription, ListTokenTransactionsRequest, Order,
    PagingFilter, PayLightningInvoiceResult, SparkAddress, SparkWallet, TransferTokenOutput,
    WalletEvent, WalletTransfer,
};
use std::{str::FromStr, sync::Arc, time::UNIX_EPOCH};
use tracing::{error, info, trace};
use web_time::{Duration, SystemTime};

use tokio::{select, sync::watch};
use tokio_with_wasm::alias as tokio;
use web_time::Instant;
use x509_parser::parse_x509_certificate;

use crate::{
    BitcoinChainService, ClaimDepositRequest, ClaimDepositResponse, DepositInfo, Fee,
    GetPaymentRequest, GetPaymentResponse, ListUnclaimedDepositsRequest,
    ListUnclaimedDepositsResponse, LnurlPayInfo, LnurlPayRequest, LnurlPayResponse, Logger,
    Network, PaymentDetails, PaymentMethod, PaymentStatus, PaymentType, PrepareLnurlPayRequest,
    PrepareLnurlPayResponse, RefundDepositRequest, RefundDepositResponse, SendPaymentOptions,
    TokenMetadata,
    error::SdkError,
    events::{EventEmitter, EventListener, SdkEvent},
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
        utxo_fetcher::{CachedUtxoFetcher, DetailedUtxo},
    },
};

const PAYMENT_SYNC_BATCH_SIZE: u64 = 50;

#[derive(Clone, Debug)]
enum SyncType {
    Full,
    PaymentsOnly,
}

/// `BreezSDK` is a wrapper around `SparkSDK` that provides a more structured API
/// with request/response objects and comprehensive error handling.
#[derive(Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct BreezSdk {
    config: Config,
    spark_wallet: Arc<SparkWallet<DefaultSigner>>,
    storage: Arc<dyn Storage>,
    chain_service: Arc<dyn BitcoinChainService>,
    lnurl_client: Arc<dyn RestClient>,
    event_emitter: Arc<EventEmitter>,
    shutdown_sender: watch::Sender<()>,
    shutdown_receiver: watch::Receiver<()>,
    sync_trigger: tokio::sync::broadcast::Sender<SyncType>,
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
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub async fn connect(request: crate::ConnectRequest) -> Result<BreezSdk, SdkError> {
    let db_path = std::path::PathBuf::from_str(&request.storage_dir)?;
    let path_suffix: String = sha256::Hash::hash(request.mnemonic.as_bytes())
        .to_string()
        .chars()
        .take(8)
        .collect();
    let storage_dir = db_path
        .join(request.config.network.to_string().to_lowercase())
        .join(path_suffix);

    let storage = default_storage(storage_dir.to_string_lossy().to_string())?;
    let builder = crate::SdkBuilder::new(request.config, request.mnemonic, storage);
    builder.build().await
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
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
pub async fn parse(input: &str) -> Result<InputType, SdkError> {
    Ok(parse_input(input).await?)
}

impl BreezSdk {
    /// Creates a new instance of the `BreezSdk`
    ///
    /// # Arguments
    ///
    /// * `config` - The Sdk configuration object
    /// * `signer` - Implementation of the `SparkSigner` trait
    /// * `storage` - Optional storage implementation for persistent data
    /// * `chain_service` - Implementation of the `ChainService` trait
    /// * `shutdown_sender` - Sender for shutdown signal
    /// * `shutdown_receiver` - Receiver for shutdown signal
    ///
    /// # Returns
    ///
    /// Result containing either the initialized `BreezSdk` or an `SdkError`
    pub(crate) async fn new(
        config: Config,
        signer: DefaultSigner,
        storage: Arc<dyn Storage>,
        chain_service: Arc<dyn BitcoinChainService>,
        lnurl_client: Arc<dyn RestClient>,
        shutdown_sender: watch::Sender<()>,
        shutdown_receiver: watch::Receiver<()>,
    ) -> Result<Self, SdkError> {
        let spark_wallet_config =
            spark_wallet::SparkWalletConfig::default_config(config.clone().network.into());
        let spark_wallet = SparkWallet::connect(spark_wallet_config, signer).await?;

        match &config.api_key {
            Some(api_key) => validate_breez_api_key(api_key)?,
            None => return Err(SdkError::Generic("Missing Breez API key".to_string())),
        }
        let sdk = Self {
            config,
            spark_wallet: Arc::new(spark_wallet),
            storage,
            chain_service,
            lnurl_client,
            event_emitter: Arc::new(EventEmitter::new()),
            shutdown_sender,
            shutdown_receiver,
            sync_trigger: tokio::sync::broadcast::channel(10).0,
        };
        Ok(sdk)
    }

    /// Starts the SDK's background tasks
    ///
    /// This method initiates the following backround tasks:
    /// 1. `periodic_sync`: the wallet with the Spark network    
    /// 2. `monitor_deposits`: monitors for new deposits
    pub(crate) fn start(&self) {
        self.periodic_sync();
    }

    fn periodic_sync(&self) {
        let sdk = self.clone();
        let mut shutdown_receiver = sdk.shutdown_receiver.clone();
        let mut subscription = sdk.spark_wallet.subscribe_events();
        let sync_trigger_sender = sdk.sync_trigger.clone();
        let mut sync_trigger_receiver = sdk.sync_trigger.clone().subscribe();
        let mut last_sync_time = SystemTime::now();
        let sync_interval = u64::from(self.config.sync_interval_secs);
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown_receiver.changed() => {
                        info!("Deposit tracking loop shutdown signal received");
                        return;
                    }
                    event = subscription.recv() => {
                        match event {
                            Ok(event) => {
                                info!("Received event: {event}");
                                trace!("Received event: {:?}", event);
                                sdk.handle_wallet_event(event);
                            }
                            Err(e) => {
                                error!("Failed to receive event: {e:?}");
                            }
                        }
                    }
                    () = async {
                      let sync_type_res = sync_trigger_receiver.recv().await;
                      if let Ok(sync_type) = sync_type_res   {
                          info!("Sync trigger changed: {:?}", &sync_type);

                          if let Err(e) = sdk.sync_wallet_internal(sync_type.clone()).await {
                              error!("Failed to sync wallet: {e:?}");
                          } else if matches!(sync_type, SyncType::Full) {
                            last_sync_time = SystemTime::now();
                          }
                      }
                  } => {}
                    // Ensure we sync at least the configured interval
                    () = tokio::time::sleep(Duration::from_secs(10)) => {
                        let now = SystemTime::now();
                        if let Ok(elapsed) = now.duration_since(last_sync_time) && elapsed.as_secs() >= sync_interval
                            && let Err(e) = sync_trigger_sender.send(SyncType::Full) {
                            error!("Failed to trigger periodic sync: {e:?}");
                        }
                    }
                }
            }
        });
    }

    fn handle_wallet_event(&self, event: WalletEvent) {
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
                if let Err(e) = self.sync_trigger.send(SyncType::Full) {
                    error!("Failed to sync wallet: {e:?}");
                }
            }
            WalletEvent::TransferClaimed(transfer) => {
                info!("Transfer claimed");
                if let Ok(payment) = transfer.try_into() {
                    self.event_emitter
                        .emit(&SdkEvent::PaymentSucceeded { payment });
                }
                if let Err(e) = self.sync_trigger.send(SyncType::PaymentsOnly) {
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
        self.event_emitter.emit(&SdkEvent::Synced {});
        Ok(())
    }

    /// Synchronizes wallet state to persistent storage, making sure we have the latest balances and payments.
    async fn sync_wallet_state_to_storage(&self) -> Result<(), SdkError> {
        let object_repository = ObjectCacheRepository::new(self.storage.clone());

        self.sync_balances_to_storage(&object_repository).await?;
        self.sync_bitcoin_payments_to_storage(&object_repository)
            .await?;
        self.sync_token_payments_to_storage(&object_repository)
            .await?;

        Ok(())
    }

    async fn sync_balances_to_storage(
        &self,
        object_repository: &ObjectCacheRepository,
    ) -> Result<(), SdkError> {
        let balance_sats = self.spark_wallet.get_balance().await?;
        let token_balances = self
            .spark_wallet
            .get_token_balances()
            .await?
            .into_iter()
            .map(|(k, v)| (k, v.into()))
            .collect();
        object_repository
            .save_account_info(&CachedAccountInfo {
                balance_sats,
                token_balances,
            })
            .await?;
        Ok(())
    }

    async fn sync_bitcoin_payments_to_storage(
        &self,
        object_repository: &ObjectCacheRepository,
    ) -> Result<(), SdkError> {
        // Get the last offset we processed from storage
        let cached_sync_info = object_repository
            .fetch_sync_info()
            .await?
            .unwrap_or_default();
        let current_offset = cached_sync_info.offset;

        // We'll keep querying in batches until we have all transfers
        let mut next_offset = current_offset;
        let mut has_more = true;
        info!("Syncing payments to storage, offset = {next_offset}");
        let mut pending_payments: u64 = 0;
        while has_more {
            // Get batch of transfers starting from current offset
            let transfers_response = self
                .spark_wallet
                .list_transfers(Some(PagingFilter::new(
                    Some(next_offset),
                    Some(PAYMENT_SYNC_BATCH_SIZE),
                    Some(Order::Ascending),
                )))
                .await?;

            info!(
                "Syncing bitcoin payments to storage, offset = {next_offset}, transfers = {}",
                transfers_response.len()
            );
            // Process transfers in this batch
            for transfer in &transfers_response {
                // Create a payment record
                let payment: Payment = transfer.clone().try_into()?;
                // Insert payment into storage
                if let Err(err) = self.storage.insert_payment(payment.clone()).await {
                    error!("Failed to insert bitcoin payment: {err:?}");
                }
                if payment.status == PaymentStatus::Pending {
                    pending_payments = pending_payments.saturating_add(1);
                }
                info!("Inserted bitcoin payment: {payment:?}");
            }

            // Check if we have more transfers to fetch
            next_offset = next_offset.saturating_add(u64::try_from(transfers_response.len())?);
            // Update our last processed offset in the storage. We should remove pending payments
            // from the offset as they might be removed from the list later.
            let save_res = object_repository
                .save_sync_info(&CachedSyncInfo {
                    offset: next_offset.saturating_sub(pending_payments),
                    last_synced_token_timestamp: cached_sync_info.last_synced_token_timestamp,
                })
                .await;

            if let Err(err) = save_res {
                error!("Failed to update last sync bitcoin offset: {err:?}");
            }
            has_more = transfers_response.len() as u64 == PAYMENT_SYNC_BATCH_SIZE;
        }

        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    async fn sync_token_payments_to_storage(
        &self,
        object_repository: &ObjectCacheRepository,
    ) -> Result<(), SdkError> {
        // Get the last offsets we processed from storage
        let cached_sync_info = object_repository
            .fetch_sync_info()
            .await?
            .unwrap_or_default();
        let last_synced_token_timestamp = cached_sync_info.last_synced_token_timestamp;

        let our_public_key = self.spark_wallet.get_identity_public_key();

        let mut latest_token_transaction_timestamp = None;

        // We'll keep querying in batches until we have all transfers
        let mut next_offset = 0;
        let mut has_more = true;
        info!(
            "Syncing token payments to storage, last synced token timestamp = {last_synced_token_timestamp:?}"
        );
        while has_more {
            // Get batch of token transactions starting from current offset
            let token_transactions = self
                .spark_wallet
                .list_token_transactions(ListTokenTransactionsRequest {
                    paging: Some(PagingFilter::new(
                        Some(next_offset),
                        Some(PAYMENT_SYNC_BATCH_SIZE),
                        None,
                    )),
                    ..Default::default()
                })
                .await?;

            // On first iteration, set the latest token transaction timestamp to the first transaction timestamp
            if next_offset == 0 {
                latest_token_transaction_timestamp =
                    token_transactions.first().map(|tx| tx.created_timestamp);
            }

            // Get prev out hashes of first input of each token transaction
            // Assumes all inputs of a tx share the same owner public key
            let token_transactions_prevout_hashes = token_transactions
                .iter()
                .filter_map(|tx| match &tx.inputs {
                    spark_wallet::TokenInputs::Transfer(token_transfer_input) => {
                        token_transfer_input.outputs_to_spend.first().cloned()
                    }
                    spark_wallet::TokenInputs::Mint(..) | spark_wallet::TokenInputs::Create(..) => {
                        None
                    }
                })
                .map(|output| output.prev_token_tx_hash)
                .collect::<Vec<_>>();

            // Since we are trying to fetch at most 1 parent transaction per token transaction,
            // we can fetch all in one go using same batch size
            let parent_transactions = self
                .spark_wallet
                .list_token_transactions(ListTokenTransactionsRequest {
                    paging: Some(PagingFilter::new(None, Some(PAYMENT_SYNC_BATCH_SIZE), None)),
                    owner_public_keys: Some(Vec::new()),
                    token_transaction_hashes: token_transactions_prevout_hashes,
                    ..Default::default()
                })
                .await?;

            info!(
                "Syncing token payments to storage, offset = {next_offset}, transactions = {}",
                token_transactions.len()
            );
            // Process transfers in this batch
            for transaction in &token_transactions {
                // Stop syncing if we have reached the last synced token transaction timestamp
                if let Some(last_synced_token_timestamp) = last_synced_token_timestamp
                    && transaction.created_timestamp <= last_synced_token_timestamp
                {
                    break;
                }

                let tx_inputs_are_ours = match &transaction.inputs {
                    spark_wallet::TokenInputs::Transfer(token_transfer_input) => {
                        let Some(first_input) = token_transfer_input.outputs_to_spend.first()
                        else {
                            return Err(SdkError::Generic(
                                "No input in token transfer input".to_string(),
                            ));
                        };
                        let Some(parent_transaction) = parent_transactions
                            .iter()
                            .find(|tx| tx.hash == first_input.prev_token_tx_hash)
                        else {
                            return Err(SdkError::Generic(
                                "Parent transaction not found".to_string(),
                            ));
                        };
                        let Some(output) = parent_transaction
                            .outputs
                            .get(first_input.prev_token_tx_vout as usize)
                        else {
                            return Err(SdkError::Generic("Output not found".to_string()));
                        };
                        output.owner_public_key == our_public_key
                    }
                    spark_wallet::TokenInputs::Mint(..) | spark_wallet::TokenInputs::Create(..) => {
                        false
                    }
                };

                // Create payment records
                let payments = self
                    .token_transaction_to_payments(transaction, tx_inputs_are_ours)
                    .await?;

                for payment in payments {
                    // Insert payment into storage
                    if let Err(err) = self.storage.insert_payment(payment.clone()).await {
                        error!("Failed to insert token payment: {err:?}");
                    }
                    info!("Inserted token payment: {payment:?}");
                }
            }

            // Check if we have more transfers to fetch
            next_offset = next_offset.saturating_add(u64::try_from(token_transactions.len())?);
            has_more = token_transactions.len() as u64 == PAYMENT_SYNC_BATCH_SIZE;
        }

        // Update our last processed transaction timestamp in the storage
        if let Some(latest_token_transaction_timestamp) = latest_token_transaction_timestamp {
            let save_res = object_repository
                .save_sync_info(&CachedSyncInfo {
                    offset: cached_sync_info.offset,
                    last_synced_token_timestamp: Some(latest_token_transaction_timestamp),
                })
                .await;

            if let Err(err) = save_res {
                error!("Failed to update last sync token timestamp: {err:?}");
            }
        }

        Ok(())
    }

    /// Converts a token transaction to payments
    ///
    /// Each resulting payment corresponds to a potential group of outputs that share the same owner public key.
    /// The id of the payment is the id of the first output in the group.
    ///
    /// Assumptions:
    /// - All outputs of a token transaction share the same token identifier
    /// - All inputs of a token transaction share the same owner public key
    #[allow(clippy::too_many_lines)]
    async fn token_transaction_to_payments(
        &self,
        transaction: &spark_wallet::TokenTransaction,
        tx_inputs_are_ours: bool,
    ) -> Result<Vec<Payment>, SdkError> {
        // Get token metadata for the first output (assuming all outputs have the same token)
        let token_identifier = transaction
            .outputs
            .first()
            .ok_or(SdkError::Generic(
                "No outputs in token transaction".to_string(),
            ))?
            .token_identifier
            .as_ref();
        let metadata: TokenMetadata = self
            .spark_wallet
            .get_tokens_metadata(&[token_identifier])
            .await?
            .first()
            .ok_or(SdkError::Generic("Token metadata not found".to_string()))?
            .clone()
            .into();

        let is_transfer_transaction =
            matches!(&transaction.inputs, spark_wallet::TokenInputs::Transfer(..));

        let timestamp = transaction
            .created_timestamp
            .duration_since(UNIX_EPOCH)
            .map_err(|_| {
                SdkError::Generic(
                    "Token transaction created timestamp is before UNIX_EPOCH".to_string(),
                )
            })?
            .as_secs();

        // Group outputs by owner public key
        let mut outputs_by_owner = std::collections::HashMap::new();
        for output in &transaction.outputs {
            outputs_by_owner
                .entry(output.owner_public_key)
                .or_insert_with(Vec::new)
                .push(output);
        }

        let mut payments = Vec::new();

        if tx_inputs_are_ours {
            // If inputs are ours, add an outgoing payment for each output group that is not ours
            for (owner_pubkey, outputs) in outputs_by_owner {
                if owner_pubkey != self.spark_wallet.get_identity_public_key() {
                    // This is an outgoing payment to another user
                    let total_amount = outputs.iter().map(|output| output.token_amount).sum();

                    let id = outputs
                        .first()
                        .ok_or(SdkError::Generic("No outputs in output group".to_string()))?
                        .id
                        .clone();

                    let payment = Payment {
                        id,
                        payment_type: PaymentType::Send,
                        status: PaymentStatus::from_token_transaction_status(
                            &transaction.status,
                            is_transfer_transaction,
                        ),
                        amount: total_amount,
                        fees: 0, // TODO: calculate actual fees when they start being charged
                        timestamp,
                        method: PaymentMethod::Token,
                        details: Some(PaymentDetails::Token {
                            metadata: metadata.clone(),
                        }),
                    };

                    payments.push(payment);
                }
                // Ignore outputs that belong to us (potential change outputs)
            }
        } else {
            // If inputs are not ours, add an incoming payment for our output group
            if let Some(our_outputs) =
                outputs_by_owner.get(&self.spark_wallet.get_identity_public_key())
            {
                let total_amount = our_outputs.iter().map(|output| output.token_amount).sum();

                let id = our_outputs
                    .first()
                    .ok_or(SdkError::Generic(
                        "No outputs in our output group".to_string(),
                    ))?
                    .id
                    .clone();

                let payment = Payment {
                    id,
                    payment_type: PaymentType::Receive,
                    status: PaymentStatus::from_token_transaction_status(
                        &transaction.status,
                        is_transfer_transaction,
                    ),
                    amount: total_amount,
                    fees: 0,
                    timestamp,
                    method: PaymentMethod::Token,
                    details: Some(PaymentDetails::Token { metadata }),
                };

                payments.push(payment);
            }
            // Ignore outputs that don't belong to us
        }

        Ok(payments)
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
                .emit(&SdkEvent::ClaimDepositsFailed { unclaimed_deposits });
        }
        if !claimed_deposits.is_empty() {
            self.event_emitter
                .emit(&SdkEvent::ClaimDepositsSucceeded { claimed_deposits });
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
    pub fn add_event_listener(&self, listener: Box<dyn EventListener>) -> String {
        self.event_emitter.add_listener(listener)
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
    pub fn remove_event_listener(&self, id: &str) -> bool {
        self.event_emitter.remove_listener(id)
    }

    /// Stops the SDK's background tasks
    ///
    /// This method stops the background tasks started by the `start()` method.
    /// It should be called before your application terminates to ensure proper cleanup.
    ///
    /// # Returns
    ///
    /// Result containing either success or an `SdkError` if the background task couldn't be stopped
    pub fn disconnect(&self) -> Result<(), SdkError> {
        self.shutdown_sender
            .send(())
            .map_err(|_| SdkError::Generic("Failed to send shutdown signal".to_string()))?;

        Ok(())
    }

    /// Returns the balance of the wallet in satoshis
    #[allow(unused_variables)]
    pub async fn get_info(&self, request: GetInfoRequest) -> Result<GetInfoResponse, SdkError> {
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
                payment_request: self.spark_wallet.get_spark_address().await?.to_string(),
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
                let address = match deposit_addresses.last() {
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
        let mut payment = self
            .send_payment_internal(
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
        let Some(PaymentDetails::Lightning { lnurl_pay_info, .. }) = &mut payment.details else {
            return Err(SdkError::Generic(
                "Expected Lightning payment details".to_string(),
            ));
        };
        *lnurl_pay_info = Some(lnurl_info.clone());

        self.storage
            .set_payment_metadata(
                payment.id.clone(),
                PaymentMetadata {
                    lnurl_pay_info: Some(lnurl_info),
                },
            )
            .await?;
        self.event_emitter.emit(&SdkEvent::PaymentSucceeded {
            payment: payment.clone(),
        });
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
                        sats_payment_details.amount.map(Into::into)
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
                    .fetch_lightning_send_fee_estimate(
                        &request.payment_request,
                        amount_sats
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
                    amount: amount_sats
                        .or(detailed_bolt11_invoice
                            .amount_msat
                            .map(|msat| u128::from(msat) / 1000))
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
        let res = match request.prepare_response.payment_method {
            SendPaymentMethod::SparkAddress {
                address,
                token_identifier,
                ..
            } => {
                let spark_address = address
                    .parse::<SparkAddress>()
                    .map_err(|_| SdkError::InvalidInput("Invalid spark address".to_string()))?;

                let payment = if let Some(identifier) = token_identifier {
                    self.send_spark_token_payment(
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
            SendPaymentMethod::Bolt11Invoice {
                invoice_details,
                spark_transfer_fee_sats,
                lightning_fee_sats,
            } => {
                let amount_to_send = match invoice_details.amount_msat {
                    // we are not sending amount in case the invoice contains it.
                    Some(_) => None,
                    // We are sending amount for zero amount invoice
                    None => Some(request.prepare_response.amount),
                };
                let use_spark = match request.options {
                    Some(SendPaymentOptions::Bolt11Invoice { use_spark }) => use_spark,
                    _ => false,
                };
                let fee_sats = match (use_spark, spark_transfer_fee_sats, lightning_fee_sats) {
                    (true, Some(fee), _) => fee,
                    _ => lightning_fee_sats,
                };
                if use_spark && spark_transfer_fee_sats.is_none() {
                    return Err(SdkError::InvalidInput(
                        "Cannot use spark to pay invoice as it doesn't contain a spark address"
                            .to_string(),
                    ));
                }

                let payment_response = self
                    .spark_wallet
                    .pay_lightning_invoice(
                        &invoice_details.invoice.bolt11,
                        amount_to_send
                            .map(|a| Ok::<u64, SdkError>(a.try_into()?))
                            .transpose()?,
                        Some(fee_sats),
                        use_spark,
                    )
                    .await?;
                let payment = match payment_response {
                    PayLightningInvoiceResult::LightningPayment(payment) => {
                        self.poll_lightning_send_payment(&payment.id);
                        Payment::from_lightning(
                            payment,
                            request.prepare_response.amount.try_into()?,
                        )?
                    }
                    PayLightningInvoiceResult::Transfer(payment) => payment.try_into()?,
                };
                Ok(SendPaymentResponse { payment })
            }
            SendPaymentMethod::BitcoinAddress { address, fee_quote } => {
                let exit_speed: ExitSpeed = match request.options {
                    Some(SendPaymentOptions::BitcoinAddress { confirmation_speed }) => {
                        confirmation_speed.into()
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
                        fee_quote.into(),
                    )
                    .await?;
                Ok(SendPaymentResponse {
                    payment: response.try_into()?,
                })
            }
        };
        if let Ok(response) = &res {
            //TODO: We get incomplete payments here from the ssp so better not to persist for now.
            // we trigger the sync here anyway to get the fresh payment.
            //self.storage.insert_payment(response.payment.clone()).await?;
            if !suppress_payment_event {
                self.event_emitter.emit(&SdkEvent::PaymentSucceeded {
                    payment: response.payment.clone(),
                });
            }
            if let Err(e) = self.sync_trigger.send(SyncType::PaymentsOnly) {
                error!("Failed to send sync trigger: {e:?}");
            }
        }
        res
    }

    // Pools the lightning send payment untill it is in completed state.
    fn poll_lightning_send_payment(&self, payment_id: &str) {
        const MAX_POLL_ATTEMPTS: u32 = 10;
        info!("Polling lightning send payment {}", payment_id);

        let spark_wallet = self.spark_wallet.clone();
        let sync_trigger = self.sync_trigger.clone();
        let payment_id = payment_id.to_string();
        let mut shutdown = self.shutdown_receiver.clone();

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
                    p = spark_wallet.fetch_lightning_send_payment(&payment_id) => {
                      if let Ok(Some(p)) = p  && p.payment_preimage.is_some(){
                          info!("Pollling payment preimage found");
                          if let Err(e) = sync_trigger.send(SyncType::PaymentsOnly) {
                              error!("Failed to send sync trigger: {e:?}");
                          }
                          return;
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
    pub fn sync_wallet(&self, request: SyncWalletRequest) -> Result<SyncWalletResponse, SdkError> {
        if let Err(e) = self.sync_trigger.send(SyncType::Full) {
            error!("Failed to send sync trigger: {e:?}");
        }
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
            .list_payments(request.offset, request.limit)
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
                if let Err(e) = self.sync_trigger.send(SyncType::PaymentsOnly) {
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
        let tx_id = tx.compute_txid().to_string();

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
}

impl BreezSdk {
    async fn send_spark_token_payment(
        &self,
        token_identifier: String,
        amount: u128,
        receiver_address: SparkAddress,
    ) -> Result<Payment, SdkError> {
        let tx_hash = self
            .spark_wallet
            .transfer_tokens(vec![TransferTokenOutput {
                token_id: token_identifier,
                amount,
                receiver_address,
            }])
            .await?;
        let tx = self
            .spark_wallet
            .list_token_transactions(ListTokenTransactionsRequest {
                token_transaction_hashes: vec![tx_hash],
                paging: None,
                ..Default::default()
            })
            .await?
            .first()
            .ok_or(SdkError::Generic(
                "Token transaction not found after being started".to_string(),
            ))?
            .clone();
        Ok(self
            .token_transaction_to_payments(&tx, true)
            .await?
            .first()
            .ok_or(SdkError::Generic(
                "No payment found in started token transaction".to_string(),
            ))?
            .clone())
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
        SdkError::Generic(format!("Invaid certificate for Breez API key: {err:?}"))
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
