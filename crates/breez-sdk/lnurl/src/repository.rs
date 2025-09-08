use crate::user::User;

#[derive(Debug, thiserror::Error)]
pub enum LnurlRepositoryError {
    #[error("name taken")]
    NameTaken,
    #[error("database error: {0}")]
    General(anyhow::Error),
}

#[async_trait::async_trait]
pub trait LnurlRepository {
    async fn delete_user(&self, pubkey: &str) -> Result<(), LnurlRepositoryError>;
    async fn get_user_by_name(&self, name: &str) -> Result<Option<User>, LnurlRepositoryError>;
    async fn get_user_by_pubkey(&self, pubkey: &str) -> Result<Option<User>, LnurlRepositoryError>;
    async fn upsert_user(&self, user: &User) -> Result<(), LnurlRepositoryError>;
}
