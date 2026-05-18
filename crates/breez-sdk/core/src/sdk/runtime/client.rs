use std::sync::Arc;

use platform_utils::time::{Duration, SystemTime};
use platform_utils::tokio;
use spark_wallet::{WalletEvent, WalletTransfer};
use tokio::{
    select,
    sync::{broadcast, watch},
};
use tracing::{Instrument, debug, error, info, trace, warn};

use crate::{
    GetInfoRequest, GetInfoResponse, Payment,
    error::SdkError,
    events::{EventListener, SdkEvent},
    persist::ObjectCacheRepository,
    token_conversion::TokenConverter,
    utils::{payments::get_payment_and_emit_event, run_with_shutdown},
};

use super::{RuntimeEvent, RuntimeProfile};
use crate::sdk::{
    BreezSdk, SyncCoordinator, SyncRequest, SyncType,
    helpers::{BalanceWatcher, update_balances},
};

pub(super) struct ClientRuntime;

#[macros::async_trait]
impl RuntimeProfile for ClientRuntime {
    fn starts_background_services(&self) -> bool {
        true
    }

    fn start_sdk_services(&self, sdk: &BreezSdk, initial_synced_sender: watch::Sender<bool>) {
        register_client_sync_listener(sdk);
        sdk.spawn_spark_private_mode_initialization();
        spawn_client_runtime_loop(sdk, initial_synced_sender);
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

    async fn ensure_spark_private_mode_initialized(&self, sdk: &BreezSdk) -> Result<(), SdkError> {
        sdk.ensure_spark_private_mode_initialized_inner().await
    }
}

fn spawn_client_runtime_loop(sdk: &BreezSdk, initial_synced_sender: watch::Sender<bool>) {
    let sdk = sdk.clone();
    let mut shutdown_receiver = sdk.shutdown_sender.subscribe();
    let mut wallet_events = sdk.spark_wallet.subscribe_events();
    let mut runtime_events = sdk.event_emitter.subscribe_runtime_events();
    let mut sync_requests = sdk.sync_coordinator.subscribe();
    let mut last_sync_time = SystemTime::now();
    let sync_interval = u64::from(sdk.config.sync_interval_secs);
    let span = tracing::Span::current();

    tokio::spawn(
        async move {
            let balance_watcher =
                BalanceWatcher::new(sdk.spark_wallet.clone(), sdk.storage.clone());
            let balance_watcher_id = sdk.add_event_listener(Box::new(balance_watcher)).await;
            sdk.init_jwt().await;

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
                        on_wallet_event(&sdk, event).await;
                    }

                    runtime_event = runtime_events.recv() => {
                        if on_runtime_event(&sdk, runtime_event).await.is_break() {
                            return;
                        }
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

                    () = sdk.jwt_refresh_interval() => {
                        refresh_jwt(&sdk).await;
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
            let payment_event_emitted = handle_wallet_event(sdk, event).await;

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

async fn on_runtime_event(
    sdk: &BreezSdk,
    event: Result<RuntimeEvent, broadcast::error::RecvError>,
) -> std::ops::ControlFlow<()> {
    match event {
        Ok(RuntimeEvent::StableBalanceConversionCompleted) => {
            sdk.sync_coordinator
                .trigger_sync_no_wait(SyncType::Full, true)
                .await;
            std::ops::ControlFlow::Continue(())
        }
        Err(broadcast::error::RecvError::Lagged(skipped)) => {
            warn!("Runtime event receiver lagged, skipped {skipped} events");
            std::ops::ControlFlow::Continue(())
        }
        Err(broadcast::error::RecvError::Closed) => {
            info!("Runtime event sender closed");
            std::ops::ControlFlow::Break(())
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
                if let Err(e) = cloned_sdk.sync_wallet_internal(&sync_request).await {
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

async fn refresh_jwt(sdk: &BreezSdk) {
    let token = match sdk.new_jwt().await {
        Ok(token) => token,
        Err(err) => {
            warn!("Could not fetch new JWT: {err}");
            return;
        }
    };
    sdk.set_and_save_jwt(token).await;
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
            let mut payment_emitted = false;
            // Drop any unclaimed-deposit record for this outpoint independently
            // of payment ingestion, so conversion failures do not leave it stale.
            if let Some((tx_id, vout)) = claim_static_deposit_outpoint(&transfer) {
                cleanup_claimed_deposit(sdk, &tx_id, vout).await;
            }
            if let Ok(mut payment) = Payment::try_from(transfer) {
                if let Err(e) = sdk.storage.insert_payment(payment.clone()).await {
                    error!("Failed to insert succeeded payment: {e:?}");
                }

                // This was already requested at TransferClaimStarting, but it
                // might still be racing, so ensure metadata before emitting.
                sdk.sync_single_lnurl_metadata(&mut payment).await;

                if let Err(e) = update_balances(sdk.spark_wallet.clone(), sdk.storage.clone()).await
                {
                    error!("Failed to update balances before PaymentSucceeded event: {e:?}");
                }

                get_payment_and_emit_event(&sdk.storage, &sdk.event_emitter, payment).await;
                payment_emitted = true;
            }
            payment_emitted
        }
        WalletEvent::TransferClaimStarting(transfer) => {
            info!("Transfer claim starting");
            let mut payment_emitted = false;
            if let Ok(mut payment) = Payment::try_from(transfer) {
                if let Err(e) = sdk.storage.insert_payment(payment.clone()).await {
                    error!("Failed to insert pending payment: {e:?}");
                }

                sdk.sync_single_lnurl_metadata(&mut payment).await;
                get_payment_and_emit_event(&sdk.storage, &sdk.event_emitter, payment).await;
                payment_emitted = true;
            }
            payment_emitted
        }
        WalletEvent::Optimization(event) => {
            info!("Optimization event: {:?}", event);
            false
        }
    }
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

fn register_client_sync_listener(sdk: &BreezSdk) {
    let event_emitter = sdk.event_emitter.clone();
    let listener = ClientSyncListener {
        sync_coordinator: sdk.sync_coordinator.clone(),
    };
    let span = tracing::Span::current();

    tokio::spawn(
        async move {
            event_emitter
                .add_internal_listener(Box::new(listener))
                .await;
        }
        .instrument(span),
    );
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
