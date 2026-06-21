use std::sync::Arc;

use platform_utils::time::{Duration, SystemTime};
use platform_utils::tokio;
use spark_wallet::{SparkWallet, WalletEvent, WalletTransfer};
use tokio::{
    select,
    sync::{broadcast, watch},
};
use tracing::{Instrument, debug, error, info, trace};

use crate::utils::token::{token_transaction_to_payments, token_tx_inputs_are_ours};
use crate::{
    GetInfoRequest, GetInfoResponse, Payment,
    error::SdkError,
    events::{EventListener, SdkEvent},
    persist::ObjectCacheRepository,
    token_conversion::TokenConverter,
    utils::{
        payments::{get_payment_and_emit_event, update_balances},
        run_with_shutdown,
    },
};
use crate::{PaymentType, StorageListPaymentsRequest, StoragePaymentDetailsFilter};

use super::{RuntimeEvent, RuntimeProfile};
use crate::sdk::{BreezSdk, SyncCoordinator, SyncRequest, SyncType, helpers::BalanceWatcher};

pub(super) struct ClientRuntime;

#[macros::async_trait]
impl RuntimeProfile for ClientRuntime {
    fn starts_background_services(&self) -> bool {
        true
    }

    async fn start_sdk_services(&self, sdk: &BreezSdk, initial_synced_sender: watch::Sender<bool>) {
        register_client_sync_listener(sdk).await;
        register_client_runtime_event_handler(sdk).await;
        sdk.spawn_spark_private_mode_initialization();
        spawn_client_runtime_loop(sdk, initial_synced_sender);

        // Subscribers are now attached: start the wallet's BackgroundProcessor so
        // its first `WalletEvent::Synced` (emitted after the operator stream
        // connects) lands in the runtime loop and drives the initial Full sync.
        sdk.spark_wallet.start_background_processing().await;

        sdk.try_recover_lightning_address();
        spawn_conversion_refunder(
            Arc::clone(&sdk.token_converter),
            sdk.shutdown_sender.subscribe(),
        );
        if let Some(stable_balance) = &sdk.stable_balance {
            stable_balance.spawn_conversion_worker(sdk.shutdown_sender.subscribe());
        }
    }

    async fn run_user_sync(
        &self,
        sdk: &BreezSdk,
        sync_type: SyncType,
        force: bool,
    ) -> Result<(), SdkError> {
        sdk.sync_coordinator
            .trigger_sync_and_wait(sync_type, force)
            .await
    }

    async fn get_info(
        &self,
        sdk: &BreezSdk,
        request: GetInfoRequest,
    ) -> Result<GetInfoResponse, SdkError> {
        if request.ensure_synced.unwrap_or_default() {
            sdk.initial_synced_watcher
                .clone()
                .changed()
                .await
                .map_err(|_| {
                    SdkError::Generic("Failed to receive initial synced signal".to_string())
                })?;
        }

        let account_info = ObjectCacheRepository::new(sdk.storage.clone())
            .fetch_account_info()
            .await?
            .unwrap_or_default();

        Ok(GetInfoResponse {
            identity_pubkey: sdk.spark_wallet.get_identity_public_key().to_string(),
            balance_sats: account_info.balance_sats,
            token_balances: account_info.token_balances,
        })
    }

    async fn maybe_ensure_spark_private_mode_initialized(
        &self,
        sdk: &BreezSdk,
    ) -> Result<(), SdkError> {
        sdk.ensure_spark_private_mode_initialized_inner().await
    }
}

fn spawn_client_runtime_loop(sdk: &BreezSdk, initial_synced_sender: watch::Sender<bool>) {
    let sdk = sdk.clone();
    let mut shutdown_receiver = sdk.shutdown_sender.subscribe();
    let mut wallet_events = sdk.spark_wallet.subscribe_events();
    let mut sync_requests = sdk.sync_coordinator.subscribe();
    let mut last_sync_time = SystemTime::now();
    let sync_interval = u64::from(sdk.config.sync_interval_secs);
    let span = tracing::Span::current();

    tokio::spawn(
        async move {
            let balance_watcher =
                BalanceWatcher::new(sdk.spark_wallet.clone(), sdk.storage.clone());
            let balance_watcher_id = sdk.add_event_listener(Box::new(balance_watcher)).await;

            loop {
                select! {
                    _ = shutdown_receiver.changed() => {
                        if !sdk.remove_event_listener(&balance_watcher_id).await {
                            error!("Failed to remove balance watcher listener");
                        }
                        info!("Client runtime loop shutdown signal received");
                        return;
                    }

                    event = wallet_events.recv() => {
                        Box::pin(on_wallet_event(&sdk, event)).await;
                    }

                    sync_request = sync_requests.recv() => {
                        if on_sync_request(
                            &sdk,
                            sync_request,
                            &shutdown_receiver,
                            &initial_synced_sender,
                        )
                        .await
                        {
                            last_sync_time = SystemTime::now();
                        }
                    }

                    () = tokio::time::sleep(Duration::from_secs(10)) => {
                        let now = SystemTime::now();
                        if let Ok(elapsed) = now.duration_since(last_sync_time) && elapsed.as_secs() >= sync_interval {
                            sdk.sync_coordinator.trigger_sync_no_wait(SyncType::Full, false).await;
                        }
                    }
                }
            }
        }
        .instrument(span),
    );
}

async fn on_wallet_event(sdk: &BreezSdk, event: Result<WalletEvent, broadcast::error::RecvError>) {
    match event {
        Ok(event) => {
            info!("Received event: {event}");
            trace!("Received event: {:?}", event);
            let wallet_synced = matches!(&event, WalletEvent::Synced);
            let transfer_claim_event = matches!(
                &event,
                WalletEvent::TransferClaimed(_) | WalletEvent::TransferClaimStarting(_)
            );
            let payment_event_emitted = Box::pin(handle_wallet_event(sdk, event)).await;

            if wallet_synced {
                sdk.sync_coordinator
                    .trigger_sync_no_wait(SyncType::Full, true)
                    .await;
            } else if transfer_claim_event && !payment_event_emitted {
                sdk.sync_coordinator
                    .trigger_sync_no_wait(SyncType::WalletState, true)
                    .await;
            }
        }
        Err(e) => {
            error!("Failed to receive event: {e:?}");
        }
    }
}

async fn register_client_runtime_event_handler(sdk: &BreezSdk) {
    sdk.event_emitter
        .add_runtime_event_handler(Box::new(ClientRuntimeEventHandler {
            sync_coordinator: sdk.sync_coordinator.clone(),
            spark_wallet: sdk.spark_wallet.clone(),
            storage: sdk.storage.clone(),
        }))
        .await;
}

struct ClientRuntimeEventHandler {
    sync_coordinator: SyncCoordinator,
    spark_wallet: Arc<SparkWallet>,
    storage: Arc<dyn crate::Storage>,
}

#[macros::async_trait]
impl crate::events::RuntimeEventHandler for ClientRuntimeEventHandler {
    async fn handle(&self, _emitter: &crate::EventEmitter, event: RuntimeEvent) {
        match event {
            RuntimeEvent::StableBalanceConversionCompleted => {
                self.sync_coordinator
                    .trigger_sync_no_wait(SyncType::Full, true)
                    .await;
            }
            RuntimeEvent::DepositClaimed { .. } => {
                if let Err(e) =
                    update_balances(self.spark_wallet.clone(), self.storage.clone()).await
                {
                    error!("Failed to refresh balances after claim_deposit: {e:?}");
                }
            }
        }
    }
}

async fn on_sync_request(
    sdk: &BreezSdk,
    sync_request: Result<SyncRequest, broadcast::error::RecvError>,
    shutdown_receiver: &watch::Receiver<()>,
    initial_synced_sender: &watch::Sender<bool>,
) -> bool {
    let Ok(sync_request) = sync_request else {
        return false;
    };
    info!("Sync trigger changed: {:?}", &sync_request);
    let cloned_sdk = sdk.clone();
    let initial_synced_sender = initial_synced_sender.clone();
    matches!(
        Box::pin(run_with_shutdown(
            shutdown_receiver.clone(),
            "Sync trigger changed",
            async move {
                if let Err(e) = cloned_sdk
                    .sync_wallet_internal(sync_request.sync_type.clone(), sync_request.force)
                    .await
                {
                    error!("Failed to sync wallet: {e:?}");
                    let () = sync_request.reply(Some(e)).await;
                    return false;
                }
                let () = sync_request.reply(None).await;
                if sync_request.sync_type.contains(SyncType::Full) {
                    if let Err(e) = initial_synced_sender.send(true) {
                        error!("Failed to send initial synced signal: {e:?}");
                    }
                    return true;
                }
                false
            }
        ))
        .await,
        Some(true)
    )
}

async fn handle_wallet_event(sdk: &BreezSdk, event: WalletEvent) -> bool {
    match event {
        WalletEvent::DepositConfirmed(_) => {
            info!("Deposit confirmed");
            false
        }
        WalletEvent::StreamConnected => {
            info!("Stream connected");
            false
        }
        WalletEvent::StreamDisconnected => {
            info!("Stream disconnected");
            false
        }
        WalletEvent::Synced => {
            info!("Synced");
            false
        }
        WalletEvent::TransferClaimed(transfer) => {
            info!("Transfer claimed");
            // Drop any unclaimed-deposit record for this outpoint independently
            // of payment ingestion, so conversion failures do not leave it stale.
            if let Some((tx_id, vout)) = claim_static_deposit_outpoint(&transfer) {
                cleanup_claimed_deposit(sdk, &tx_id, vout).await;
            }
            if let Ok(payment) = Payment::try_from(transfer) {
                sdk.finalize_payment(payment).await
            } else {
                false
            }
        }
        WalletEvent::TransferClaimStarting(transfer) => {
            info!("Transfer claim starting");
            let mut payment_emitted = false;
            if let Ok(mut payment) = Payment::try_from(transfer) {
                // Persist before syncing metadata so the Pending payment is not
                // delayed by the metadata fetch.
                let should_emit = match sdk.storage.apply_payment_update(payment.clone()).await {
                    Ok(should_emit) => should_emit,
                    Err(e) => {
                        error!("Failed to apply pending payment update: {e:?}");
                        return false;
                    }
                };

                sdk.sync_single_lnurl_metadata(&mut payment).await;

                // Drop this Pending event if sync already saw the transfer Completed.
                if should_emit {
                    get_payment_and_emit_event(&sdk.storage, &sdk.event_emitter, payment).await;
                    payment_emitted = true;
                }
            }
            payment_emitted
        }
        WalletEvent::TokenTransaction(transaction) => {
            info!("Token transaction event: {}", transaction.hash);
            process_token_transaction_event(sdk, transaction)
                .await
                .unwrap_or_else(|e| {
                    error!("Failed to process token transaction event: {e:?}");
                    false
                })
        }
        WalletEvent::AutoOptimization(event) => {
            info!("AutoOptimization event: {:?}", event);
            // Only the background auto-optimizer reaches this branch;
            // manually-triggered optimize_leaves calls return their result
            // directly and never produce wallet-level optimization events.
            sdk.event_emitter
                .emit(&SdkEvent::AutoOptimization {
                    optimization_event: event.into(),
                })
                .await;
            false
        }
    }
}

async fn process_token_transaction_event(
    sdk: &BreezSdk,
    transaction: spark_wallet::TokenTransaction,
) -> Result<bool, SdkError> {
    let tx_inputs_are_ours = token_tx_inputs_are_ours_cached_or_query(sdk, &transaction).await?;
    let object_repository = ObjectCacheRepository::new(sdk.storage.clone());
    let payments = token_transaction_to_payments(
        &sdk.spark_wallet,
        &object_repository,
        &transaction,
        tx_inputs_are_ours,
    )
    .await?;

    if payments.is_empty() {
        return Ok(false);
    }

    let mut payment_emitted = false;
    for payment in payments {
        if sdk.finalize_payment(payment).await {
            payment_emitted = true;
        }
    }

    Ok(payment_emitted)
}

/// Wraps `token_tx_inputs_are_ours` with a local-cache fast path: if any
/// payment for this tx hash already exists in storage, its `payment_type`
/// answers the question and we skip the parent-tx fetch.
async fn token_tx_inputs_are_ours_cached_or_query(
    sdk: &BreezSdk,
    transaction: &spark_wallet::TokenTransaction,
) -> Result<bool, SdkError> {
    let existing = sdk
        .storage
        .list_payments(StorageListPaymentsRequest {
            payment_details_filter: Some(vec![StoragePaymentDetailsFilter::Token {
                tx_hash: Some(transaction.hash.clone()),
                conversion_filter: None,
                tx_type: None,
            }]),
            limit: Some(1),
            ..Default::default()
        })
        .await?;
    if let Some(payment) = existing.first() {
        return Ok(payment.payment_type == PaymentType::Send);
    }

    let parent_transaction = match &transaction.inputs {
        spark_wallet::TokenInputs::Transfer(token_transfer_input) => {
            let first_input = token_transfer_input
                .outputs_to_spend
                .first()
                .ok_or_else(|| SdkError::Generic("No input in token transfer input".to_string()))?;
            sdk.spark_wallet
                .get_token_transactions_by_hashes(vec![first_input.prev_token_tx_hash.clone()])
                .await?
                .into_iter()
                .next()
        }
        spark_wallet::TokenInputs::Mint(_) | spark_wallet::TokenInputs::Create(_) => None,
    };

    token_tx_inputs_are_ours(
        transaction,
        parent_transaction.as_ref(),
        sdk.spark_wallet.get_identity_public_key(),
    )
}

async fn cleanup_claimed_deposit(sdk: &BreezSdk, tx_id: &str, vout: u32) {
    if let Err(e) = sdk.storage.delete_deposit(tx_id.to_string(), vout).await {
        error!("Failed to delete claimed deposit {tx_id}:{vout} from storage: {e:?}");
    }
}

fn claim_static_deposit_outpoint(transfer: &WalletTransfer) -> Option<(String, u32)> {
    match transfer.user_request.as_ref()? {
        spark_wallet::SspUserRequest::ClaimStaticDeposit(info) => {
            let vout = u32::try_from(info.output_index).ok()?;
            Some((info.transaction_id.clone(), vout))
        }
        _ => None,
    }
}

async fn register_client_sync_listener(sdk: &BreezSdk) {
    let listener = ClientSyncListener {
        sync_coordinator: sdk.sync_coordinator.clone(),
    };
    sdk.event_emitter
        .add_internal_listener(Box::new(listener))
        .await;
}

struct ClientSyncListener {
    sync_coordinator: SyncCoordinator,
}

#[macros::async_trait]
impl EventListener for ClientSyncListener {
    async fn on_event(&self, event: SdkEvent) {
        match event {
            SdkEvent::PaymentSucceeded { .. }
            | SdkEvent::PaymentPending { .. }
            | SdkEvent::PaymentFailed { .. }
            | SdkEvent::ClaimedDeposits { .. } => {
                self.sync_coordinator
                    .trigger_sync_no_wait(SyncType::WalletState, true)
                    .await;
            }
            _ => {}
        }
    }
}

fn spawn_conversion_refunder(
    token_converter: Arc<dyn TokenConverter>,
    mut shutdown_receiver: watch::Receiver<()>,
) {
    let mut refund_requests = token_converter.subscribe_refund_requests();
    let span = tracing::Span::current();

    tokio::spawn(
        async move {
            loop {
                if let Err(e) = token_converter.refund_pending().await {
                    error!("Failed to refund failed conversions: {e:?}");
                }

                match refund_requests.as_mut() {
                    Some(trigger_receiver) => {
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
                    None => {
                        select! {
                            _ = shutdown_receiver.changed() => {
                                info!("Conversion refunder shutdown signal received");
                                return;
                            }
                            () = tokio::time::sleep(Duration::from_secs(150)) => {}
                        }
                    }
                }
            }
        }
        .instrument(span),
    );
}
