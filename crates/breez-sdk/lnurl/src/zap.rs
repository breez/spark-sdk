use crate::repository::LnurlRepository;
use lightning_invoice::Bolt11Invoice;
use nostr::{EventBuilder, JsonUtil, Keys, TagStandard};
use spark::operator::OperatorConfig;
use spark::operator::rpc::spark::SubscribeToEventsRequest;
use spark::operator::rpc::spark::subscribe_to_events_response::Event;
use spark::operator::rpc::{ConnectionManager, SparkRpcClient};
use spark::services::Transfer;
use spark::session_manager::InMemorySessionManager;
use spark::ssp::ServiceProvider;
use spark_wallet::DefaultSigner;
use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, error, trace, warn};

#[derive(Debug, Clone)]
pub struct Zap {
    pub payment_hash: String,
    pub zap_request: String,
    pub zap_event: Option<String>,
    pub user_pubkey: String,
    pub invoice_expiry: i64,
}

/// Helper function to create an RPC client and subscribe to a user for zaps.
/// This consolidates the common pattern of getting a transport, creating an RPC client,
/// and starting the zap subscription.
#[allow(clippy::too_many_arguments)]
pub async fn create_rpc_client_and_subscribe<DB>(
    db: DB,
    user_pubkey: bitcoin::secp256k1::PublicKey,
    connection_manager: &Arc<dyn ConnectionManager>,
    coordinator: &OperatorConfig,
    signer: Arc<DefaultSigner>,
    session_manager: Arc<InMemorySessionManager>,
    service_provider: Arc<ServiceProvider>,
    nostr_keys: Keys,
    subscribed_keys: Arc<Mutex<HashSet<String>>>,
) -> Result<(), anyhow::Error>
where
    DB: LnurlRepository + Clone + Send + Sync + 'static,
{
    let transport = connection_manager.get_transport(coordinator).await?;
    let rpc_client = SparkRpcClient::new(transport, signer, user_pubkey, session_manager);

    subscribe_to_user_for_zaps(
        db,
        user_pubkey,
        rpc_client,
        service_provider,
        nostr_keys,
        subscribed_keys,
    );

    Ok(())
}

#[allow(clippy::too_many_lines)]
pub fn subscribe_to_user_for_zaps<DB>(
    db: DB,
    user_pk: bitcoin::secp256k1::PublicKey,
    rpc: SparkRpcClient,
    ssp_client: Arc<ServiceProvider>,
    nostr_keys: Keys,
    subscribed_keys: Arc<Mutex<HashSet<String>>>,
) where
    DB: LnurlRepository + Clone + Send + Sync + 'static,
{
    trace!("Subscribing to user {user_pk}");

    tokio::spawn(async move {
        let mut subscribed = subscribed_keys.lock().await;
        if !subscribed.insert(user_pk.to_string()) {
            debug!("Already subscribed to user {user_pk}, skipping");
            return;
        }
        drop(subscribed); // release the lock

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
                    error!("Failed to subscribe to events for user {user_pk}: {e}, retrying in 5s");
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
                        match db.user_has_unexpired_invoices(&user_pk.to_string()).await {
                            Ok(has_unexpired) => {
                                if !has_unexpired {
                                    debug!("User {user_pk} has no more unexpired invoices (timeout check), unsubscribing");
                                    subscribed.remove(&user_pk.to_string());
                                    drop(subscribed);
                                    return; // Exit subscription completely
                                }
                            }
                            Err(e) => {
                                error!("Failed to check unexpired invoices for user {user_pk}: {e}");
                            }
                        }
                        drop(subscribed);
                        continue; // Continue to next iteration
                    }
                };
                let response = match message {
                    Ok(Some(event)) => event,
                    Ok(None) => {
                        warn!("Server event stream closed for user {user_pk}, reconnecting...");
                        break; // Break inner loop to reconnect
                    }
                    Err(e) => {
                        error!("Error receiving event for user {user_pk}: {e}, reconnecting...");
                        break; // Break inner loop to reconnect
                    }
                };

                let Some(event) = response.event else {
                    warn!("Received empty event, skipping");
                    continue;
                };

                match event {
                    Event::Transfer(transfer_event) => {
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

                        if let Some(req) = ssp_transfer.and_then(|s| s.user_request) {
                            if let Some(inv) = req.get_lightning_invoice()
                                && let Ok(invoice) = Bolt11Invoice::from_str(&inv)
                            {
                                let payment_hash = invoice.payment_hash().to_string();
                                if let Ok(Some(mut zap)) =
                                    db.get_zap_by_payment_hash(&payment_hash).await
                                    && zap.zap_event.is_none()
                                {
                                    let zap_request = nostr::Event::from_json(&zap.zap_request)
                                        .expect("we validated this before inserting");
                                    if let Ok(zap_event) = EventBuilder::zap_receipt(
                                        inv,
                                        req.get_lightning_preimage(),
                                        &zap_request,
                                    )
                                    .sign_with_keys(&nostr_keys)
                                    {
                                        zap.zap_event = Some(zap_event.as_json());
                                        db.upsert_zap(&zap).await.unwrap();

                                        let nostr_client =
                                            nostr_sdk::Client::new(nostr_keys.clone());

                                        let relays = zap_request
                                            .tags
                                            .iter()
                                            .filter_map(|t| {
                                                if let Some(TagStandard::Relay(r)) =
                                                    t.as_standardized()
                                                {
                                                    Some(r.clone())
                                                } else {
                                                    None
                                                }
                                            })
                                            .collect::<Vec<_>>();

                                        for r in relays {
                                            if let Err(e) = nostr_client.add_relay(&r).await {
                                                error!("Failed to add relay {r}: {e}");
                                            }
                                        }
                                        nostr_client.connect().await;

                                        if let Err(e) = nostr_client.send_event(&zap_event).await {
                                            error!("Failed to send zap event to nostr relay: {e}",);
                                        } else {
                                            debug!("Sent zap event to nostr relay");
                                        }

                                        nostr_client.disconnect().await; // safely cleanup

                                        // Check if user still has unexpired invoices
                                        // Hold the lock while checking to prevent race condition with new subscriptions
                                        let mut subscribed = subscribed_keys.lock().await;
                                        match db
                                            .user_has_unexpired_invoices(&user_pk.to_string())
                                            .await
                                        {
                                            Ok(has_unexpired) => {
                                                if !has_unexpired {
                                                    debug!(
                                                        "User {user_pk} has no more unexpired invoices, unsubscribing"
                                                    );
                                                    subscribed.remove(&user_pk.to_string());
                                                    drop(subscribed);
                                                    break; // Exit subscription loop
                                                }
                                            }
                                            Err(e) => {
                                                error!(
                                                    "Failed to check unexpired invoices for user {user_pk}: {e}"
                                                );
                                            }
                                        }
                                        drop(subscribed);
                                    }
                                }
                            }
                        } else {
                            trace!(
                                "No SSP transfer found for transfer {}, skipping",
                                transfer.id
                            );
                        }
                    }
                    Event::Deposit(_) => {
                        trace!("Received deposit event, skipping");
                    }
                    Event::Connected(_) => {
                        debug!("Received connected event");
                    }
                }
            }

            // Connection lost, wait before reconnecting
            debug!("Connection lost for user {user_pk}, reconnecting in 5s...");
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }
    });
}
