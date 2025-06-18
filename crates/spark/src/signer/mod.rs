mod default_signer;
mod error;

pub use default_signer::DefaultSigner;
pub use error::SignerError;

use bitcoin::{hashes::sha256, secp256k1::PublicKey};

use crate::core::Network;

#[async_trait::async_trait]
pub trait Signer {
    async fn generate_public_key(&self, hash: sha256::Hash) -> Result<PublicKey, SignerError>;
    fn get_identity_public_key(
        &self,
        account_index: u32,
        network: Network,
    ) -> Result<PublicKey, SignerError>;
}
