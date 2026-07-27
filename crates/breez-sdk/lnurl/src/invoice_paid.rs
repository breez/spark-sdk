use bitcoin::hashes::{Hash, sha256};
use tokio::sync::watch;
use tracing::{debug, error};

use crate::repository::{Invoice, LnurlRepository, LnurlRepositoryError};
use crate::time::now_millis;
use crate::webhooks::{WebhookRepository, WebhookService};

#[derive(Debug, thiserror::Error)]
pub enum HandleInvoicePaidError {
    #[error("invalid preimage: {0}")]
    InvalidPreimage(String),
    #[error(transparent)]
    Repository(#[from] LnurlRepositoryError),
}

/// Verify that the SHA-256 hash of the preimage matches the expected payment hash.
/// Both values are hex-encoded strings.
fn verify_preimage(payment_hash: &str, preimage: &str) -> Result<(), HandleInvoicePaidError> {
    let preimage_bytes = hex::decode(preimage).map_err(|e| {
        HandleInvoicePaidError::InvalidPreimage(format!("could not hex-decode preimage: {e}"))
    })?;
    let computed_hash = sha256::Hash::hash(&preimage_bytes).to_string();
    if computed_hash != payment_hash {
        return Err(HandleInvoicePaidError::InvalidPreimage(
            "preimage does not match payment hash".to_string(),
        ));
    }
    Ok(())
}

/// Handle an invoice being paid by storing the preimage and queueing for background processing.
pub async fn handle_invoice_paid<DB>(
    db: &DB,
    webhook_service: &WebhookService<DB>,
    payment_hash: &str,
    preimage: &str,
    amount_received_sat: Option<i64>,
    trigger: &watch::Sender<()>,
) -> Result<(), HandleInvoicePaidError>
where
    DB: LnurlRepository + WebhookRepository + Clone + Send + Sync + 'static,
{
    verify_preimage(payment_hash, preimage)?;

    let now = now_millis();

    // Get the existing invoice
    let Some(mut invoice) = db.get_invoice_by_payment_hash(payment_hash).await? else {
        debug!(
            "Invoice not found for payment hash {}, cannot mark as paid",
            payment_hash
        );
        return Ok(());
    };

    if invoice.preimage.is_none() {
        invoice.preimage = Some(preimage.to_string());
        invoice.amount_received_sat = amount_received_sat;
        invoice.updated_at = now;
        db.upsert_invoice(&invoice).await?;
        debug!("Stored preimage for invoice {}", payment_hash);
    }

    // Enqueue on every call, not just when the preimage was newly stored, so the
    // zap receipt is still queued if a prior attempt stored the preimage but
    // failed before enqueueing. Idempotent via ON CONFLICT DO NOTHING, and the
    // background publisher drops receipts that were already published.
    crate::zap::enqueue_zap_receipt(db, payment_hash).await?;

    // Notify for all payment hashes, not just newly-affected ones, so that
    // webhooks are delivered even if the server crashed after storing preimages
    // but before enqueueing webhooks. Idempotent via ON CONFLICT DO NOTHING.
    if let Err(e) =
        crate::webhook_notify::notify_webhooks(db, webhook_service, &[payment_hash.to_string()])
            .await
    {
        error!("Failed to enqueue webhook for {}: {}", payment_hash, e);
    }

    if trigger.send(()).is_err() {
        error!("Failed to trigger background processor - receiver dropped");
    }

    Ok(())
}

/// Create a new invoice record for LUD-21 and NIP-57 support.
pub async fn create_invoice<DB>(
    db: &DB,
    payment_hash: &str,
    user_pubkey: &str,
    invoice: &str,
    invoice_expiry: i64,
    domain: &str,
) -> Result<(), LnurlRepositoryError>
where
    DB: LnurlRepository + Clone + Send + Sync + 'static,
{
    let now = now_millis();
    let invoice_record = Invoice {
        payment_hash: payment_hash.to_string(),
        user_pubkey: user_pubkey.to_string(),
        invoice: invoice.to_string(),
        preimage: None,
        invoice_expiry,
        created_at: now,
        updated_at: now,
        domain: Some(domain.to_string()),
        amount_received_sat: None,
    };
    db.upsert_invoice(&invoice_record).await?;
    debug!("Created invoice record for payment hash {}", payment_hash);
    Ok(())
}

#[cfg(test)]
mod test_helpers {
    use super::*;
    use bitcoin::secp256k1::{Secp256k1, SecretKey};
    use lightning_invoice::{Currency, InvoiceBuilder};

    /// Generate a valid bolt11 invoice for the given preimage bytes.
    /// Returns (`preimage_hex`, `payment_hash_hex`, `invoice_string`).
    pub fn generate_test_invoice(preimage_bytes: &[u8; 32]) -> (String, String, String) {
        let preimage_hex = hex::encode(preimage_bytes);
        let payment_hash = sha256::Hash::hash(preimage_bytes);

        let secp = Secp256k1::new();
        let key = SecretKey::from_slice(&[42u8; 32]).unwrap();

        let invoice = InvoiceBuilder::new(Currency::Regtest)
            .description("test invoice".to_string())
            .payment_hash(payment_hash)
            .payment_secret(lightning_invoice::PaymentSecret([0u8; 32]))
            .current_timestamp()
            .min_final_cltv_expiry_delta(144)
            .amount_milli_satoshis(1_000_000)
            .build_signed(|hash| secp.sign_ecdsa_recoverable(hash, &key))
            .unwrap();

        (preimage_hex, payment_hash.to_string(), invoice.to_string())
    }
}

/// Shared test logic that runs against any `LnurlRepository` implementation.
#[cfg(test)]
mod shared_tests {
    use super::*;

    use super::test_helpers::generate_test_invoice;

    /// Regression: a prior attempt stored the preimage but failed before
    /// enqueueing the zap receipt. A subsequent call must still enqueue it,
    /// rather than skip because the preimage is already present.
    pub async fn invoice_paid_enqueues_zap_when_preimage_already_stored<DB>(db: &DB)
    where
        DB: LnurlRepository + WebhookRepository + Clone + Send + Sync + 'static,
    {
        let (trigger, _rx) = watch::channel(());
        let webhook_service = WebhookService::new(db.clone());

        let preimage_bytes = [7u8; 32];
        let (preimage_hex, payment_hash, invoice_str) = generate_test_invoice(&preimage_bytes);
        let user_pubkey = "test_user_pubkey";

        db.upsert_zap(&crate::zap::Zap {
            payment_hash: payment_hash.clone(),
            zap_request: r#"{"kind":9734}"#.to_string(),
            zap_event: None,
            user_pubkey: user_pubkey.to_string(),
            invoice_expiry: i64::MAX,
            updated_at: 1000,
            is_user_nostr_key: false,
        })
        .await
        .unwrap();

        // Simulate the stuck state: preimage already stored, no zap enqueued.
        db.upsert_invoice(&Invoice {
            payment_hash: payment_hash.clone(),
            user_pubkey: user_pubkey.to_string(),
            invoice: invoice_str,
            preimage: Some(preimage_hex.clone()),
            invoice_expiry: i64::MAX,
            created_at: 1000,
            updated_at: 1000,
            domain: None,
            amount_received_sat: None,
        })
        .await
        .unwrap();

        handle_invoice_paid(
            db,
            &webhook_service,
            &payment_hash,
            &preimage_hex,
            None,
            &trigger,
        )
        .await
        .unwrap();

        let pending = db.take_pending_zap_receipts(100).await.unwrap();
        assert!(
            pending.iter().any(|p| p.payment_hash == payment_hash),
            "zap receipt must be enqueued even when the preimage was already stored"
        );
    }

    pub async fn get_or_create_setting_returns_default_on_first_call<DB>(db: &DB)
    where
        DB: LnurlRepository + Clone + Send + Sync + 'static,
    {
        let value = db
            .get_or_create_setting("webhook_secret", "my_secret")
            .await
            .unwrap();
        assert_eq!(value, "my_secret");
    }

    pub async fn get_or_create_setting_returns_existing_on_subsequent_calls<DB>(db: &DB)
    where
        DB: LnurlRepository + Clone + Send + Sync + 'static,
    {
        let first = db
            .get_or_create_setting("webhook_secret", "first_secret")
            .await
            .unwrap();
        let second = db
            .get_or_create_setting("webhook_secret", "different_secret")
            .await
            .unwrap();
        assert_eq!(first, "first_secret");
        assert_eq!(
            second, "first_secret",
            "should return the first value, not the new default"
        );
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::shared_tests;
    use crate::test_support::test_db;

    #[tokio::test]
    async fn invoice_paid_enqueues_zap_when_preimage_already_stored() {
        let db = test_db("invoice_paid_enqueues_zap").await;
        shared_tests::invoice_paid_enqueues_zap_when_preimage_already_stored(&db).await;
    }

    #[tokio::test]
    async fn get_or_create_setting_returns_default_on_first_call() {
        let db = test_db("setting_default_on_first_call").await;
        shared_tests::get_or_create_setting_returns_default_on_first_call(&db).await;
    }

    #[tokio::test]
    async fn get_or_create_setting_returns_existing_on_subsequent_calls() {
        let db = test_db("setting_existing_on_subsequent").await;
        shared_tests::get_or_create_setting_returns_existing_on_subsequent_calls(&db).await;
    }
}
