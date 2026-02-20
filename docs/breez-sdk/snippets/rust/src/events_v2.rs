use anyhow::Result;
use breez_sdk_spark::*;
use log::info;

#[allow(unused)]
async fn typed_event_listeners(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: on-payment
    // Listen only for payment events (Rust only)
    let payment_listener_id = sdk
        .on_payment(|payment| {
            info!(
                "Payment {}: {:?} — {} sats",
                payment.id, payment.status, payment.amount
            );
        })
        .await;
    // ANCHOR_END: on-payment

    // ANCHOR: on-sync
    // Listen only for Synced events (Rust only)
    let sync_listener_id = sdk
        .on_sync(|update| match update {
            SyncUpdate::BalanceUpdated { balance } => {
                if let Some(b) = balance {
                    info!("Balance: {} sats", b.balance_sats);
                }
            }
            SyncUpdate::PaymentsUpdated => info!("Payments synced"),
            SyncUpdate::FullSync => info!("Full sync complete"),
        })
        .await;
    // ANCHOR_END: on-sync

    // ANCHOR: on-deposit
    // Listen for deposit events (Rust only)
    let deposit_listener_id = sdk
        .on_deposit(|unclaimed, claimed| {
            if !unclaimed.is_empty() {
                info!("Unclaimed deposits: {}", unclaimed.len());
            }
            if !claimed.is_empty() {
                info!("Claimed deposits: {}", claimed.len());
            }
        })
        .await;
    // ANCHOR_END: on-deposit

    // ANCHOR: remove-typed-listener
    // Remove a typed listener using its ID
    sdk.remove_event_listener(&payment_listener_id).await;
    sdk.remove_event_listener(&sync_listener_id).await;
    sdk.remove_event_listener(&deposit_listener_id).await;
    // ANCHOR_END: remove-typed-listener
    Ok(())
}
