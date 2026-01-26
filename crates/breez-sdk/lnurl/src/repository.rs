use lnurl_models::ListMetadataMetadata;

use crate::user::User;
use crate::zap::Zap;

#[derive(Debug, thiserror::Error)]
pub enum LnurlRepositoryError {
    #[error("name taken")]
    NameTaken,
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
}

#[derive(Debug, Clone)]
pub struct NewlyPaid {
    pub payment_hash: String,
    pub created_at: i64,
    pub retry_count: i32,
    pub next_retry_at: i64,
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

    async fn upsert_zap(&self, zap: &Zap) -> Result<(), LnurlRepositoryError>;
    async fn get_zap_by_payment_hash(
        &self,
        payment_hash: &str,
    ) -> Result<Option<Zap>, LnurlRepositoryError>;
    /// Get list of user pubkeys that have unexpired invoices that should be signed by the server
    async fn get_zap_monitored_users(&self) -> Result<Vec<String>, LnurlRepositoryError>;
    /// Check if a specific user has any unexpired invoices that should be signed by the server
    async fn is_zap_monitored_user(&self, user_pubkey: &str) -> Result<bool, LnurlRepositoryError>;
    async fn insert_lnurl_sender_comment(
        &self,
        comment: &LnurlSenderComment,
    ) -> Result<(), LnurlRepositoryError>;
    async fn get_metadata_by_pubkey(
        &self,
        pubkey: &str,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<ListMetadataMetadata>, LnurlRepositoryError>;

    /// Get all allowed domains from the database
    async fn list_domains(&self) -> Result<Vec<String>, LnurlRepositoryError>;

    /// Insert a domain if it doesn't already exist
    async fn add_domain(&self, domain: &str) -> Result<(), LnurlRepositoryError>;

    /// Insert or update an invoice
    async fn upsert_invoice(&self, invoice: &Invoice) -> Result<(), LnurlRepositoryError>;

    /// Get an invoice by payment hash
    async fn get_invoice_by_payment_hash(
        &self,
        payment_hash: &str,
    ) -> Result<Option<Invoice>, LnurlRepositoryError>;

    /// Get list of user pubkeys that have unexpired invoices without preimages
    async fn get_invoice_monitored_users(&self) -> Result<Vec<String>, LnurlRepositoryError>;

    /// Check if a specific user has any unexpired invoices without preimages
    async fn is_invoice_monitored_user(
        &self,
        user_pubkey: &str,
    ) -> Result<bool, LnurlRepositoryError>;

    /// Insert a newly paid invoice into the queue
    async fn insert_newly_paid(&self, newly_paid: &NewlyPaid) -> Result<(), LnurlRepositoryError>;

    /// Get all newly paid invoices ready for processing (`next_retry_at` <= now)
    async fn get_pending_newly_paid(&self) -> Result<Vec<NewlyPaid>, LnurlRepositoryError>;

    /// Update retry count and next retry time for a newly paid invoice
    async fn update_newly_paid_retry(
        &self,
        payment_hash: &str,
        retry_count: i32,
        next_retry_at: i64,
    ) -> Result<(), LnurlRepositoryError>;

    /// Delete a newly paid invoice from the queue
    async fn delete_newly_paid(&self, payment_hash: &str) -> Result<(), LnurlRepositoryError>;
}
