use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;

use lightning_invoice::Bolt11Invoice;
use nostr::{JsonUtil, TagStandard};
use spark::operator::OperatorConfig;
use spark::operator::rpc::spark::SubscribeToEventsRequest;
use spark::operator::rpc::spark::subscribe_to_events_response::Event;
use spark::operator::rpc::{ConnectionManager, OperatorRpcError, SparkRpcClient};
use spark::services::Transfer;
use spark::session_manager::InMemorySessionManager;
use spark::ssp::ServiceProvider;
use spark_wallet::DefaultSigner;
use tokio::sync::Mutex;
use tokio::sync::watch;
use tracing::{debug, error, trace, warn};

use crate::invoice_paid::handle_invoice_paid;
use crate::repository::LnurlRepository;
use crate::time::now_millis;
use crate::zap::Zap;

/// Retry configuration for zap receipt publishing
const BASE_RETRY_DELAY_MS: i64 = 30_000; // 30 seconds
const RETRY_MULTIPLIER: f64 = 1.5;
const MAX_RETRY_DURATION_MS: i64 = 14 * 24 * 60 * 60 * 1000; // 14 days

/// Helper function to create an RPC client and subscribe to a user for invoice payments.
#[allow(clippy::too_many_arguments)]
pub async fn create_rpc_client_and_subscribe<DB>(
    db: DB,
    user_pubkey: bitcoin::secp256k1::PublicKey,
    connection_manager: &Arc<dyn ConnectionManager>,
    coordinator: &OperatorConfig,
    signer: Arc<DefaultSigner>,
    session_manager: Arc<InMemorySessionManager>,
    service_provider: Arc<ServiceProvider>,
    nostr_keys: nostr::Keys,
    subscribed_keys: Arc<Mutex<HashSet<String>>>,
    trigger: watch::Sender<()>,
) -> Result<(), anyhow::Error>
where
    DB: LnurlRepository + Clone + Send + Sync + 'static,
{
    let transport = connection_manager.get_transport(coordinator).await?;
    let rpc_client = SparkRpcClient::new(transport, signer, user_pubkey, session_manager);

    subscribe_to_user_for_invoices(
        db,
        user_pubkey,
        rpc_client,
        service_provider,
        nostr_keys,
        subscribed_keys,
        trigger,
    );

    Ok(())
}

/// Subscribe to a user's payment events and handle invoice payments.
#[allow(clippy::too_many_lines)]
fn subscribe_to_user_for_invoices<DB>(
    db: DB,
    user_pk: bitcoin::secp256k1::PublicKey,
    rpc: SparkRpcClient,
    ssp_client: Arc<ServiceProvider>,
    _nostr_keys: nostr::Keys,
    subscribed_keys: Arc<Mutex<HashSet<String>>>,
    trigger: watch::Sender<()>,
) where
    DB: LnurlRepository + Clone + Send + Sync + 'static,
{
    trace!("Subscribing to user {user_pk} for invoice payments");

    tokio::spawn(async move {
        let mut subscribed = subscribed_keys.lock().await;
        if !subscribed.insert(user_pk.to_string()) {
            debug!("Already subscribed to user {user_pk}, skipping");
            return;
        }
        drop(subscribed);

        // Outer reconnection loop
        loop {
            debug!("Connecting to event stream for user {user_pk}");
            let mut stream = match rpc
                .subscribe_to_events(SubscribeToEventsRequest {
                    identity_public_key: user_pk.serialize().to_vec(),
                })
                .await
            {
                Ok(stream) => stream,
                Err(e) => {
                    if let OperatorRpcError::Connection(status) = &e
                        && status.code() == tonic::Code::PermissionDenied
                    {
                        debug!("Permission denied for user {user_pk}, unsubscribing...");
                        let mut subscribed = subscribed_keys.lock().await;
                        subscribed.remove(&user_pk.to_string());
                        drop(subscribed);
                        return;
                    }
                    error!("Failed to subscribe to events for user {user_pk}: {e}, retrying in 5s");
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                    continue;
                }
            };

            // Inner event processing loop
            loop {
                let message = tokio::select! {
                    message = stream.message() => message,
                    () = tokio::time::sleep(tokio::time::Duration::from_secs(60)) => {
                        // Periodically check if user still has unexpired invoices
                        let mut subscribed = subscribed_keys.lock().await;
                        match db.is_invoice_monitored_user(&user_pk.to_string()).await {
                            Ok(has_unexpired) => {
                                if !has_unexpired {
                                    debug!("User {user_pk} has no more unexpired invoices (timeout check), unsubscribing");
                                    subscribed.remove(&user_pk.to_string());
                                    drop(subscribed);
                                    return;
                                }
                            }
                            Err(e) => {
                                error!("Failed to check unexpired invoices for user {user_pk}: {e}");
                            }
                        }
                        drop(subscribed);
                        continue;
                    }
                };
                let response = match message {
                    Ok(Some(event)) => event,
                    Ok(None) => {
                        warn!("Server event stream closed for user {user_pk}, reconnecting...");
                        break;
                    }
                    Err(e) => {
                        if e.code() == tonic::Code::PermissionDenied {
                            debug!("Permission denied for user {user_pk}, unsubscribing...");
                            let mut subscribed = subscribed_keys.lock().await;
                            subscribed.remove(&user_pk.to_string());
                            drop(subscribed);
                            return;
                        }
                        error!("Error receiving event for user {user_pk}: {e}, reconnecting...");
                        break;
                    }
                };

                let Some(event) = response.event else {
                    warn!("Received empty event, skipping");
                    continue;
                };

                let transfer_event = match event {
                    Event::Transfer(transfer_event) => transfer_event,
                    Event::Deposit(_) => {
                        trace!("Received deposit event, skipping");
                        continue;
                    }
                    Event::Connected(_) => {
                        debug!("Received connected event");
                        continue;
                    }
                };

                let Some(transfer) = transfer_event.transfer else {
                    warn!("Received empty transfer event, skipping");
                    continue;
                };
                debug!("Received transfer event with transfer id {}", transfer.id);
                trace!("Received transfer event with transfer: {:?}", transfer);
                let transfer: Transfer = match transfer.try_into() {
                    Ok(transfer) => transfer,
                    Err(e) => {
                        error!("Failed to convert transfer event: {}", e);
                        continue;
                    }
                };

                // we only care about LN receive transfers
                if transfer.transfer_type != spark::services::TransferType::PreimageSwap {
                    continue;
                }

                let ssp_transfer = ssp_client
                    .get_transfers(vec![transfer.id.to_string()])
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .next();

                let Some(req) = ssp_transfer.and_then(|s| s.user_request) else {
                    debug!(
                        "No SSP transfer found for transfer {}, skipping",
                        transfer.id
                    );
                    continue;
                };

                let Some(inv) = req.get_lightning_invoice() else {
                    debug!(
                        "No lightning invoice found in user request for transfer {}, skipping",
                        transfer.id
                    );
                    continue;
                };

                let Ok(invoice) = Bolt11Invoice::from_str(&inv) else {
                    error!(
                        "Failed to parse lightning invoice from user request for transfer {}, skipping",
                        transfer.id
                    );
                    continue;
                };

                let payment_hash = invoice.payment_hash().to_string();

                // Get the preimage from the SSP transfer
                let Some(preimage) = req.get_lightning_preimage() else {
                    debug!("No preimage found for transfer {}, skipping", transfer.id);
                    continue;
                };

                // Use the central invoice paid handler
                if let Err(e) = handle_invoice_paid(&db, &payment_hash, &preimage, &trigger).await {
                    error!(
                        "Failed to handle invoice paid for payment hash {}: {}",
                        payment_hash, e
                    );
                }

                // Check if user still has unexpired invoices
                let mut subscribed = subscribed_keys.lock().await;
                match db.is_invoice_monitored_user(&user_pk.to_string()).await {
                    Ok(has_unexpired) => {
                        if !has_unexpired {
                            debug!("User {user_pk} has no more unexpired invoices, unsubscribing");
                            subscribed.remove(&user_pk.to_string());
                            drop(subscribed);
                            return;
                        }
                    }
                    Err(e) => {
                        error!("Failed to check unexpired invoices for user {user_pk}: {e}");
                    }
                }
                drop(subscribed);
            }

            // Connection lost, wait before reconnecting
            debug!("Connection lost for user {user_pk}, reconnecting in 5s...");
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }
    });
}

/// Start the background processor that handles the `newly_paid` queue.
/// This processor publishes zap receipts for paid invoices with retry logic.
pub fn start_background_processor<DB>(
    db: DB,
    nostr_keys: nostr::Keys,
    mut trigger_rx: watch::Receiver<()>,
) where
    DB: LnurlRepository + Clone + Send + Sync + 'static,
{
    tokio::spawn(async move {
        // Process any pending items on startup
        process_newly_paid_queue(&db, &nostr_keys).await;

        // Wait for triggers
        loop {
            // Wait for a trigger or timeout after 60 seconds to check for any missed items
            tokio::select! {
                result = trigger_rx.changed() => {
                    if result.is_err() {
                        debug!("Background processor trigger channel closed, exiting");
                        return;
                    }
                }
                () = tokio::time::sleep(tokio::time::Duration::from_secs(60)) => {
                    // Periodic check for any items that need processing
                }
            }

            process_newly_paid_queue(&db, &nostr_keys).await;
        }
    });
}

/// Process all pending items in the `newly_paid` queue.
async fn process_newly_paid_queue<DB>(db: &DB, nostr_keys: &nostr::Keys)
where
    DB: LnurlRepository + Clone + Send + Sync + 'static,
{
    let pending = match db.get_pending_newly_paid().await {
        Ok(pending) => pending,
        Err(e) => {
            error!("Failed to get pending newly paid: {}", e);
            return;
        }
    };

    for item in pending {
        process_newly_paid_item(db, nostr_keys, &item).await;
    }
}

/// Process a single `newly_paid` item: publish zap receipt if applicable.
async fn process_newly_paid_item<DB>(
    db: &DB,
    nostr_keys: &nostr::Keys,
    item: &crate::repository::NewlyPaid,
) where
    DB: LnurlRepository + Clone + Send + Sync + 'static,
{
    let payment_hash = &item.payment_hash;

    // Check if we've exceeded max retry duration (14 days)
    let now = now_millis();
    if now.saturating_sub(item.created_at) > MAX_RETRY_DURATION_MS {
        debug!(
            "Payment hash {} exceeded max retry duration, removing from queue",
            payment_hash
        );
        if let Err(e) = db.delete_newly_paid(payment_hash).await {
            error!(
                "Failed to delete expired newly paid {}: {}",
                payment_hash, e
            );
        }
        return;
    }

    // Check if there's a zap record for this payment
    let zap = match db.get_zap_by_payment_hash(payment_hash).await {
        Ok(Some(zap)) => zap,
        Ok(None) => {
            // No zap record - just remove from queue (non-zap invoice)
            debug!(
                "No zap found for payment hash {}, removing from queue",
                payment_hash
            );
            if let Err(e) = db.delete_newly_paid(payment_hash).await {
                error!("Failed to delete newly paid {}: {}", payment_hash, e);
            }
            return;
        }
        Err(e) => {
            error!("Failed to get zap by payment hash {}: {}", payment_hash, e);
            return;
        }
    };

    // If zap receipt already exists, remove from queue
    if zap.zap_event.is_some() {
        debug!(
            "Zap receipt already exists for payment hash {}, removing from queue",
            payment_hash
        );
        if let Err(e) = db.delete_newly_paid(payment_hash).await {
            error!("Failed to delete newly paid {}: {}", payment_hash, e);
        }
        return;
    }

    // Get the invoice to get preimage and bolt11
    let invoice = match db.get_invoice_by_payment_hash(payment_hash).await {
        Ok(Some(invoice)) => invoice,
        Ok(None) => {
            debug!(
                "Invoice not found for payment hash {}, removing from queue",
                payment_hash
            );
            if let Err(e) = db.delete_newly_paid(payment_hash).await {
                error!("Failed to delete newly paid {}: {}", payment_hash, e);
            }
            return;
        }
        Err(e) => {
            error!(
                "Failed to get invoice by payment hash {}: {}",
                payment_hash, e
            );
            return;
        }
    };

    let Some(preimage) = &invoice.preimage else {
        // No preimage yet, keep in queue
        debug!(
            "Invoice {} has no preimage yet, keeping in queue",
            payment_hash
        );
        return;
    };

    // Try to publish zap receipt
    match publish_zap_receipt(db, nostr_keys, &zap, &invoice.invoice, preimage).await {
        Ok(()) => {
            debug!("Successfully published zap receipt for {}", payment_hash);
            if let Err(e) = db.delete_newly_paid(payment_hash).await {
                error!("Failed to delete newly paid {}: {}", payment_hash, e);
            }
        }
        Err(e) => {
            warn!(
                "Failed to publish zap receipt for {}: {}, will retry",
                payment_hash, e
            );

            // Calculate next retry time with exponential backoff
            let retry_count = item.retry_count.saturating_add(1);
            #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
            let delay_ms = (BASE_RETRY_DELAY_MS as f64 * RETRY_MULTIPLIER.powi(retry_count)) as i64;
            let next_retry_at = now.saturating_add(delay_ms);

            if let Err(e) = db
                .update_newly_paid_retry(payment_hash, retry_count, next_retry_at)
                .await
            {
                error!(
                    "Failed to update retry for newly paid {}: {}",
                    payment_hash, e
                );
            }
        }
    }
}

/// Publish a zap receipt to nostr relays.
async fn publish_zap_receipt<DB>(
    db: &DB,
    nostr_keys: &nostr::Keys,
    zap: &Zap,
    bolt11: &str,
    preimage: &str,
) -> Result<(), anyhow::Error>
where
    DB: LnurlRepository + Clone + Send + Sync + 'static,
{
    let zap_request = nostr::Event::from_json(&zap.zap_request)?;

    // Build the zap receipt
    let zap_event =
        nostr::EventBuilder::zap_receipt(bolt11, Some(preimage.to_string()), &zap_request)
            .sign_with_keys(nostr_keys)?;

    let relays = zap_request
        .tags
        .iter()
        .filter_map(|t| {
            if let Some(TagStandard::Relays(r)) = t.as_standardized() {
                Some(r.clone())
            } else {
                None
            }
        })
        .flatten()
        .collect::<Vec<_>>();

    if relays.is_empty() {
        return Err(anyhow::anyhow!("No relays in zap request"));
    }

    let nostr_client = nostr_sdk::Client::new(nostr_keys.clone());
    for r in &relays {
        if let Err(e) = nostr_client.add_relay(r).await {
            warn!("Failed to add relay {r}: {e}");
        }
    }
    nostr_client.connect().await;

    let result = nostr_client.send_event(&zap_event).await;
    nostr_client.disconnect().await;

    if let Err(e) = result {
        return Err(anyhow::anyhow!("Failed to send zap event: {}", e));
    }

    // Update the zap record with the zap event
    let mut updated_zap = zap.clone();
    updated_zap.zap_event = Some(zap_event.as_json());
    updated_zap.updated_at = now_millis();
    db.upsert_zap(&updated_zap).await?;

    debug!("Published zap receipt to {} relays", relays.len());
    Ok(())
}
