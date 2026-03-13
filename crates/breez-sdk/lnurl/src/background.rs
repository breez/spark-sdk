use nostr::{JsonUtil, TagStandard};
use tokio::sync::watch;
use tracing::{debug, error, warn};

use crate::repository::LnurlRepository;
use crate::time::now_millis;
use crate::zap::Zap;

/// Retry configuration for zap receipt publishing
const BASE_RETRY_DELAY_MS: i64 = 30_000; // 30 seconds
const RETRY_MULTIPLIER: f64 = 1.5;
const MAX_RETRY_DURATION_MS: i64 = 14 * 24 * 60 * 60 * 1000; // 14 days

/// Start the background processor that handles the `newly_paid` queue.
/// This processor publishes zap receipts for paid invoices with retry logic.
pub fn start_background_processor<DB>(
    db: DB,
    nostr_keys: Option<nostr::Keys>,
    mut trigger_rx: watch::Receiver<()>,
) where
    DB: LnurlRepository + Clone + Send + Sync + 'static,
{
    tokio::spawn(async move {
        debug!("Background processor started");

        // Process any pending items on startup
        process_newly_paid_queue(&db, nostr_keys.as_ref()).await;

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

            process_newly_paid_queue(&db, nostr_keys.as_ref()).await;
        }
    });
}

/// Process all pending items in the `newly_paid` queue.
async fn process_newly_paid_queue<DB>(db: &DB, nostr_keys: Option<&nostr::Keys>)
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

    debug!(
        "Background processor: found {} pending newly paid items",
        pending.len()
    );

    for item in pending {
        process_newly_paid_item(db, nostr_keys, &item).await;
    }
}

/// Process a single `newly_paid` item: publish zap receipt if applicable.
#[allow(clippy::too_many_lines)]
async fn process_newly_paid_item<DB>(
    db: &DB,
    nostr_keys: Option<&nostr::Keys>,
    item: &crate::repository::NewlyPaid,
) where
    DB: LnurlRepository + Clone + Send + Sync + 'static,
{
    let payment_hash = &item.payment_hash;

    let Some(nostr_keys) = nostr_keys else {
        if let Err(e) = db.delete_newly_paid(payment_hash).await {
            error!(
                "Failed to delete expired newly paid {}: {}",
                payment_hash, e
            );
        }

        return;
    };

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
