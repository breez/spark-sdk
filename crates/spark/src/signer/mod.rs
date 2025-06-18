pub mod default_signer;
pub mod error;
use bitcoin::{hashes::sha256, secp256k1::PublicKey};
use error::SignerError;

use crate::Network;

#[async_trait::async_trait]
pub trait Signer {
    async fn generate_public_key(&self, hash: sha256::Hash) -> Result<PublicKey, SignerError>;
    fn get_identity_public_key(
        &self,
        account_index: u32,
        network: Network,
    ) -> Result<PublicKey, SignerError>;
}
