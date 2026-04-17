#[derive(Debug, Clone)]
pub struct Zap {
    pub payment_hash: String,
    pub zap_request: String,
    pub zap_event: Option<String>,
    pub user_pubkey: String,
    pub invoice_expiry: i64,
    pub updated_at: i64,
    pub is_user_nostr_key: bool,
}

// -- Zap receipt enqueueing ----------------------------------------------------

use crate::repository::{LnurlRepository, LnurlRepositoryError, PendingZapReceipt};
use crate::time::now_millis;

/// Enqueue a single zap receipt for background publishing.
pub async fn enqueue_zap_receipt<DB>(
    db: &DB,
    payment_hash: &str,
) -> Result<(), LnurlRepositoryError>
where
    DB: LnurlRepository,
{
    let now = now_millis();
    let pending = PendingZapReceipt {
        payment_hash: payment_hash.to_string(),
        created_at: now,
        retry_count: 0,
        next_retry_at: now,
    };
    db.insert_pending_zap_receipt(&pending).await
}

/// Enqueue multiple zap receipts for background publishing.
pub async fn enqueue_zap_receipts<DB>(
    db: &DB,
    payment_hashes: &[String],
) -> Result<(), LnurlRepositoryError>
where
    DB: LnurlRepository,
{
    if payment_hashes.is_empty() {
        return Ok(());
    }
    let now = now_millis();
    let pending: Vec<PendingZapReceipt> = payment_hashes
        .iter()
        .map(|payment_hash| PendingZapReceipt {
            payment_hash: payment_hash.clone(),
            created_at: now,
            retry_count: 0,
            next_retry_at: now,
        })
        .collect();
    db.insert_pending_zap_receipt_batch(&pending).await
}

// -- Background processor for publishing zap receipts to nostr relays ----------

use nostr::{JsonUtil, TagStandard};
use tracing::{debug, error, warn};

/// Retry configuration for zap receipt publishing.
const BASE_RETRY_DELAY_MS: i64 = 30_000; // 30 seconds
const RETRY_MULTIPLIER: f64 = 1.5;

/// Maximum number of pending zap receipts to claim and process per poll cycle.
const ZAP_RECEIPT_BATCH_LIMIT: u32 = 4;
const MAX_RETRY_DURATION_MS: i64 = 14 * 24 * 60 * 60 * 1000; // 14 days
/// Maximum number of nostr relays to connect to when publishing zap receipts.
const MAX_NOSTR_RELAYS: usize = 10;

#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn next_retry_delay(retry_count: i32) -> i64 {
    (BASE_RETRY_DELAY_MS as f64 * RETRY_MULTIPLIER.powi(retry_count)) as i64
}

/// Start the zap receipt background processor.
pub fn start_background_processor<DB>(
    db: DB,
    nostr_keys: Option<&nostr::Keys>,
    trigger_rx: tokio::sync::watch::Receiver<()>,
) where
    DB: LnurlRepository + Clone + Send + Sync + 'static,
{
    let nostr_keys = nostr_keys.cloned();
    let mut trigger_rx = trigger_rx;
    tokio::spawn(async move {
        debug!("Zap receipt processor started");

        process_pending_zap_receipts(&db, nostr_keys.as_ref()).await;

        loop {
            tokio::select! {
                result = trigger_rx.changed() => {
                    if result.is_err() {
                        debug!("Zap receipt processor trigger channel closed, exiting");
                        return;
                    }
                }
                () = tokio::time::sleep(tokio::time::Duration::from_mins(1)) => {}
            }

            process_pending_zap_receipts(&db, nostr_keys.as_ref()).await;
        }
    });
}

/// Process pending zap receipts, fetching in batches until the queue is drained.
async fn process_pending_zap_receipts<DB>(db: &DB, nostr_keys: Option<&nostr::Keys>)
where
    DB: LnurlRepository + Clone + Send + Sync + 'static,
{
    loop {
        let pending = match db.take_pending_zap_receipts(ZAP_RECEIPT_BATCH_LIMIT).await {
            Ok(pending) => pending,
            Err(e) => {
                error!("Failed to get pending zap receipts: {}", e);
                return;
            }
        };

        if pending.is_empty() {
            return;
        }

        let count = pending.len();
        debug!("Background processor: processing {count} pending zap receipts");

        for item in pending {
            process_pending_zap_receipt(db, nostr_keys, &item).await;
        }

        // If we got fewer than the limit, the queue is drained.
        if count < ZAP_RECEIPT_BATCH_LIMIT as usize {
            return;
        }
    }
}

/// Process a single pending zap receipt: publish zap receipt if applicable.
#[allow(clippy::too_many_lines)]
async fn process_pending_zap_receipt<DB>(
    db: &DB,
    nostr_keys: Option<&nostr::Keys>,
    item: &crate::repository::PendingZapReceipt,
) where
    DB: LnurlRepository + Clone + Send + Sync + 'static,
{
    let payment_hash = &item.payment_hash;

    let Some(nostr_keys) = nostr_keys else {
        if let Err(e) = db.delete_pending_zap_receipt(payment_hash).await {
            error!(
                "Failed to delete pending zap receipt {}: {}",
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
        if let Err(e) = db.delete_pending_zap_receipt(payment_hash).await {
            error!(
                "Failed to delete expired pending zap receipt {}: {}",
                payment_hash, e
            );
        }
        return;
    }

    // Fetch both zap and invoice in a single query
    let (zap, invoice) = match db.get_zap_and_invoice_by_payment_hash(payment_hash).await {
        Ok(result) => result,
        Err(e) => {
            error!(
                "Failed to get zap and invoice for payment hash {}: {}",
                payment_hash, e
            );
            return;
        }
    };

    let Some(zap) = zap else {
        // No zap record - just remove from queue (non-zap invoice)
        debug!(
            "No zap found for payment hash {}, removing from queue",
            payment_hash
        );
        if let Err(e) = db.delete_pending_zap_receipt(payment_hash).await {
            error!(
                "Failed to delete pending zap receipt {}: {}",
                payment_hash, e
            );
        }
        return;
    };

    // If zap receipt already exists, remove from queue
    if zap.zap_event.is_some() {
        debug!(
            "Zap receipt already exists for payment hash {}, removing from queue",
            payment_hash
        );
        if let Err(e) = db.delete_pending_zap_receipt(payment_hash).await {
            error!(
                "Failed to delete pending zap receipt {}: {}",
                payment_hash, e
            );
        }
        return;
    }

    let Some(invoice) = invoice else {
        debug!(
            "Invoice not found for payment hash {}, removing from queue",
            payment_hash
        );
        if let Err(e) = db.delete_pending_zap_receipt(payment_hash).await {
            error!(
                "Failed to delete pending zap receipt {}: {}",
                payment_hash, e
            );
        }
        return;
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
            if let Err(e) = db.delete_pending_zap_receipt(payment_hash).await {
                error!(
                    "Failed to delete pending zap receipt {}: {}",
                    payment_hash, e
                );
            }
        }
        Err(e) => {
            warn!(
                "Failed to publish zap receipt for {}: {}, will retry",
                payment_hash, e
            );

            let retry_count = item.retry_count.saturating_add(1);
            let next_retry_at = now.saturating_add(next_retry_delay(retry_count));

            if let Err(e) = db
                .update_pending_zap_receipt_retry(payment_hash, retry_count, next_retry_at)
                .await
            {
                error!(
                    "Failed to update retry for pending zap receipt {}: {}",
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

    let relays: Vec<_> = zap_request
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
        .take(MAX_NOSTR_RELAYS)
        .collect();

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

#[cfg(test)]
mod shared_tests {
    use crate::repository::{LnurlRepository, PendingZapReceipt};
    use crate::time::now_millis;

    pub async fn take_pending_zap_receipts_claims_items<DB>(db: &DB)
    where
        DB: LnurlRepository + Clone + Send + Sync + 'static,
    {
        let now = now_millis();

        let item = PendingZapReceipt {
            payment_hash: "claim_test_hash".to_string(),
            created_at: now,
            retry_count: 0,
            next_retry_at: now,
        };
        db.insert_pending_zap_receipt(&item).await.unwrap();

        // First call claims the item
        let claimed = db.take_pending_zap_receipts(100).await.unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].payment_hash, "claim_test_hash");

        // Second call cannot claim the same item (it was just claimed)
        let claimed_again = db.take_pending_zap_receipts(100).await.unwrap();
        assert!(
            claimed_again.is_empty(),
            "recently claimed items should not be claimable again"
        );
    }

    pub async fn take_pending_zap_receipts_respects_next_retry_at<DB>(db: &DB)
    where
        DB: LnurlRepository + Clone + Send + Sync + 'static,
    {
        let future_item = PendingZapReceipt {
            payment_hash: "future_hash".to_string(),
            created_at: now_millis(),
            retry_count: 1,
            next_retry_at: now_millis().saturating_add(999_999_999),
        };
        db.insert_pending_zap_receipt(&future_item).await.unwrap();

        let claimed = db.take_pending_zap_receipts(100).await.unwrap();
        assert!(
            claimed.is_empty(),
            "items with future next_retry_at should not be claimed"
        );
    }

    pub async fn take_pending_zap_receipts_respects_limit<DB>(db: &DB)
    where
        DB: LnurlRepository + Clone + Send + Sync + 'static,
    {
        let now = now_millis();

        for i in 0..5 {
            let item = PendingZapReceipt {
                payment_hash: format!("limit_test_{i}"),
                created_at: now,
                retry_count: 0,
                next_retry_at: now,
            };
            db.insert_pending_zap_receipt(&item).await.unwrap();
        }

        // Request at most 2
        let claimed = db.take_pending_zap_receipts(2).await.unwrap();
        assert_eq!(claimed.len(), 2, "should only return up to the limit");

        // Remaining 3 are still unclaimed
        let claimed2 = db.take_pending_zap_receipts(10).await.unwrap();
        assert_eq!(
            claimed2.len(),
            3,
            "should claim the remaining unclaimed items"
        );
    }
}

#[cfg(test)]
mod sqlite_tests {
    use super::shared_tests;

    async fn setup_test_db() -> crate::sqlite::LnurlRepository {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect(":memory:")
            .await
            .unwrap();
        crate::sqlite::run_migrations(&pool).await.unwrap();
        crate::sqlite::LnurlRepository::new(pool)
    }

    #[tokio::test]
    async fn take_pending_zap_receipts_claims_items() {
        let db = setup_test_db().await;
        shared_tests::take_pending_zap_receipts_claims_items(&db).await;
    }

    #[tokio::test]
    async fn take_pending_zap_receipts_respects_next_retry_at() {
        let db = setup_test_db().await;
        shared_tests::take_pending_zap_receipts_respects_next_retry_at(&db).await;
    }

    #[tokio::test]
    async fn take_pending_zap_receipts_respects_limit() {
        let db = setup_test_db().await;
        shared_tests::take_pending_zap_receipts_respects_limit(&db).await;
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::shared_tests;

    async fn setup_test_db() -> Option<crate::postgresql::LnurlRepository> {
        let url = std::env::var("LNURL_TEST_POSTGRES_URL").ok()?;
        let pool = sqlx::PgPool::connect(&url).await.ok()?;
        crate::postgresql::run_migrations(&pool).await.ok()?;

        sqlx::query("DELETE FROM pending_zap_receipts")
            .execute(&pool)
            .await
            .ok()?;

        Some(crate::postgresql::LnurlRepository::new(pool))
    }

    #[tokio::test]
    async fn take_pending_zap_receipts_claims_items() {
        let Some(db) = setup_test_db().await else {
            return;
        };
        shared_tests::take_pending_zap_receipts_claims_items(&db).await;
    }

    #[tokio::test]
    async fn take_pending_zap_receipts_respects_next_retry_at() {
        let Some(db) = setup_test_db().await else {
            return;
        };
        shared_tests::take_pending_zap_receipts_respects_next_retry_at(&db).await;
    }

    #[tokio::test]
    async fn take_pending_zap_receipts_respects_limit() {
        let Some(db) = setup_test_db().await else {
            return;
        };
        shared_tests::take_pending_zap_receipts_respects_limit(&db).await;
    }
}
