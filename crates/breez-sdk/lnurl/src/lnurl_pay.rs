use crate::repository::LnurlRepository;
use lightning_invoice::Bolt11Invoice;
use spark::operator::OperatorConfig;
use spark::operator::rpc::spark::SubscribeToEventsRequest;
use spark::operator::rpc::spark::subscribe_to_events_response::Event;
use spark::operator::rpc::{ConnectionManager, OperatorRpcError, SparkRpcClient};
use spark::services::Transfer;
use spark::session_manager::InMemorySessionManager;
use spark::ssp::ServiceProvider;
use spark_wallet::DefaultSigner;
use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, error, trace, warn};

/// Helper function to create an RPC client and subscribe to a user for LNURL-pay invoice monitoring.
/// This consolidates the common pattern of getting a transport, creating an RPC client,
/// and starting the subscription for payment status updates.
#[allow(clippy::too_many_arguments)]
pub async fn create_rpc_client_and_subscribe<DB>(
    db: DB,
    user_pubkey: bitcoin::secp256k1::PublicKey,
    connection_manager: &Arc<dyn ConnectionManager>,
    coordinator: &OperatorConfig,
    signer: Arc<DefaultSigner>,
    session_manager: Arc<InMemorySessionManager>,
    service_provider: Arc<ServiceProvider>,
    subscribed_keys: Arc<Mutex<HashSet<String>>>,
) -> Result<(), anyhow::Error>
where
    DB: LnurlRepository + Clone + Send + Sync + 'static,
{
    let transport = connection_manager.get_transport(coordinator).await?;
    let rpc_client = SparkRpcClient::new(transport, signer, user_pubkey, session_manager);

    subscribe_to_user_for_lnurl_payments(
        db,
        user_pubkey,
        rpc_client,
        service_provider,
        subscribed_keys,
    );

    Ok(())
}

/// Subscribes to a user's events to monitor LNURL-pay invoice payments.
/// When a payment is received, updates the invoice with the preimage.
/// This is used for non-privacy mode where the server monitors payments directly.
#[allow(clippy::too_many_lines)]
pub fn subscribe_to_user_for_lnurl_payments<DB>(
    db: DB,
    user_pk: bitcoin::secp256k1::PublicKey,
    rpc: SparkRpcClient,
    ssp_client: Arc<ServiceProvider>,
    subscribed_keys: Arc<Mutex<HashSet<String>>>,
) where
    DB: LnurlRepository + Clone + Send + Sync + 'static,
{
    trace!("Subscribing to user {user_pk} for LNURL-pay invoice monitoring");

    tokio::spawn(async move {
        let mut subscribed = subscribed_keys.lock().await;
        if !subscribed.insert(user_pk.to_string()) {
            debug!("Already subscribed to user {user_pk} for LNURL-pay, skipping");
            return;
        }
        drop(subscribed); // release the lock

        // Outer reconnection loop
        loop {
            debug!("Connecting to event stream for user {user_pk} (LNURL-pay)");
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
                        warn!("Permission denied for user {user_pk}, unsubscribing from LNURL-pay...");
                        let mut subscribed = subscribed_keys.lock().await;
                        subscribed.remove(&user_pk.to_string());
                        drop(subscribed);
                        return;
                    }
                    error!("Failed to subscribe to events for user {user_pk} (LNURL-pay): {e}, retrying in 5s");
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                    continue;
                }
            };

            // Inner event processing loop
            loop {
                // Check if user still has unexpired invoices, or wait for a message
                // We check every 60 seconds to avoid keeping subscriptions active after expiry
                let message = tokio::select! {
                    message = stream.message() => message,
                    () = tokio::time::sleep(tokio::time::Duration::from_secs(60)) => {
                        // Periodically check if user still has unexpired invoices
                        // Hold the lock while checking to prevent race condition with new subscriptions
                        let mut subscribed = subscribed_keys.lock().await;
                        match db.is_lnurl_pay_monitored_user(&user_pk.to_string()).await {
                            Ok(has_unexpired) => {
                                if !has_unexpired {
                                    debug!("User {user_pk} has no more unexpired LNURL-pay invoices (timeout check), unsubscribing");
                                    subscribed.remove(&user_pk.to_string());
                                    drop(subscribed);
                                    return; // Exit subscription completely
                                }
                            }
                            Err(e) => {
                                error!("Failed to check unexpired LNURL-pay invoices for user {user_pk}: {e}");
                            }
                        }
                        drop(subscribed);
                        continue; // Continue to next iteration
                    }
                };
                let response = match message {
                    Ok(Some(event)) => event,
                    Ok(None) => {
                        warn!("Server event stream closed for user {user_pk} (LNURL-pay), reconnecting...");
                        break; // Break inner loop to reconnect
                    }
                    Err(e) => {
                        if e.code() == tonic::Code::PermissionDenied {
                            warn!("Permission denied for user {user_pk}, unsubscribing from LNURL-pay...");
                            let mut subscribed = subscribed_keys.lock().await;
                            subscribed.remove(&user_pk.to_string());
                            drop(subscribed);
                            return;
                        }
                        error!("Error receiving event for user {user_pk} (LNURL-pay): {e}, reconnecting...");
                        break; // Break inner loop to reconnect
                    }
                };

                let Some(event) = response.event else {
                    warn!("Received empty event (LNURL-pay), skipping");
                    continue;
                };

                let transfer_event = match event {
                    Event::Transfer(transfer_event) => transfer_event,
                    Event::Deposit(_) => {
                        trace!("Received deposit event (LNURL-pay), skipping");
                        continue;
                    }
                    Event::Connected(_) => {
                        debug!("Received connected event (LNURL-pay)");
                        continue;
                    }
                };

                let Some(transfer) = transfer_event.transfer else {
                    warn!("Received empty transfer event (LNURL-pay), skipping");
                    continue;
                };
                debug!("Received transfer event with transfer id {} (LNURL-pay)", transfer.id);
                trace!("Received transfer event with transfer: {:?} (LNURL-pay)", transfer);
                let transfer: Transfer = match transfer.try_into() {
                    Ok(transfer) => transfer,
                    Err(e) => {
                        error!("Failed to convert transfer event (LNURL-pay): {}", e);
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
                        "No SSP transfer found for transfer {} (LNURL-pay), skipping",
                        transfer.id
                    );
                    continue;
                };

                let Some(inv) = req.get_lightning_invoice() else {
                    debug!(
                        "No lightning invoice found in user request for transfer {} (LNURL-pay), skipping",
                        transfer.id
                    );
                    continue;
                };

                let Ok(invoice) = Bolt11Invoice::from_str(&inv) else {
                    error!(
                        "Failed to parse lightning invoice from user request for transfer {} (LNURL-pay), skipping",
                        transfer.id
                    );
                    continue;
                };

                let payment_hash = invoice.payment_hash().to_string();
                
                // Check if we have an LNURL-pay invoice record for this payment
                let lnurl_invoice = match db.get_lnurl_pay_invoice_by_payment_hash(&payment_hash).await {
                    Ok(Some(invoice)) => invoice,
                    Ok(None) => {
                        debug!("No LNURL-pay invoice found for payment hash {} (LNURL-pay), skipping", payment_hash);
                        continue;
                    }
                    Err(e) => {
                        error!(
                            "Failed to get LNURL-pay invoice by payment hash {} (LNURL-pay): {}, skipping",
                            payment_hash, e
                        );
                        continue;
                    }
                };

                // Skip if already has a preimage (already marked as paid)
                if lnurl_invoice.preimage.is_some() {
                    debug!(
                        "LNURL-pay invoice already has preimage for payment hash {} (LNURL-pay), skipping",
                        payment_hash
                    );
                    continue;
                }

                // Get the preimage from the SSP response
                let Some(preimage) = req.get_lightning_preimage() else {
                    debug!(
                        "No preimage found in SSP response for payment hash {} (LNURL-pay), skipping",
                        payment_hash
                    );
                    continue;
                };

                // Update the invoice with the preimage
                if let Err(e) = db.set_lnurl_pay_invoice_preimage(&payment_hash, &preimage).await {
                    error!(
                        "Failed to set preimage for LNURL-pay invoice {}: {} (LNURL-pay)",
                        payment_hash, e
                    );
                    continue;
                }

                debug!("Updated LNURL-pay invoice {} with preimage (LNURL-pay)", payment_hash);

                // Check if user still has unexpired invoices
                // Hold the lock while checking to prevent race condition with new subscriptions
                let mut subscribed = subscribed_keys.lock().await;
                match db.is_lnurl_pay_monitored_user(&user_pk.to_string()).await {
                    Ok(has_unexpired) => {
                        if !has_unexpired {
                            debug!("User {user_pk} has no more unexpired LNURL-pay invoices, unsubscribing");
                            subscribed.remove(&user_pk.to_string());
                            drop(subscribed);
                            return; // Exit subscription completely
                        }
                    }
                    Err(e) => {
                        error!("Failed to check unexpired LNURL-pay invoices for user {user_pk}: {e}");
                    }
                }
                drop(subscribed);
            }

            // Connection lost, wait before reconnecting
            debug!("Connection lost for user {user_pk} (LNURL-pay), reconnecting in 5s...");
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }
    });
}
