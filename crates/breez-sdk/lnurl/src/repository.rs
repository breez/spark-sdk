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
    pub invoice_expiry: i64,
    pub payment_hash: String,
    pub user_pubkey: String,
    pub updated_at: i64,
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
    /// Get list of user pubkeys that have unexpired invoices
    async fn get_users_with_unexpired_invoices(&self) -> Result<Vec<String>, LnurlRepositoryError>;
    /// Check if a specific user has any unexpired invoices
    async fn user_has_unexpired_invoices(
        &self,
        user_pubkey: &str,
    ) -> Result<bool, LnurlRepositoryError>;
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
}
