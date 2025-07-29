pub mod error;
pub mod events;
pub mod models;
pub mod persist;
pub mod sdk_builder;

use breez_sdk_common::input::parse;
use log::{error, info, trace};
use models::Config;
use spark_wallet::{
    DefaultSigner, PagingFilter, PayLightningInvoiceResult, SparkAddress, SparkWallet, WalletEvent,
};
use std::{sync::Arc, time::Instant};

// Export the persist module for external use
pub use persist::{SqliteStorage, Storage};
// Export events module for external use
pub use events::{EventEmitter, EventListener, SdkEvent};

use tokio::sync::watch;

// Export the builder module
pub use sdk_builder::SdkBuilder;

use crate::{
    error::SdkError,
    models::{
        GetInfoRequest, GetInfoResponse, ListPaymentsRequest, ListPaymentsResponse, Payment,
        PrepareReceivePaymentRequest, PrepareReceivePaymentResponse, PrepareSendPaymentRequest,
        PrepareSendPaymentResponse, ReceivePaymentMethod, ReceivePaymentRequest,
        ReceivePaymentResponse, SendPaymentMethod, SendPaymentRequest, SendPaymentResponse,
        SyncWalletRequest, SyncWalletResponse,
    },
    persist::{CachedAccountInfo, CachedSyncInfo},
};

/// `BreezSDK` is a wrapper around `SparkSDK` that provides a more structured API
/// with request/response objects and comprehensive error handling.
#[derive(Clone)]
pub struct BreezSdk {
    spark_wallet: Arc<SparkWallet<DefaultSigner>>,
    storage: Arc<dyn Storage>,
    event_emitter: Arc<EventEmitter>,
    shutdown_sender: watch::Sender<()>,
    shutdown_receiver: watch::Receiver<()>,
}

// Modify the connect function to use the builder pattern
pub async fn connect(config: Config) -> Result<BreezSdk, SdkError> {
    let sdk = SdkBuilder::new(config).build().await?;
    sdk.start()?;
    Ok(sdk)
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
        shutdown_sender: watch::Sender<()>,
        shutdown_receiver: watch::Receiver<()>,
    ) -> Result<Self, SdkError> {
        let spark_wallet_config =
            spark_wallet::SparkWalletConfig::default_config(config.network.into());
        let spark_wallet = SparkWallet::connect(spark_wallet_config, signer).await?;

        let sdk = Self {
            spark_wallet: Arc::new(spark_wallet),
            storage,
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

    /// Starts the SDK's background tasks
    ///
    /// This method initiates the following backround tasks:
    /// 1. `periodic_sync`: the wallet with the Spark network    
    ///
    pub fn start(&self) -> Result<(), SdkError> {
        self.periodic_sync();
        Ok(())
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
    pub fn stop(&self) -> Result<(), SdkError> {
        self.shutdown_sender
            .send(())
            .map_err(|_| SdkError::GenericError("Failed to send shutdown signal".to_string()))?;

        Ok(())
    }

    /// Returns the balance of the wallet in satoshis
    pub async fn get_info(&self, _request: GetInfoRequest) -> Result<GetInfoResponse, SdkError> {
        let account_info = CachedAccountInfo::fetch(self.storage.as_ref())?;
        Ok(GetInfoResponse {
            balance_sats: account_info.balance_sats,
        })
    }

    pub async fn prepare_receive_payment(
        &self,
        request: PrepareReceivePaymentRequest,
    ) -> Result<PrepareReceivePaymentResponse, SdkError> {
        match &request.payment_method {
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
            ReceivePaymentMethod::BitcoinAddress => Ok(ReceivePaymentResponse {
                // TODO: allow passing amount
                payment_identifier: self
                    .spark_wallet
                    .generate_deposit_address(true)
                    .await?
                    .to_qr_uri(),
            }),
            ReceivePaymentMethod::Bolt11Invoice {
                description,
                amount_sats,
            } => Ok(ReceivePaymentResponse {
                payment_identifier: self
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
        if let Ok(spark_address) = request.payment_identifier.parse::<SparkAddress>() {
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
        let parsed_input = parse(&request.payment_identifier).await?;
        match &parsed_input {
            breez_sdk_common::input::InputType::Bolt11Invoice(detailed_bolt11_invoice) => {
                let fee_estimation = self
                    .spark_wallet
                    .fetch_lightning_send_fee_estimate(
                        &request.payment_identifier,
                        request.amount_sats,
                    )
                    .await?;
                Ok(PrepareSendPaymentResponse {
                    payment_method: SendPaymentMethod::Bolt11Invoice {
                        raw_invoice: request.payment_identifier.clone(),
                        invoice: detailed_bolt11_invoice.clone(),
                    },
                    fee_sats: fee_estimation,
                    amount_sats: request
                        .amount_sats
                        .or(detailed_bolt11_invoice.amount_msat)
                        .ok_or(SdkError::InvalidInput("Amount is required".to_string()))?,
                })
            }
            breez_sdk_common::input::InputType::BitcoinAddress(bitcoin_address) => todo!(),
            breez_sdk_common::input::InputType::Bip21(bip21) => todo!(),
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
                    payment: transfer.into(),
                })
            }
            SendPaymentMethod::Bolt11Invoice {
                raw_invoice,
                invoice,
            } => {
                let payment_response = self
                    .spark_wallet
                    .pay_lightning_invoice(
                        &raw_invoice,
                        Some(request.prepare_response.amount_sats),
                        Some(request.prepare_response.fee_sats),
                        true,
                    )
                    .await?;
                let payment = match payment_response {
                    PayLightningInvoiceResult::LightningPayment(payment) => {
                        Payment::from_lightning(payment, request.prepare_response.amount_sats)
                    }
                    PayLightningInvoiceResult::Transfer(payment) => payment.into(),
                };
                Ok(SendPaymentResponse { payment })
            }
            SendPaymentMethod::BitcoinAddress { address } => todo!(),
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
        const BATCH_SIZE: u64 = 50;

        // Get the last offset we processed from storage
        let cached_sync_info = CachedSyncInfo::fetch(self.storage.as_ref())?;
        let current_offset = cached_sync_info.offset;

        // We'll keep querying in batches until we have all transfers
        let mut next_offset = current_offset;
        let mut has_more = true;
        while has_more {
            // Get batch of transfers starting from current offset
            let transfers_response = self
                .spark_wallet
                .list_transfers(Some(PagingFilter::new(Some(next_offset), Some(BATCH_SIZE))))
                .await?;

            // Process transfers in this batch
            for transfer in &transfers_response {
                // Create a payment record
                let payment = transfer.clone().into();
                // Insert payment into storage
                if let Err(err) = self.storage.insert_payment(&payment) {
                    error!("Failed to insert payment: {err:?}");
                }
            }

            // Check if we have more transfers to fetch
            next_offset = next_offset.saturating_add(u64::try_from(transfers_response.len())?);
            // Update our last processed offset in the storage
            let save_res = CachedSyncInfo {
                offset: next_offset,
            }
            .save(self.storage.as_ref());

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
    pub fn list_payments(
        &self,
        request: &ListPaymentsRequest,
    ) -> Result<ListPaymentsResponse, SdkError> {
        let payments = self.storage.list_payments(request.offset, request.limit)?;
        Ok(ListPaymentsResponse { payments })
    }
}
