use lnurl_models::ListMetadataMetadata;

use crate::user::User;
use crate::zap::Zap;

#[derive(Debug, thiserror::Error)]
pub enum LnurlRepositoryError {
    #[error("name taken")]
    NameTaken,
    #[error("source user does not own this username")]
    SourceNotOwner,
    #[error("database error: {0}")]
    General(anyhow::Error),
}

pub struct LnurlSenderComment {
    pub comment: String,
    pub payment_hash: String,
    pub user_pubkey: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct Invoice {
    pub payment_hash: String,
    pub user_pubkey: String,
    pub invoice: String,
    pub preimage: Option<String>,
    pub invoice_expiry: i64,
    pub created_at: i64,
    pub updated_at: i64,
    /// The domain this invoice was created for, if any.
    pub domain: Option<String>,
    /// Amount received in satoshis (from the HTLC). NULL when unknown.
    pub amount_received_sat: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct PendingZapReceipt {
    pub payment_hash: String,
    pub created_at: i64,
    pub retry_count: i32,
    pub next_retry_at: i64,
}

#[derive(Debug, Clone)]
pub struct DomainConfig {
    pub domain: String,
    /// The domain's own Breez API key, if set.
    pub api_key: Option<String>,
    /// The cached partner JWT if one has been fetched and persisted.
    pub jwt: Option<String>,
}

#[async_trait::async_trait]
pub trait LnurlRepository {
    async fn delete_user(&self, domain: &str, pubkey: &str) -> Result<(), LnurlRepositoryError>;
    async fn get_user_by_name(
        &self,
        domain: &str,
        name: &str,
    ) -> Result<Option<User>, LnurlRepositoryError>;
    async fn get_user_by_pubkey(
        &self,
        domain: &str,
        pubkey: &str,
    ) -> Result<Option<User>, LnurlRepositoryError>;
    async fn upsert_user(&self, user: &User) -> Result<(), LnurlRepositoryError>;

    /// Atomically transfer ownership of `username` in `domain` from `from_pubkey`
    /// to `to_pubkey`, replacing any existing row for `to_pubkey`.
    /// Returns [`LnurlRepositoryError::SourceNotOwner`] if `from_pubkey` does not
    /// currently own `username` in `domain`.
    async fn transfer_username(
        &self,
        domain: &str,
        from_pubkey: &str,
        to_pubkey: &str,
        username: &str,
        description: &str,
    ) -> Result<(), LnurlRepositoryError>;

    async fn upsert_zap(&self, zap: &Zap) -> Result<(), LnurlRepositoryError>;
    async fn insert_lnurl_sender_comment(
        &self,
        comment: &LnurlSenderComment,
    ) -> Result<(), LnurlRepositoryError>;
    async fn get_metadata_by_pubkey(
        &self,
        pubkey: &str,
        offset: u32,
        limit: u32,
        updated_after: Option<i64>,
    ) -> Result<Vec<ListMetadataMetadata>, LnurlRepositoryError>;

    /// Get all allowed domains and their optional Breez API keys.
    async fn list_domains(&self) -> Result<Vec<DomainConfig>, LnurlRepositoryError>;

    /// Insert a domain if it doesn't already exist
    async fn add_domain(&self, domain: &str) -> Result<(), LnurlRepositoryError>;

    /// Store the cached partner JWT for a domain.
    async fn set_domain_jwt(&self, domain: &str, jwt: &str) -> Result<(), LnurlRepositoryError>;

    /// Filter a list of payment hashes to only those the server already knows about
    /// (i.e. have an existing invoice, zap, or sender comment record).
    async fn filter_known_payment_hashes(
        &self,
        payment_hashes: &[String],
    ) -> Result<Vec<String>, LnurlRepositoryError>;

    /// Insert or update an invoice
    async fn upsert_invoice(&self, invoice: &Invoice) -> Result<(), LnurlRepositoryError>;

    /// Batch upsert invoices with preimages. Inserts new records, or updates existing
    /// ones only if they belong to the same user and don't already have a preimage.
    /// Returns payment hashes that were actually inserted or updated.
    async fn upsert_invoices_paid(
        &self,
        invoices: &[Invoice],
    ) -> Result<Vec<String>, LnurlRepositoryError>;

    /// Get an invoice by payment hash
    async fn get_invoice_by_payment_hash(
        &self,
        payment_hash: &str,
    ) -> Result<Option<Invoice>, LnurlRepositoryError>;

    /// Get both the zap and invoice for a payment hash in a single query
    async fn get_zap_and_invoice_by_payment_hash(
        &self,
        payment_hash: &str,
    ) -> Result<(Option<Zap>, Option<Invoice>), LnurlRepositoryError>;
    /// Insert a pending zap receipt into the queue
    async fn insert_pending_zap_receipt(
        &self,
        pending: &PendingZapReceipt,
    ) -> Result<(), LnurlRepositoryError>;

    /// Batch insert pending zap receipts into the queue
    async fn insert_pending_zap_receipt_batch(
        &self,
        pending: &[PendingZapReceipt],
    ) -> Result<(), LnurlRepositoryError>;

    /// Get pending zap receipts ready for processing (`next_retry_at` <= now),
    /// atomically claiming them. Items already claimed by another instance
    /// within the last 5 minutes are skipped.
    async fn take_pending_zap_receipts(
        &self,
        limit: u32,
    ) -> Result<Vec<PendingZapReceipt>, LnurlRepositoryError>;

    /// Update retry count and next retry time for a pending zap receipt
    async fn update_pending_zap_receipt_retry(
        &self,
        payment_hash: &str,
        retry_count: i32,
        next_retry_at: i64,
    ) -> Result<(), LnurlRepositoryError>;

    /// Delete a pending zap receipt from the queue
    async fn delete_pending_zap_receipt(
        &self,
        payment_hash: &str,
    ) -> Result<(), LnurlRepositoryError>;

    /// Get or create a setting. If the key doesn't exist, insert the default value.
    /// Returns the current value (either existing or newly inserted).
    async fn get_or_create_setting(
        &self,
        key: &str,
        default_value: &str,
    ) -> Result<String, LnurlRepositoryError>;

    /// Get data needed to build webhook payloads for the given payment hashes.
    /// Joins invoices, users, `sender_comments`, and `domain_webhooks`.
    /// Returns rows for invoices that have a domain and a preimage.
    async fn get_webhook_payloads(
        &self,
        payment_hashes: &[String],
    ) -> Result<Vec<WebhookPayloadData>, LnurlRepositoryError>;
}

/// Data returned by the webhook enqueue query.
pub struct WebhookPayloadData {
    pub payment_hash: String,
    pub user_pubkey: String,
    pub invoice: String,
    pub preimage: String,
    pub amount_received_sat: Option<i64>,
    pub lightning_address: Option<String>,
    pub sender_comment: Option<String>,
    pub domain: String,
}

/// Backend-agnostic tests for the domain-attribution repository methods, run
/// against both the `SQLite` and `PostgreSQL` implementations. Assertions look
/// up domains by name rather than by count, so they tolerate a shared test
/// database with rows from other tests.
#[cfg(test)]
pub mod shared_tests {
    use super::{LnurlRepository, LnurlRepositoryError};
    use crate::user::User;

    /// Upserting a name already owned by a different pubkey returns `NameTaken`
    /// and leaves the existing owner's row intact, rather than replacing it.
    pub async fn registering_taken_name_with_other_pubkey_is_rejected<DB>(db: &DB)
    where
        DB: LnurlRepository + Clone + Send + Sync + 'static,
    {
        db.upsert_user(&User {
            domain: "a.com".into(),
            pubkey: "aaaa".into(),
            name: "alice".into(),
            description: "alice".into(),
        })
        .await
        .unwrap();

        let result = db
            .upsert_user(&User {
                domain: "a.com".into(),
                pubkey: "bbbb".into(),
                name: "alice".into(),
                description: "bob".into(),
            })
            .await;
        assert!(
            matches!(result, Err(LnurlRepositoryError::NameTaken)),
            "expected NameTaken, got {result:?}"
        );

        let owner = db
            .get_user_by_name("a.com", "alice")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            owner.pubkey, "aaaa",
            "existing owner was replaced, now resolves to pubkey {}",
            owner.pubkey
        );
    }

    /// `list_domains` surfaces a domain's `api_key` and reports `None` for one
    /// with no key, added via `add_domain`. The caller seeds `a.com` with an
    /// `api_key` (`key-a`) first, since setting a key is a direct row write with
    /// no trait method (admins manage keys out-of-band).
    pub async fn list_domains_surfaces_api_keys<DB>(db: &DB)
    where
        DB: LnurlRepository + Clone + Send + Sync + 'static,
    {
        db.add_domain("b.com").await.unwrap();

        let domains = db.list_domains().await.unwrap();
        let with_key = domains
            .iter()
            .find(|d| d.domain == "a.com")
            .expect("seeded domain with an api key");
        assert_eq!(with_key.api_key.as_deref(), Some("key-a"));
        let without_key = domains
            .iter()
            .find(|d| d.domain == "b.com")
            .expect("domain with no api key");
        assert_eq!(without_key.api_key, None);
    }

    /// `set_domain_jwt` updates the cached JWT of a domain with an api key
    /// (readable via `list_domains`); a domain with no attribution row is a
    /// no-op, not an error. The caller seeds `a.com` with an api key (allowlisted
    /// + `api_key` set) first, since a row can only be created by setting an api key.
    pub async fn set_domain_jwt_round_trips<DB>(db: &DB)
    where
        DB: LnurlRepository + Clone + Send + Sync + 'static,
    {
        let before = db.list_domains().await.unwrap();
        assert_eq!(
            before.iter().find(|d| d.domain == "a.com").unwrap().jwt,
            None
        );

        db.set_domain_jwt("a.com", "tok").await.unwrap();
        // A domain with no attribution row updates zero rows, not an error.
        db.set_domain_jwt("missing.com", "x").await.unwrap();

        let after = db.list_domains().await.unwrap();
        assert_eq!(
            after
                .iter()
                .find(|d| d.domain == "a.com")
                .unwrap()
                .jwt
                .as_deref(),
            Some("tok")
        );
    }
}
