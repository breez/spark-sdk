use spark_wallet::WalletEvent;
use std::sync::Arc;
use tokio::sync::{oneshot, watch};
use tokio_with_wasm::alias as tokio;
use tracing::{debug, error, info, trace, warn};
use web_time::{Duration, Instant, SystemTime};

use crate::{
    DepositInfo, InputType, MaxFee, PaymentDetails, PaymentType,
    error::SdkError,
    events::{InternalSyncedEvent, SdkEvent},
    lnurl::ListMetadataRequest,
    models::{Payment, SyncWalletRequest, SyncWalletResponse},
    persist::{ObjectCacheRepository, UpdateDepositPayload},
    sync::SparkSyncService,
    utils::{
        deposit_chain_syncer::DepositChainSyncer, run_with_shutdown, utxo_fetcher::DetailedUtxo,
    },
};

use super::{
    BreezSdk, CLAIM_TX_SIZE_VBYTES, SYNC_PAGING_LIMIT, SyncRequest, SyncType,
    helpers::{BalanceWatcher, update_balances},
    parse_input,
};

impl BreezSdk {
    pub(super) fn periodic_sync(&self, initial_synced_sender: watch::Sender<bool>) {
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
                        let Ok(sync_request) = sync_type_res else {
                            continue;
                        };
                        info!("Sync trigger changed: {:?}", &sync_request);
                        let cloned_sdk = sdk.clone();
                        let initial_synced_sender = initial_synced_sender.clone();
                        if let Some(true) = Box::pin(run_with_shutdown(shutdown_receiver.clone(), "Sync trigger changed", async move {
                            if let Err(e) = cloned_sdk.sync_wallet_internal(&sync_request).await {
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
                            && let Err(e) = sync_trigger_sender.send(SyncRequest::periodic()) {
                            error!("Failed to trigger periodic sync: {e:?}");
                        }
                    }
                }
            }
        });
    }

    pub(super) async fn handle_wallet_event(&self, event: WalletEvent) {
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
                if let Ok(mut payment) = Payment::try_from(transfer) {
                    // Insert the payment into storage to make it immediately available for listing
                    if let Err(e) = self.storage.insert_payment(payment.clone()).await {
                        error!("Failed to insert succeeded payment: {e:?}");
                    }

                    // Ensure potential lnurl metadata is synced before emitting the event.
                    // Note this is already synced at TransferClaimStarting, but it might not have completed yet, so that could race.
                    self.sync_single_lnurl_metadata(&mut payment).await;

                    self.event_emitter
                        .emit(&SdkEvent::PaymentSucceeded { payment })
                        .await;
                }
                if let Err(e) = self
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
                    if let Err(e) = self.storage.insert_payment(payment.clone()).await {
                        error!("Failed to insert pending payment: {e:?}");
                    }

                    // Ensure potential lnurl metadata is synced before emitting the event
                    self.sync_single_lnurl_metadata(&mut payment).await;

                    self.event_emitter
                        .emit(&SdkEvent::PaymentPending { payment })
                        .await;
                }
                if let Err(e) = self
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

    pub(super) async fn sync_single_lnurl_metadata(&self, payment: &mut Payment) {
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

        // Let's check whether the lnurl receive metadata was already synced, then return early.
        // Important: Only return early if metadata is actually present (Some), otherwise we need
        // to trigger a sync. This prevents a race condition where the payment is in storage but
        // metadata sync from TransferClaimStarting hasn't completed yet.
        if let Ok(db_payment) = self.storage.get_payment_by_id(payment.id.clone()).await
            && let Some(PaymentDetails::Lightning {
                lnurl_receive_metadata: db_lnurl_receive_metadata @ Some(_),
                ..
            }) = db_payment.details
        {
            *lnurl_receive_metadata = db_lnurl_receive_metadata;
            return;
        }

        // Sync lnurl metadata directly instead of going through the sync trigger,
        // because this function is called from the sync loop's event handler,
        // which would deadlock waiting for itself to process the trigger.
        if let Err(e) = self.sync_lnurl_metadata().await {
            error!("Failed to sync lnurl metadata for invoice {invoice}: {e}");
            return;
        }

        let db_payment = match self.storage.get_payment_by_id(payment.id.clone()).await {
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
    pub(super) async fn sync_wallet_internal(&self, request: &SyncRequest) -> Result<(), SdkError> {
        let cache = ObjectCacheRepository::new(self.storage.clone());
        let sync_interval_secs = u64::from(self.config.sync_interval_secs);

        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // Skip if we synced recently (unless forced).
        if !request.force
            && let Some(last) = cache.get_last_sync_time().await?
            && now.saturating_sub(last) < sync_interval_secs
        {
            debug!("sync_wallet_internal: Synced recently, skipping");
            return Ok(());
        }

        // Update last sync time if this is a full sync.
        if request.sync_type.contains(SyncType::Full)
            && let Err(e) = cache.set_last_sync_time(now).await
        {
            error!("sync_wallet_internal: Failed to update last sync time: {e:?}");
        }

        let start_time = Instant::now();

        let sync_wallet = async {
            let wallet_synced = if request.sync_type.contains(SyncType::Wallet) {
                debug!("sync_wallet_internal: Starting Wallet sync");
                let wallet_start = Instant::now();
                match self.spark_wallet.sync().await {
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

            let wallet_state_synced = if request.sync_type.contains(SyncType::WalletState) {
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
            if request.sync_type.contains(SyncType::LnurlMetadata) {
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
            if request.sync_type.contains(SyncType::Deposits) {
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

        // Trigger auto-conversion after sync
        if wallet_state && let Some(stable_balance) = &self.stable_balance {
            stable_balance.trigger_auto_convert();
        }

        let elapsed = start_time.elapsed();
        let event = InternalSyncedEvent {
            wallet,
            wallet_state,
            lnurl_metadata,
            deposits,
            storage_incoming: None,
        };
        info!("sync_wallet_internal: Wallet sync completed in {elapsed:?}: {event:?}");
        self.event_emitter.emit_synced(&event).await;
        Ok(())
    }

    /// Synchronizes wallet state to persistent storage, making sure we have the latest balances and payments.
    pub(super) async fn sync_wallet_state_to_storage(&self) -> Result<(), SdkError> {
        update_balances(self.spark_wallet.clone(), self.storage.clone()).await?;

        let initial_sync_complete = *self.initial_synced_watcher.borrow();
        let sync_service = SparkSyncService::new(
            self.spark_wallet.clone(),
            self.storage.clone(),
            self.event_emitter.clone(),
        );
        sync_service.sync_payments(initial_sync_complete).await?;

        Ok(())
    }

    pub(super) async fn check_and_claim_static_deposits(&self) -> Result<(), SdkError> {
        self.ensure_spark_private_mode_initialized().await?;
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

    pub(super) async fn sync_lnurl_metadata(&self) -> Result<(), SdkError> {
        let Some(lnurl_server_client) = self.lnurl_server_client.clone() else {
            return Ok(());
        };

        let cache = ObjectCacheRepository::new(Arc::clone(&self.storage));
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
            self.storage
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

            let _ = self.zap_receipt_trigger.send(());
            if len < SYNC_PAGING_LIMIT {
                // No more invoices to fetch
                break;
            }
        }

        Ok(())
    }

    pub(super) async fn claim_utxo(
        &self,
        detailed_utxo: &DetailedUtxo,
        max_claim_fee: Option<MaxFee>,
    ) -> Result<spark_wallet::WalletTransfer, SdkError> {
        info!(
            "Fetching static deposit claim quote for deposit tx {}:{} and amount: {}",
            detailed_utxo.txid, detailed_utxo.vout, detailed_utxo.value
        );
        let quote = self
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
            .to_fee(self.chain_service.as_ref())
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
        let transfer = self.spark_wallet.claim_static_deposit(quote).await?;
        info!(
            "Claimed static deposit transfer for utxo {}:{}, value {}",
            detailed_utxo.txid, detailed_utxo.vout, transfer.total_value_sat,
        );
        Ok(transfer)
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
#[allow(clippy::needless_pass_by_value)]
impl BreezSdk {
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
}
