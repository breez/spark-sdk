use bitcoin::{Transaction, consensus::encode::deserialize_hex};
use breez_sdk_common::input::InputType;
pub use breez_sdk_common::input::parse as parse_input;
use spark_wallet::{
    DefaultSigner, Order, PagingFilter, PayLightningInvoiceResult, SparkAddress, SparkWallet, Utxo,
    WalletEvent,
};
use std::{
    path::PathBuf,
    str::FromStr,
    sync::Arc,
    time::{Duration, Instant},
};
use tracing::{error, info, trace};

use tokio::sync::watch;

use crate::{
    BitcoinChainService, Fee, GetPaymentRequest, GetPaymentResponse, Logger, Network,
    PaymentStatus, SqliteStorage,
    error::SdkError,
    events::{EventEmitter, EventListener, SdkEvent},
    logger,
    models::{
        Config, GetInfoRequest, GetInfoResponse, ListPaymentsRequest, ListPaymentsResponse,
        Payment, PrepareReceivePaymentRequest, PrepareReceivePaymentResponse,
        PrepareSendPaymentRequest, PrepareSendPaymentResponse, ReceivePaymentMethod,
        ReceivePaymentRequest, ReceivePaymentResponse, SendPaymentMethod, SendPaymentRequest,
        SendPaymentResponse, SyncWalletRequest, SyncWalletResponse,
    },
    persist::{CachedAccountInfo, CachedSyncInfo, ObjectCacheRepository, Storage},
};

/// `BreezSDK` is a wrapper around `SparkSDK` that provides a more structured API
/// with request/response objects and comprehensive error handling.
#[derive(Clone)]
pub struct BreezSdk {
    config: Config,
    spark_wallet: Arc<SparkWallet<DefaultSigner>>,
    storage: Arc<dyn Storage>,
    chain_service: Arc<dyn BitcoinChainService + Send + Sync>,
    event_emitter: Arc<EventEmitter>,
    shutdown_sender: watch::Sender<()>,
    shutdown_receiver: watch::Receiver<()>,
}

pub async fn init_logging(
    log_dir: &str,
    app_logger: Option<Box<dyn Logger>>,
    log_filter: Option<String>,
) -> Result<(), SdkError> {
    logger::init_logging(log_dir, app_logger, log_filter)
}

pub fn default_storage(data_dir: String) -> Result<Box<dyn Storage>, SdkError> {
    let db_path = PathBuf::from_str(&data_dir)?;
    let storage = SqliteStorage::new(&db_path)?;
    Ok(Box::new(storage))
}

pub fn default_config(network: Network) -> Config {
    Config {
        network,
        deposits_monitoring_interval_secs: 5 * 60, // every 5 minutes
        max_deposit_claim_fee: None,
    }
}

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
    pub async fn new(
        config: Config,
        signer: DefaultSigner,
        storage: Arc<dyn Storage + Send + Sync>,
        chain_service: Arc<dyn BitcoinChainService + Send + Sync>,
        shutdown_sender: watch::Sender<()>,
        shutdown_receiver: watch::Receiver<()>,
    ) -> Result<Self, SdkError> {
        let spark_wallet_config =
            spark_wallet::SparkWalletConfig::default_config(config.clone().network.into());
        let spark_wallet = SparkWallet::connect(spark_wallet_config, signer).await?;

        let sdk = Self {
            config,
            spark_wallet: Arc::new(spark_wallet),
            storage,
            chain_service,
            event_emitter: Arc::new(EventEmitter::new()),
            shutdown_sender,
            shutdown_receiver,
        };

        Ok(sdk)
    }

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
    pub async fn remove_event_listener(&self, id: &str) -> bool {
        self.event_emitter.remove_listener(id)
    }

    /// Starts the SDK's background tasks
    ///
    /// This method initiates the following backround tasks:
    /// 1. `periodic_sync`: the wallet with the Spark network    
    ///
    pub fn start(&self) -> Result<(), SdkError> {
        self.periodic_sync();
        self.monitor_deposits();
        Ok(())
    }

    fn monitor_deposits(&self) {
        let sdk = self.clone();
        let mut shutdown_receiver = sdk.shutdown_receiver.clone();

        info!("Monitoring deposits started");
        // First interval is immediate, after first iteration we change it according to the configuration
        let mut deposits_monitoring_interval = 1;
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown_receiver.changed() => {
                        info!("Deposit tracking loop shutdown signal received");
                        return;
                    }
                    _ = tokio::time::sleep(Duration::from_secs(deposits_monitoring_interval.into())) => {

                      tokio::select! {
                        _ = shutdown_receiver.changed() => {
                          info!("Check claim static deposits shutdown signal received");
                          return;
                        }
                        claim_result = sdk.check_and_claim_static_deposits() => {
                          if let Err(e) = claim_result {
                            error!("Monitor deposits failed to claim static deposit: {e:?}");
                          }
                        }
                      }

                      deposits_monitoring_interval = sdk.config.deposits_monitoring_interval_secs;
                    }
                }
            }
        });
    }

    fn periodic_sync(&self) {
        let sdk = self.clone();
        let mut shutdown_receiver = sdk.shutdown_receiver.clone();
        let mut subscription = sdk.spark_wallet.subscribe_events();
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
                                info!("Received event: {event:?}");
                                sdk.handle_wallet_event(event).await;
                            }
                            Err(e) => {
                                error!("Failed to receive event: {e:?}");
                            }
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
                if let Err(e) = self.sync_payments_to_storage().await {
                    error!("Failed to sync payments to storage: {e:?}");
                }
            }
            WalletEvent::TransferClaimed(_) => {
                info!("Transfer claimed");
                if let Err(e) = self.sync_payments_to_storage().await {
                    error!("Failed to sync payments to storage: {e:?}");
                }
            }
        }
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
        self.shutdown_sender
            .send(())
            .map_err(|_| SdkError::GenericError("Failed to send shutdown signal".to_string()))?;

        Ok(())
    }

    /// Returns the balance of the wallet in satoshis
    pub async fn get_info(&self, _request: GetInfoRequest) -> Result<GetInfoResponse, SdkError> {
        let object_repository = ObjectCacheRepository::new(self.storage.clone());
        let account_info = object_repository.fetch_account_info()?.unwrap_or_default();
        Ok(GetInfoResponse {
            balance_sats: account_info.balance_sats,
        })
    }

    pub async fn prepare_receive_payment(
        &self,
        request: PrepareReceivePaymentRequest,
    ) -> Result<PrepareReceivePaymentResponse, SdkError> {
        match &request.payment_method {
            ReceivePaymentMethod::SparkAddress => Ok(PrepareReceivePaymentResponse {
                payment_method: request.payment_method,
                fee_sats: 0,
            }),
            ReceivePaymentMethod::BitcoinAddress => {
                Ok(PrepareReceivePaymentResponse {
                    payment_method: request.payment_method,
                    fee_sats: 0, // TODO: calculate fee
                })
            }
            ReceivePaymentMethod::Bolt11Invoice { .. } => Ok(PrepareReceivePaymentResponse {
                payment_method: request.payment_method,
                fee_sats: 0,
            }),
        }
    }

    pub async fn receive_payment(
        &self,
        request: ReceivePaymentRequest,
    ) -> Result<ReceivePaymentResponse, SdkError> {
        match &request.prepare_response.payment_method {
            ReceivePaymentMethod::SparkAddress => Ok(ReceivePaymentResponse {
                payment_request: self.spark_wallet.get_spark_address().await?.to_string(),
            }),
            ReceivePaymentMethod::BitcoinAddress => Ok(ReceivePaymentResponse {
                // TODO: allow passing amount
                payment_request: self
                    .spark_wallet
                    .generate_deposit_address(true)
                    .await?
                    .to_string(),
            }),
            ReceivePaymentMethod::Bolt11Invoice {
                description,
                amount_sats,
            } => Ok(ReceivePaymentResponse {
                payment_request: self
                    .spark_wallet
                    .create_lightning_invoice(
                        amount_sats.unwrap_or_default(),
                        Some(description.clone()),
                    )
                    .await?
                    .invoice,
            }),
        }
    }

    pub async fn prepare_send_payment(
        &self,
        request: PrepareSendPaymentRequest,
    ) -> Result<PrepareSendPaymentResponse, SdkError> {
        // First check for spark address
        if let Ok(spark_address) = request.payment_request.parse::<SparkAddress>() {
            return Ok(PrepareSendPaymentResponse {
                payment_method: SendPaymentMethod::SparkAddress {
                    address: spark_address.to_string(),
                },
                fee_sats: 0,
                amount_sats: request
                    .amount_sats
                    .ok_or(SdkError::InvalidInput("Amount is required".to_string()))?,
            });
        }
        // Then check for other types of inputs
        let parsed_input = parse(&request.payment_request).await?;
        match &parsed_input {
            breez_sdk_common::input::InputType::Bolt11Invoice(detailed_bolt11_invoice) => {
                let fee_estimation = self
                    .spark_wallet
                    .fetch_lightning_send_fee_estimate(
                        &request.payment_request,
                        request.amount_sats,
                    )
                    .await?;
                Ok(PrepareSendPaymentResponse {
                    payment_method: SendPaymentMethod::Bolt11Invoice {
                        detailed_invoice: detailed_bolt11_invoice.clone(),
                    },
                    fee_sats: fee_estimation,
                    amount_sats: request
                        .amount_sats
                        .or(detailed_bolt11_invoice.amount_msat.map(|msat| msat / 1000))
                        .ok_or(SdkError::InvalidInput("Amount is required".to_string()))?,
                })
            }
            breez_sdk_common::input::InputType::BitcoinAddress(_bitcoin_address) => todo!(),
            _ => Err(SdkError::GenericError("Unsupported input type".to_string())),
        }
    }

    pub async fn send_payment(
        &self,
        request: SendPaymentRequest,
    ) -> Result<SendPaymentResponse, SdkError> {
        match request.prepare_response.payment_method {
            SendPaymentMethod::SparkAddress { address } => {
                let spark_address = address
                    .parse::<SparkAddress>()
                    .map_err(|_| SdkError::InvalidInput("Invalid spark address".to_string()))?;
                let transfer = self
                    .spark_wallet
                    .transfer(request.prepare_response.amount_sats, &spark_address)
                    .await?;
                Ok(SendPaymentResponse {
                    payment: transfer.try_into()?,
                })
            }
            SendPaymentMethod::Bolt11Invoice { detailed_invoice } => {
                let amount_to_send = match detailed_invoice.amount_msat {
                    // we are not sending amount in case the invoice contains it.
                    Some(_) => None,
                    // We are sending amount for zero amount invoice
                    None => Some(request.prepare_response.amount_sats),
                };
                let payment_response = self
                    .spark_wallet
                    .pay_lightning_invoice(
                        &detailed_invoice.invoice.bolt11,
                        amount_to_send,
                        Some(request.prepare_response.fee_sats),
                        true,
                    )
                    .await?;
                let payment = match payment_response {
                    PayLightningInvoiceResult::LightningPayment(payment) => {
                        Payment::from_lightning(payment, request.prepare_response.amount_sats)?
                    }
                    PayLightningInvoiceResult::Transfer(payment) => payment.try_into()?,
                };
                Ok(SendPaymentResponse { payment })
            }
            SendPaymentMethod::BitcoinAddress { address: _ } => todo!(),
        }
    }

    /// Synchronizes the wallet with the Spark network
    pub async fn sync_wallet(
        &self,
        _request: SyncWalletRequest,
    ) -> Result<SyncWalletResponse, SdkError> {
        self.sync_wallet_internal().await?;
        Ok(SyncWalletResponse {})
    }

    async fn sync_wallet_internal(&self) -> Result<(), SdkError> {
        let start_time = Instant::now();

        // Sync with the Spark network
        self.spark_wallet.sync().await?;
        self.sync_payments_to_storage().await?;
        let elapsed = start_time.elapsed();
        trace!("Wallet sync completed in {elapsed:?}");
        self.event_emitter.emit(&SdkEvent::Synced {});
        Ok(())
    }

    /// Synchronizes payments from transfers to persistent storage
    async fn sync_payments_to_storage(&self) -> Result<(), SdkError> {
        //sync balance
        let balance = self.spark_wallet.get_balance().await?;
        let object_repository = ObjectCacheRepository::new(self.storage.clone());
        object_repository.save_account_info(CachedAccountInfo {
            balance_sats: balance,
        })?;

        // sync payments
        const BATCH_SIZE: u64 = 50;

        // Get the last offset we processed from storage
        let cached_sync_info = object_repository.fetch_sync_info()?.unwrap_or_default();
        let current_offset = cached_sync_info.offset;

        // We'll keep querying in batches until we have all transfers
        let mut next_offset = current_offset;
        let mut has_more = true;
        info!("Syncing payments to storage, offset = {next_offset}");
        let mut pending_payments = 0;
        while has_more {
            // Get batch of transfers starting from current offset
            let transfers_response = self
                .spark_wallet
                .list_transfers(Some(PagingFilter::new(
                    Some(next_offset),
                    Some(BATCH_SIZE),
                    Some(Order::Ascending),
                )))
                .await?;

            info!(
                "Syncing payments to storage, offset = {next_offset}, transfers = {}",
                transfers_response.len()
            );
            // Process transfers in this batch
            for transfer in &transfers_response {
                // Create a payment record
                let payment = transfer.clone().try_into()?;
                // Insert payment into storage
                if let Err(err) = self.storage.insert_payment(&payment) {
                    error!("Failed to insert payment: {err:?}");
                }
                if payment.status == PaymentStatus::Pending {
                    pending_payments += 1;
                }
                info!("Inserted payment: {payment:?}");
            }

            // Check if we have more transfers to fetch
            next_offset = next_offset.saturating_add(u64::try_from(transfers_response.len())?);
            // Update our last processed offset in the storage. We should remove pending payments
            // from the offset as they might be removed from the list later.
            let save_res = object_repository.save_sync_info(CachedSyncInfo {
                offset: next_offset - pending_payments,
            });

            if let Err(err) = save_res {
                error!("Failed to update last sync offset: {err:?}");
            }
            has_more = transfers_response.len() as u64 == BATCH_SIZE;
        }

        Ok(())
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
        let payments = self.storage.list_payments(request.offset, request.limit)?;
        Ok(ListPaymentsResponse { payments })
    }

    pub async fn get_payment(
        &self,
        request: GetPaymentRequest,
    ) -> Result<GetPaymentResponse, SdkError> {
        let payment = self.storage.get_payment_by_id(&request.payment_id)?;
        Ok(GetPaymentResponse { payment })
    }

    async fn check_and_claim_static_deposits(&self) -> Result<(), SdkError> {
        let addresses = self
            .spark_wallet
            .list_static_deposit_addresses(None)
            .await?;
        for address in addresses {
            info!("Checking static deposit address: {}", address.to_string());
            let utxos = self
                .spark_wallet
                .get_utxos_for_address(&address.to_string())
                .await;
            match utxos {
                Ok(utxos) => {
                    info!("Found {} utxos for address {}", utxos.len(), address);
                    for utxo in utxos {
                        info!("Processing utxo {}:{}", utxo.txid, utxo.vout);
                        match self
                            .claim_utxo(&utxo, self.config.max_deposit_claim_fee.clone())
                            .await
                        {
                            Ok(_) => info!("Claimed utxo {}:{}", utxo.txid, utxo.vout),
                            Err(e) => {
                                error!("Failed to claim utxo {}:{}: {e}", utxo.txid, utxo.vout)
                            }
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to get utxos for address {}: {e}", address);
                }
            }
        }

        Ok(())
    }

    async fn claim_utxo(&self, utxo: &Utxo, max_claim_fee: Option<Fee>) -> Result<(), SdkError> {
        info!("Claiming utxo {}:{}", utxo.txid, utxo.vout);
        let tx: Transaction = match utxo.tx.clone() {
            Some(tx) => tx,
            None => {
                let tx_hex = self
                    .chain_service
                    .get_transaction_hex(&utxo.txid.to_string())
                    .await?;
                deserialize_hex(tx_hex.as_str())?
            }
        };

        info!(
            "Fetching static deposit claim quote for utxo {}:{}",
            utxo.txid, utxo.vout
        );
        let utxo_value_sat = tx.output[utxo.vout as usize].value.to_sat();
        let quote = self
            .spark_wallet
            .fetch_static_deposit_claim_quote(tx.clone(), Some(utxo.vout))
            .await?;
        let spark_requested_fee = utxo_value_sat - quote.credit_amount_sats;
        if let Some(max_deposit_claim_fee) = max_claim_fee {
            match max_deposit_claim_fee {
                Fee::Fixed { amount } => {
                    if spark_requested_fee > amount {
                        return Err(SdkError::DepositClaimFeeExceeds(
                            utxo.txid.to_string(),
                            utxo.vout,
                            max_deposit_claim_fee,
                            spark_requested_fee,
                        ));
                    }
                }
                Fee::Rate { sat_per_vbyte } => {
                    let vsize: u64 = tx.vsize().try_into()?;
                    let user_max_fee = vsize * sat_per_vbyte;
                    if spark_requested_fee > user_max_fee {
                        return Err(SdkError::DepositClaimFeeExceeds(
                            utxo.txid.to_string(),
                            utxo.vout,
                            max_deposit_claim_fee,
                            spark_requested_fee,
                        ));
                    }
                }
            }
        }
        info!(
            "Claiming static deposit for utxo {}:{}",
            utxo.txid, utxo.vout
        );
        let transfer = self.spark_wallet.claim_static_deposit(quote).await?;
        info!(
            "Claimed static deposit transfer: {}",
            serde_json::to_string_pretty(&transfer)?
        );
        Ok(())
    }
}
