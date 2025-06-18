use bip32::{ChildNumber, XPrv};
use bitcoin::secp256k1::SecretKey;
use bitcoin::{
    NetworkKind,
    hashes::{Hash, sha256},
    key::Secp256k1,
    secp256k1::All,
    secp256k1::PublicKey,
};

use crate::{
    Network,
    signer::{Signer, SignerError},
};

pub struct DefaultSigner {
    master_key: XPrv,
    network: NetworkKind,
    secp: Secp256k1<All>,
}

pub enum DefaultSignerError {
    InvalidSeed,
}

impl DefaultSigner {
    pub fn new(
        seed: [u8; 32],
        network: impl Into<NetworkKind>,
    ) -> Result<Self, DefaultSignerError> {
        let master_key = XPrv::new(&seed).map_err(|_| DefaultSignerError::InvalidSeed)?;
        Ok(DefaultSigner {
            master_key,
            network: network.into(),
            secp: Secp256k1::new(),
        })
    }
}

impl DefaultSigner {
    fn derive_signing_key(&self, hash: sha256::Hash) -> Result<SecretKey, SignerError> {
        let u32_bytes = hash.as_byte_array()[..4]
            .try_into()
            .map_err(|_| SignerError::InvalidHash)?;
        let index = u32::from_be_bytes(u32_bytes) % 0x80000000;
        let child_number = ChildNumber::new(index, true).map_err(|_| SignerError::InvalidHash)?;
        let child = self.master_key.derive_child(child_number).map_err(|e| {
            SignerError::KeyDerivationError(format!("failed to derive child: {}", e))
        })?;
        Ok(
            SecretKey::from_slice(&child.private_key().to_bytes()).map_err(|e| {
                SignerError::KeyDerivationError(format!("failed to create private key: {}", e))
            })?,
        )
    }
}

#[async_trait::async_trait]
impl Signer for DefaultSigner {
    async fn generate_public_key(&self, hash: sha256::Hash) -> Result<PublicKey, SignerError> {
        let signing_key = self.derive_signing_key(hash)?;
        Ok(signing_key.public_key(&self.secp))
    }

    fn get_identity_public_key(
        &self,
        account_index: u32,
        network: Network,
    ) -> Result<PublicKey, SignerError> {
        todo!()
    }
}
