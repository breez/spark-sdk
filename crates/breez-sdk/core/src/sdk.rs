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
    DefaultSigner, ExitSpeed, Order, PagingFilter, PayLightningInvoiceResult, SparkAddress,
    SparkWallet, WalletEvent, WalletTransfer,
};
use std::{path::PathBuf, str::FromStr, sync::Arc};
use tracing::{error, info};
use web_time::{Duration, SystemTime};

use tokio::sync::watch;
use tokio_with_wasm::alias as tokio;
use web_time::Instant;

use crate::{
    BitcoinChainService, ClaimDepositRequest, ClaimDepositResponse, DepositInfo, Fee,
    GetPaymentRequest, GetPaymentResponse, ListUnclaimedDepositsRequest,
    ListUnclaimedDepositsResponse, LnurlPayInfo, LnurlPayRequest, LnurlPayResponse, Logger,
    Network, PaymentDetails, PaymentStatus, PrepareLnurlPayRequest, PrepareLnurlPayResponse,
    RefundDepositRequest, RefundDepositResponse, SendPaymentOptions, SqliteStorage,
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
    persist::{
        CachedAccountInfo, CachedSyncInfo, ObjectCacheRepository, PaymentMetadata, Storage,
        UpdateDepositPayload,
    },
    utils::{
        deposit_chain_syncer::DepositChainSyncer,
        utxo_fetcher::{CachedUtxoFetcher, DetailedUtxo},
    },
};

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
    event_emitter: breez_sdk_common::utils::Arc<EventEmitter>,
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

#[cfg_attr(feature = "uniffi", uniffi::export)]
#[allow(clippy::needless_pass_by_value)]
pub fn default_storage(data_dir: String) -> Result<Arc<dyn Storage>, SdkError> {
    let db_path = PathBuf::from_str(&data_dir)?;

    let storage = SqliteStorage::new(&db_path)?;
    Ok(Arc::new(storage))
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn default_config(network: Network) -> Config {
    Config {
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

        let sdk = Self {
            config,
            spark_wallet: Arc::new(spark_wallet),
            storage,
            chain_service,
            lnurl_client,
            event_emitter: breez_sdk_common::utils::Arc::new(EventEmitter::new()),
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
        if let Err(e) = self.sync_trigger.send(SyncType::Full) {
            error!("Failed to execute initial sync: {e:?}");
        }
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
                                info!("Received event: {event:?}");
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
                if let Err(e) = self.sync_trigger.send(SyncType::Full) {
                    error!("Failed to sync wallet: {e:?}");
                }
            }
            WalletEvent::StreamDisconnected => {
                info!("Stream disconnected");
            }
            WalletEvent::Synced => {
                info!("Synced");
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
            self.spark_wallet.sync().await?;
        }
        self.sync_payments_to_storage().await?;
        self.check_and_claim_static_deposits().await?;
        let elapsed = start_time.elapsed();
        info!("Wallet sync completed in {elapsed:?}");
        self.event_emitter.emit(&SdkEvent::Synced {});
        Ok(())
    }

    /// Synchronizes payments from transfers to persistent storage
    async fn sync_payments_to_storage(&self) -> Result<(), SdkError> {
        const BATCH_SIZE: u64 = 50;

        // Sync balance
        let balance = self.spark_wallet.get_balance().await?;
        let object_repository = ObjectCacheRepository::new(self.storage.clone());
        object_repository.save_account_info(&CachedAccountInfo {
            balance_sats: balance,
        })?;

        // Get the last offset we processed from storage
        let cached_sync_info = object_repository.fetch_sync_info()?.unwrap_or_default();
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
                let payment: Payment = transfer.clone().try_into()?;
                // Insert payment into storage
                if let Err(err) = self.storage.insert_payment(payment.clone()) {
                    error!("Failed to insert payment: {err:?}");
                }
                if payment.status == PaymentStatus::Pending {
                    pending_payments = pending_payments.saturating_add(1);
                }
                info!("Inserted payment: {payment:?}");
            }

            // Check if we have more transfers to fetch
            next_offset = next_offset.saturating_add(u64::try_from(transfers_response.len())?);
            // Update our last processed offset in the storage. We should remove pending payments
            // from the offset as they might be removed from the list later.
            let save_res = object_repository.save_sync_info(&CachedSyncInfo {
                offset: next_offset.saturating_sub(pending_payments),
            });

            if let Err(err) = save_res {
                error!("Failed to update last sync offset: {err:?}");
            }
            has_more = transfers_response.len() as u64 == BATCH_SIZE;
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
                        .delete_deposit(detailed_utxo.txid.to_string(), detailed_utxo.vout)?;
                    claimed_deposits.push(detailed_utxo.into());
                }
                Err(e) => {
                    error!(
                        "Failed to claim utxo {}:{}: {e}",
                        detailed_utxo.txid, detailed_utxo.vout
                    );
                    self.storage.update_deposit(
                        detailed_utxo.txid.to_string(),
                        detailed_utxo.vout,
                        UpdateDepositPayload::ClaimError {
                            error: e.clone().into(),
                        },
                    )?;
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
    pub fn get_info(&self, request: GetInfoRequest) -> Result<GetInfoResponse, SdkError> {
        let object_repository = ObjectCacheRepository::new(self.storage.clone());
        let account_info = object_repository.fetch_account_info()?.unwrap_or_default();
        Ok(GetInfoResponse {
            balance_sats: account_info.balance_sats,
        })
    }

    pub fn prepare_receive_payment(
        &self,
        request: PrepareReceivePaymentRequest,
    ) -> Result<PrepareReceivePaymentResponse, SdkError> {
        match &request.payment_method {
            ReceivePaymentMethod::Bolt11Invoice { .. } | ReceivePaymentMethod::SparkAddress => {
                Ok(PrepareReceivePaymentResponse {
                    payment_method: request.payment_method,
                    fee_sats: 0,
                })
            }
            #[allow(clippy::match_same_arms)]
            ReceivePaymentMethod::BitcoinAddress => {
                Ok(PrepareReceivePaymentResponse {
                    payment_method: request.payment_method,
                    fee_sats: 0, // TODO: calculate fee
                })
            }
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
                amount_sats: Some(request.amount_sats),
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
                        amount_sats: request.prepare_response.amount_sats,
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

        self.storage.set_payment_metadata(
            payment.id.clone(),
            PaymentMetadata {
                lnurl_pay_info: Some(lnurl_info),
            },
        )?;
        self.event_emitter.emit(&SdkEvent::PaymentSucceeded {
            payment: payment.clone(),
        });
        Ok(LnurlPayResponse {
            payment,
            success_action,
        })
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
                    fee_sats: 0,
                },
                amount_sats: request
                    .amount_sats
                    .ok_or(SdkError::InvalidInput("Amount is required".to_string()))?,
            });
        }
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
                        request.amount_sats,
                    )
                    .await?;

                Ok(PrepareSendPaymentResponse {
                    payment_method: SendPaymentMethod::Bolt11Invoice {
                        invoice_details: detailed_bolt11_invoice.clone(),
                        spark_transfer_fee_sats,
                        lightning_fee_sats,
                    },
                    amount_sats: request
                        .amount_sats
                        .or(detailed_bolt11_invoice.amount_msat.map(|msat| msat / 1000))
                        .ok_or(SdkError::InvalidInput("Amount is required".to_string()))?,
                })
            }
            InputType::BitcoinAddress(withdrawal_address) => {
                let fee_quote = self
                    .spark_wallet
                    .fetch_coop_exit_fee_quote(
                        &withdrawal_address.address,
                        Some(
                            request
                                .amount_sats
                                .ok_or(SdkError::InvalidInput("Amount is required".to_string()))?,
                        ),
                    )
                    .await?;
                Ok(PrepareSendPaymentResponse {
                    payment_method: SendPaymentMethod::BitcoinAddress {
                        address: withdrawal_address.clone(),
                        fee_quote: fee_quote.into(),
                    },
                    amount_sats: request
                        .amount_sats
                        .ok_or(SdkError::InvalidInput("Amount is required".to_string()))?,
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

    async fn send_payment_internal(
        &self,
        request: SendPaymentRequest,
        suppress_payment_event: bool,
    ) -> Result<SendPaymentResponse, SdkError> {
        let res = match request.prepare_response.payment_method {
            SendPaymentMethod::SparkAddress { address, .. } => {
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
            SendPaymentMethod::Bolt11Invoice {
                invoice_details,
                spark_transfer_fee_sats,
                lightning_fee_sats,
            } => {
                let amount_to_send = match invoice_details.amount_msat {
                    // we are not sending amount in case the invoice contains it.
                    Some(_) => None,
                    // We are sending amount for zero amount invoice
                    None => Some(request.prepare_response.amount_sats),
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
                        amount_to_send,
                        Some(fee_sats),
                        use_spark,
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
                        Some(request.prepare_response.amount_sats),
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
            //self.storage.insert_payment(response.payment.clone())?;
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
    pub fn list_payments(
        &self,
        request: ListPaymentsRequest,
    ) -> Result<ListPaymentsResponse, SdkError> {
        let payments = self.storage.list_payments(request.offset, request.limit)?;
        Ok(ListPaymentsResponse { payments })
    }

    pub fn get_payment(&self, request: GetPaymentRequest) -> Result<GetPaymentResponse, SdkError> {
        let payment = self.storage.get_payment_by_id(request.payment_id)?;
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
                    .delete_deposit(detailed_utxo.txid.to_string(), detailed_utxo.vout)?;
                if let Err(e) = self.sync_trigger.send(SyncType::PaymentsOnly) {
                    error!("Failed to execute sync after deposit claim: {e:?}");
                }
                Ok(ClaimDepositResponse {
                    payment: transfer.try_into()?,
                })
            }
            Err(e) => {
                error!("Failed to claim deposit: {e:?}");
                self.storage.update_deposit(
                    detailed_utxo.txid.to_string(),
                    detailed_utxo.vout,
                    UpdateDepositPayload::ClaimError {
                        error: e.clone().into(),
                    },
                )?;
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
        self.storage.update_deposit(
            deposit.txid.clone(),
            deposit.vout,
            UpdateDepositPayload::Refund {
                refund_tx: tx_hex.clone(),
                refund_txid: tx_id.clone(),
            },
        )?;

        self.chain_service
            .broadcast_transaction(tx_hex.clone())
            .await?;
        Ok(RefundDepositResponse { tx_id, tx_hex })
    }

    #[allow(unused_variables)]
    pub fn list_unclaimed_deposits(
        &self,
        request: ListUnclaimedDepositsRequest,
    ) -> Result<ListUnclaimedDepositsResponse, SdkError> {
        let deposits = self.storage.list_deposits()?;
        Ok(ListUnclaimedDepositsResponse { deposits })
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
