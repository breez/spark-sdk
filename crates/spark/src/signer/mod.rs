use bitcoin::{PublicKey, hashes::sha256};

pub enum SignerError {
    InvalidHash,
    KeyDerivationError(String),
}

#[async_trait::async_trait]
pub trait Signer {
    async fn generate_public_key(&self, hash: sha256::Hash) -> Result<PublicKey, SignerError>;
}
