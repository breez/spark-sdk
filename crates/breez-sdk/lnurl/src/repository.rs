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

pub struct LnurlPayInvoice {
    pub payment_hash: String,
    pub user_pubkey: String,
    pub domain: String,
    pub username: String,
    pub metadata: String,
    pub invoice_expiry: i64,
    pub updated_at: i64,
    pub lightning_receive_id: Option<String>,
    pub bolt11_invoice: Option<String>,
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

    /// Insert or update an LNURL-pay invoice for verification
    async fn upsert_lnurl_pay_invoice(
        &self,
        invoice: &LnurlPayInvoice,
    ) -> Result<(), LnurlRepositoryError>;

    /// Get an LNURL-pay invoice by payment hash
    async fn get_lnurl_pay_invoice_by_payment_hash(
        &self,
        payment_hash: &str,
    ) -> Result<Option<LnurlPayInvoice>, LnurlRepositoryError>;
}
