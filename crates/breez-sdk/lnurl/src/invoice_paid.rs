use bitcoin::hashes::{Hash, sha256};
use lightning_invoice::Bolt11Invoice;
use lnurl_models::PaidInvoice;
use std::str::FromStr;
use tokio::sync::watch;
use tracing::{debug, error};

use crate::repository::{Invoice, LnurlRepository, LnurlRepositoryError, NewlyPaid};
use crate::time::now_millis;

#[derive(Debug, thiserror::Error)]
pub enum HandleInvoicePaidError {
    #[error("invalid invoice: {0}")]
    InvalidInvoice(String),
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
    payment_hash: &str,
    preimage: &str,
    trigger: &watch::Sender<()>,
) -> Result<(), HandleInvoicePaidError>
where
    DB: LnurlRepository + Clone + Send + Sync + 'static,
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

    // Check if already paid
    if invoice.preimage.is_some() {
        debug!("Invoice {} already has preimage, skipping", payment_hash);
        return Ok(());
    }

    // Store the preimage
    invoice.preimage = Some(preimage.to_string());
    invoice.updated_at = now;
    db.upsert_invoice(&invoice).await?;
    debug!("Stored preimage for invoice {}", payment_hash);

    // Queue for background processing (zap receipt publishing)
    let newly_paid = NewlyPaid {
        payment_hash: payment_hash.to_string(),
        created_at: now,
        retry_count: 0,
        next_retry_at: now, // Process immediately
    };
    db.insert_newly_paid(&newly_paid).await?;
    debug!("Queued invoice {} for background processing", payment_hash);

    // Trigger the background processor
    // Using watch channel so multiple triggers result in a single processing run
    if trigger.send(()).is_err() {
        error!("Failed to trigger background processor - receiver dropped");
    }

    Ok(())
}

/// Handle multiple invoices being paid by storing preimages and queueing for background
/// processing in batch. Only processes invoices for payment hashes the server already
/// knows about (has an existing invoice, zap, or sender comment record).
/// Existing invoices are only updated if they belong to the same user and don't already
/// have a preimage.
pub async fn handle_invoices_paid<DB>(
    db: &DB,
    items: &[PaidInvoice],
    user_pubkey: &str,
    trigger: &watch::Sender<()>,
) -> Result<(), HandleInvoicePaidError>
where
    DB: LnurlRepository + Clone + Send + Sync + 'static,
{
    let now = now_millis();
    let mut invoices = Vec::with_capacity(items.len());

    for item in items {
        let preimage_bytes = hex::decode(&item.preimage).map_err(|e| {
            HandleInvoicePaidError::InvalidPreimage(format!("could not hex-decode preimage: {e}"))
        })?;
        let payment_hash = sha256::Hash::hash(&preimage_bytes).to_string();

        let bolt11 = Bolt11Invoice::from_str(&item.invoice).map_err(|e| {
            HandleInvoicePaidError::InvalidInvoice(format!("invalid bolt11 invoice: {e}"))
        })?;

        if bolt11.payment_hash().to_string() != payment_hash {
            return Err(HandleInvoicePaidError::InvalidPreimage(format!(
                "invoice payment hash does not match preimage for hash {payment_hash}"
            )));
        }

        let invoice_expiry = bolt11
            .expires_at()
            .map_or(0, |t| i64::try_from(t.as_millis()).unwrap_or(i64::MAX));

        invoices.push(Invoice {
            payment_hash,
            user_pubkey: user_pubkey.to_string(),
            invoice: item.invoice.clone(),
            preimage: Some(item.preimage.clone()),
            invoice_expiry,
            created_at: now,
            updated_at: now,
        });
    }

    // Only process invoices for payment hashes the server already knows about
    // (has an existing invoice, zap, or sender comment).
    let all_hashes: Vec<String> = invoices.iter().map(|i| i.payment_hash.clone()).collect();
    let known_hashes: std::collections::HashSet<String> = db
        .filter_known_payment_hashes(&all_hashes)
        .await?
        .into_iter()
        .collect();

    let invoices: Vec<Invoice> = invoices
        .into_iter()
        .filter(|i| known_hashes.contains(&i.payment_hash))
        .collect();

    if invoices.is_empty() {
        debug!("No known payment hashes in invoices-paid request, skipping");
        return Ok(());
    }

    let affected = db.upsert_invoices_paid(&invoices).await?;
    if affected.is_empty() {
        return Ok(());
    }
    debug!("Stored preimages for {} invoices", affected.len());

    let newly_paid_items: Vec<NewlyPaid> = affected
        .iter()
        .map(|payment_hash| NewlyPaid {
            payment_hash: payment_hash.clone(),
            created_at: now,
            retry_count: 0,
            next_retry_at: now,
        })
        .collect();

    db.insert_newly_paid_batch(&newly_paid_items).await?;
    debug!(
        "Queued {} invoices for background processing",
        newly_paid_items.len()
    );

    // Trigger the background processor once
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
    };
    db.upsert_invoice(&invoice_record).await?;
    debug!("Created invoice record for payment hash {}", payment_hash);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository::LnurlSenderComment;
    use bitcoin::secp256k1::{Secp256k1, SecretKey};
    use lightning_invoice::{Currency, InvoiceBuilder};

    async fn setup_test_db() -> crate::sqlite::LnurlRepository {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect(":memory:")
            .await
            .unwrap();
        crate::sqlite::run_migrations(&pool).await.unwrap();
        crate::sqlite::LnurlRepository::new(pool)
    }

    /// Generate a valid bolt11 invoice for the given preimage bytes.
    /// Returns (`preimage_hex`, `payment_hash_hex`, `invoice_string`).
    fn generate_test_invoice(preimage_bytes: &[u8; 32]) -> (String, String, String) {
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

    /// When the server has a sender comment for a payment hash but no invoice record,
    /// `handle_invoices_paid` should create the invoice from the client-provided bolt11.
    #[tokio::test]
    async fn invoices_paid_creates_invoice_when_only_comment_exists() {
        let db = setup_test_db().await;
        let (trigger, _rx) = watch::channel(());

        let preimage_bytes = [1u8; 32];
        let (preimage_hex, payment_hash, invoice_str) = generate_test_invoice(&preimage_bytes);
        let user_pubkey = "test_user_pubkey";

        // Server has a sender comment but no invoice record for this payment hash.
        db.insert_lnurl_sender_comment(&LnurlSenderComment {
            comment: "hello from sender".to_string(),
            payment_hash: payment_hash.clone(),
            user_pubkey: user_pubkey.to_string(),
            updated_at: 1000,
        })
        .await
        .unwrap();

        // Verify no invoice exists yet.
        assert!(
            db.get_invoice_by_payment_hash(&payment_hash)
                .await
                .unwrap()
                .is_none()
        );

        // Client posts the paid invoice.
        handle_invoices_paid(
            &db,
            &[PaidInvoice {
                preimage: preimage_hex.clone(),
                invoice: invoice_str.clone(),
            }],
            user_pubkey,
            &trigger,
        )
        .await
        .unwrap();

        // The server should now have the invoice stored.
        let stored = db
            .get_invoice_by_payment_hash(&payment_hash)
            .await
            .unwrap()
            .expect("invoice should have been created");
        assert_eq!(stored.preimage.as_deref(), Some(preimage_hex.as_str()));
        assert_eq!(stored.user_pubkey, user_pubkey);
        assert_eq!(stored.invoice, invoice_str);
    }

    /// When the server has a zap for a payment hash but no invoice record,
    /// `handle_invoices_paid` should create the invoice from the client-provided bolt11.
    #[tokio::test]
    async fn invoices_paid_creates_invoice_when_only_zap_exists() {
        let db = setup_test_db().await;
        let (trigger, _rx) = watch::channel(());

        let preimage_bytes = [2u8; 32];
        let (preimage_hex, payment_hash, invoice_str) = generate_test_invoice(&preimage_bytes);
        let user_pubkey = "test_user_pubkey";

        // Server has a zap but no invoice record for this payment hash.
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

        // Verify no invoice exists yet.
        assert!(
            db.get_invoice_by_payment_hash(&payment_hash)
                .await
                .unwrap()
                .is_none()
        );

        // Client posts the paid invoice.
        handle_invoices_paid(
            &db,
            &[PaidInvoice {
                preimage: preimage_hex.clone(),
                invoice: invoice_str.clone(),
            }],
            user_pubkey,
            &trigger,
        )
        .await
        .unwrap();

        // The server should now have the invoice stored.
        let stored = db
            .get_invoice_by_payment_hash(&payment_hash)
            .await
            .unwrap()
            .expect("invoice should have been created");
        assert_eq!(stored.preimage.as_deref(), Some(preimage_hex.as_str()));
        assert_eq!(stored.user_pubkey, user_pubkey);
        assert_eq!(stored.invoice, invoice_str);
    }

    /// When the server has no prior record for a payment hash (no invoice, zap, or comment),
    /// `handle_invoices_paid` should silently ignore it and not create an invoice.
    #[tokio::test]
    async fn invoices_paid_ignores_unknown_payment_hash() {
        let db = setup_test_db().await;
        let (trigger, _rx) = watch::channel(());

        let preimage_bytes = [3u8; 32];
        let (preimage_hex, payment_hash, invoice_str) = generate_test_invoice(&preimage_bytes);
        let user_pubkey = "test_user_pubkey";

        // No prior records exist for this payment hash.
        handle_invoices_paid(
            &db,
            &[PaidInvoice {
                preimage: preimage_hex,
                invoice: invoice_str,
            }],
            user_pubkey,
            &trigger,
        )
        .await
        .unwrap();

        // The invoice should NOT have been created.
        assert!(
            db.get_invoice_by_payment_hash(&payment_hash)
                .await
                .unwrap()
                .is_none(),
            "invoice should not be created for unknown payment hash"
        );
    }

    /// When a batch contains both known and unknown payment hashes,
    /// only the known ones should be stored.
    #[tokio::test]
    async fn invoices_paid_filters_mixed_batch() {
        let db = setup_test_db().await;
        let (trigger, _rx) = watch::channel(());
        let user_pubkey = "test_user_pubkey";

        // Known: has a sender comment.
        let known_preimage = [4u8; 32];
        let (known_hex, known_hash, known_invoice) = generate_test_invoice(&known_preimage);
        db.insert_lnurl_sender_comment(&LnurlSenderComment {
            comment: "known".to_string(),
            payment_hash: known_hash.clone(),
            user_pubkey: user_pubkey.to_string(),
            updated_at: 1000,
        })
        .await
        .unwrap();

        // Unknown: no prior records.
        let unknown_preimage = [5u8; 32];
        let (unknown_hex, unknown_hash, unknown_invoice) = generate_test_invoice(&unknown_preimage);

        handle_invoices_paid(
            &db,
            &[
                PaidInvoice {
                    preimage: known_hex.clone(),
                    invoice: known_invoice.clone(),
                },
                PaidInvoice {
                    preimage: unknown_hex,
                    invoice: unknown_invoice,
                },
            ],
            user_pubkey,
            &trigger,
        )
        .await
        .unwrap();

        // Known invoice should be stored.
        let stored = db
            .get_invoice_by_payment_hash(&known_hash)
            .await
            .unwrap()
            .expect("known invoice should have been created");
        assert_eq!(stored.preimage.as_deref(), Some(known_hex.as_str()));

        // Unknown invoice should NOT be stored.
        assert!(
            db.get_invoice_by_payment_hash(&unknown_hash)
                .await
                .unwrap()
                .is_none(),
            "unknown invoice should not be created"
        );
    }
}
