use crate::signer::SignerError;
use bitcoin::{
    key::Secp256k1,
    secp256k1::{All, PublicKey, SecretKey},
};
use rand::thread_rng;

pub trait KeyPairGenerator {
    fn generate_keypair(&self) -> Result<(SecretKey, PublicKey), SignerError>;
}

pub struct DefaultKeyPairGenerator {
    secp: Secp256k1<All>,
}

impl DefaultKeyPairGenerator {
    pub fn new() -> Self {
        DefaultKeyPairGenerator {
            secp: Secp256k1::new(),
        }
    }
}

impl KeyPairGenerator for DefaultKeyPairGenerator {
    fn generate_keypair(&self) -> Result<(SecretKey, PublicKey), SignerError> {
        let (secret_key, public_key) = self.secp.generate_keypair(&mut thread_rng());
        Ok((secret_key, public_key))
    }
}
