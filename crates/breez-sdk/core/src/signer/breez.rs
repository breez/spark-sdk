use crate::error::SdkError;
use bitcoin::bip32::{DerivationPath, Xpriv};
use bitcoin::hashes::{Hash, HashEngine, Hmac, HmacEngine, sha256};
use bitcoin::secp256k1::{self, Message, Secp256k1, rand::thread_rng};

use super::BreezSigner;

/// SDK-layer signer for non-Spark operations (LNURL-auth, real-time sync,
/// message signing, ECIES). It holds a single master key and derives every key
/// from it by BIP32 path; the Spark layer decides which master to hand it.
pub struct BreezSignerImpl {
    master: Xpriv,
    secp: Secp256k1<secp256k1::All>,
}

impl BreezSignerImpl {
    pub fn new(master: Xpriv) -> Self {
        Self {
            master,
            secp: Secp256k1::new(),
        }
    }
}

#[macros::async_trait]
impl BreezSigner for BreezSignerImpl {
    async fn derive_public_key(
        &self,
        path: &DerivationPath,
    ) -> Result<secp256k1::PublicKey, SdkError> {
        let derived = self
            .master
            .derive_priv(&self.secp, path)
            .map_err(|e| SdkError::Generic(e.to_string()))?;
        Ok(derived.private_key.public_key(&self.secp))
    }

    async fn sign_ecdsa(
        &self,
        message: Message,
        path: &DerivationPath,
    ) -> Result<secp256k1::ecdsa::Signature, SdkError> {
        let derived = self
            .master
            .derive_priv(&self.secp, path)
            .map_err(|e| SdkError::Generic(e.to_string()))?;
        Ok(self.secp.sign_ecdsa_low_r(&message, &derived.private_key))
    }

    async fn sign_ecdsa_recoverable(
        &self,
        message: Message,
        path: &DerivationPath,
    ) -> Result<secp256k1::ecdsa::RecoverableSignature, SdkError> {
        let derived = self
            .master
            .derive_priv(&self.secp, path)
            .map_err(|e| SdkError::Generic(e.to_string()))?;
        Ok(self
            .secp
            .sign_ecdsa_recoverable(&message, &derived.private_key))
    }

    async fn encrypt_ecies(
        &self,
        message: &[u8],
        path: &DerivationPath,
    ) -> Result<Vec<u8>, SdkError> {
        let derived = self
            .master
            .derive_priv(&self.secp, path)
            .map_err(|e| SdkError::Generic(e.to_string()))?;
        let rc_pub = derived.private_key.public_key(&self.secp).serialize();
        utils::ecies::encrypt(&rc_pub, message)
            .map_err(|err| SdkError::Generic(format!("Could not encrypt data: {err}")))
    }

    async fn decrypt_ecies(
        &self,
        message: &[u8],
        path: &DerivationPath,
    ) -> Result<Vec<u8>, SdkError> {
        let derived = self
            .master
            .derive_priv(&self.secp, path)
            .map_err(|e| SdkError::Generic(e.to_string()))?;
        let rc_prv = derived.private_key.secret_bytes();
        utils::ecies::decrypt(&rc_prv, message)
            .map_err(|err| SdkError::Generic(format!("Could not decrypt data: {err}")))
    }

    async fn sign_hash_schnorr(
        &self,
        hash: &[u8],
        path: &DerivationPath,
    ) -> Result<secp256k1::schnorr::Signature, SdkError> {
        let derived = self
            .master
            .derive_priv(&self.secp, path)
            .map_err(|e| SdkError::Generic(e.to_string()))?;
        let message =
            Message::from_digest_slice(hash).map_err(|e| SdkError::Generic(e.to_string()))?;
        let keypair = derived.private_key.keypair(&self.secp);

        // Always use auxiliary randomness for enhanced security
        let mut rng = thread_rng();
        Ok(self
            .secp
            .sign_schnorr_with_rng(&message, &keypair, &mut rng))
    }

    async fn hmac_sha256(
        &self,
        key_path: &DerivationPath,
        input: &[u8],
    ) -> Result<Hmac<sha256::Hash>, SdkError> {
        let derived = self
            .master
            .derive_priv(&self.secp, key_path)
            .map_err(|e| SdkError::Generic(e.to_string()))?;

        let mut engine = HmacEngine::<sha256::Hash>::new(&derived.private_key.secret_bytes());
        engine.input(input);
        Ok(Hmac::<sha256::Hash>::from_engine(engine))
    }
}
