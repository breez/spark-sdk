#[cfg_attr(test, mockall::automock)]
#[macros::async_trait]
pub trait SyncSigner: Send + Sync {
    async fn sign_ecdsa_recoverable(&self, data: &[u8]) -> anyhow::Result<Vec<u8>>;
    async fn ecies_encrypt(&self, msg: Vec<u8>) -> anyhow::Result<Vec<u8>>;
    async fn ecies_decrypt(&self, msg: Vec<u8>) -> anyhow::Result<Vec<u8>>;
}
