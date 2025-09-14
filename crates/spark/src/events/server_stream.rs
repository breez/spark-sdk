use std::{sync::Arc, time::Duration};

use bitcoin::secp256k1::PublicKey;
use tokio::time::sleep;
use tokio_with_wasm::alias as tokio;
use tracing::{debug, error, info, trace, warn};

use crate::{
    events::{EventPublisher, models::SparkEvent},
    operator::{
        OperatorPool,
        rpc::spark::{SubscribeToEventsRequest, subscribe_to_events_response::Event},
    },
    services::Transfer,
    tree::TreeNode,
};

pub async fn subscribe_server_events(
    identity_public_key: PublicKey,
    operator_pool: Arc<OperatorPool>,
    publisher: &EventPublisher,
    reconnect_interval: Duration,
    cancellation_token: &mut tokio::sync::watch::Receiver<()>,
) {
    loop {
        match cancellation_token.has_changed() {
            Ok(true) => {
                info!("Cancellation token changed, stopping event subscription");
                return;
            }
            Ok(false) => {}
            Err(_) => {
                info!("Cancellation token sender is gone, returning");
                return;
            }
        }

        let mut stream = match operator_pool
            .get_coordinator()
            .client
            .subscribe_to_events(SubscribeToEventsRequest {
                identity_public_key: identity_public_key.serialize().to_vec(),
            })
            .await
        {
            Ok(stream) => stream,
            Err(e) => {
                error!("Failed to subscribe to server events: {}", e);
                tokio::select! {
                    _ = cancellation_token.changed() => {
                        info!("Cancellation token changed during backoff, stopping event subscription");
                        return;
                    }
                    _ = sleep(reconnect_interval) => {}
                }
                continue;
            }
        };

        loop {
            let message = tokio::select! {
                message = stream.message() => message,
                _ = cancellation_token.changed() => {
                    info!("Cancellation token changed while waiting for a message, stopping event subscription");
                    return;
                }
            };
            let response = match message {
                Ok(Some(event)) => event,
                Ok(None) => {
                    warn!("Server event stream closed, reconnecting...");
                    break;
                }
                Err(e) => {
                    error!("Error receiving event, reconnecting: {}", e);
                    break;
                }
            };

            let Some(event) = response.event else {
                warn!("Received empty event, skipping");
                continue;
            };

            let spark_event = match event {
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
                    SparkEvent::Transfer(Box::new(transfer))
                }
                Event::Deposit(deposit_event) => {
                    let Some(deposit) = deposit_event.deposit else {
                        warn!("Received empty deposit event, skipping");
                        continue;
                    };
                    debug!("Received deposit event with tree node id {}", deposit.id);
                    trace!("Received deposit event with tree node: {:?}", deposit);
                    let deposit: TreeNode = match deposit.try_into() {
                        Ok(deposit) => deposit,
                        Err(e) => {
                            error!("Failed to convert deposit event: {}", e);
                            continue;
                        }
                    };
                    SparkEvent::Deposit(Box::new(deposit))
                }
                Event::Connected(_) => {
                    debug!("Received connected event");
                    SparkEvent::Connected
                }
            };

            debug!("Emitting spark event: {spark_event}");
            trace!("Emitting spark event: {spark_event:?}");
            if publisher.send(spark_event).is_err() {
                error!(
                    "Failed to send spark event, all receivers dropped. Quitting event subscription."
                );
                return;
            }
        }

        if publisher.send(SparkEvent::Disconnected).is_err() {
            error!(
                "Failed to send disconnected event, all receivers dropped. Quitting event subscription."
            );
            return;
        }
    }
}
