use lnurl_models::ListInvoicesInvoice;

use crate::user::User;

pub struct ZapRequest {
    pub user_pubkey: String,
    pub invoice: String,
    pub zap_request: String,
}

pub struct LnurlSenderComment {
    pub user_pubkey: String,
    pub invoice: String,
    pub comment: String,
}

#[derive(Debug, thiserror::Error)]
pub enum LnurlRepositoryError {
    #[error("name taken")]
    NameTaken,
    #[error("database error: {0}")]
    General(anyhow::Error),
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
    async fn insert_nostr_zap_request(
        &self,
        zap_request: &ZapRequest,
    ) -> Result<(), LnurlRepositoryError>;
    async fn insert_lnurl_sender_comment(
        &self,
        comment: &LnurlSenderComment,
    ) -> Result<(), LnurlRepositoryError>;
    async fn get_invoices_by_pubkey(
        &self,
        pubkey: &str,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<ListInvoicesInvoice>, LnurlRepositoryError>;
}
