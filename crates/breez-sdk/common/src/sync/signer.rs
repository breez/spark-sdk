#[macros::async_trait]
pub trait SyncSigner: Send + Sync {
    async fn sign_ecdsa_recoverable(&self, data: &[u8]) -> anyhow::Result<Vec<u8>>;
}
