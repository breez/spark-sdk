use std::sync::Arc;

use crate::{Seed, error::SdkError, models::Config};
use bitcoin::bip32::DerivationPath;
use bitcoin::hashes::{Hash, HashEngine, Hmac, HmacEngine, sha256};
use bitcoin::secp256k1::{self, Message, Secp256k1, rand::thread_rng};
use spark_wallet::{DefaultSigner, KeySet, KeySetType, SparkSignerAdapter};

use super::BreezSigner;

pub struct BreezSignerImpl {
    key_set: KeySet,
    secp: Secp256k1<secp256k1::All>,
}

impl BreezSignerImpl {
    pub fn new(
        config: &Config,
        seed: &Seed,
        key_set_type: KeySetType,
        use_address_index: bool,
        account_number: Option<u32>,
    ) -> Result<Self, SdkError> {
        let seed_bytes = seed.to_bytes()?;
        let key_set = KeySet::new(
            &seed_bytes,
            config.network.into(),
            key_set_type,
            use_address_index,
            account_number,
        )
        .map_err(|e| SdkError::Generic(e.to_string()))?;

        Ok(Self {
            key_set,
            secp: Secp256k1::new(),
        })
    }

    /// Builds the high-level Spark signer for this wallet's seed by wrapping the
    /// in-process low-level `DefaultSigner` in a `SparkSignerAdapter`.
    pub fn spark_signer(&self) -> Arc<dyn spark_wallet::SparkSigner> {
        Arc::new(SparkSignerAdapter::new(Arc::new(
            DefaultSigner::from_key_set(self.key_set.clone()),
        )))
    }
}

#[macros::async_trait]
impl BreezSigner for BreezSignerImpl {
    fn identity_public_key(&self) -> Result<secp256k1::PublicKey, SdkError> {
        Ok(self.key_set.identity_key_pair.public_key())
    }

    async fn derive_public_key(
        &self,
        path: &DerivationPath,
    ) -> Result<secp256k1::PublicKey, SdkError> {
        let derived = self
            .key_set
            .identity_master_key
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
            .key_set
            .identity_master_key
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
            .key_set
            .identity_master_key
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
            .key_set
            .identity_master_key
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
            .key_set
            .identity_master_key
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
            .key_set
            .identity_master_key
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
            .key_set
            .identity_master_key
            .derive_priv(&self.secp, key_path)
            .map_err(|e| SdkError::Generic(e.to_string()))?;

        let mut engine = HmacEngine::<sha256::Hash>::new(&derived.private_key.secret_bytes());
        engine.input(input);
        Ok(Hmac::<sha256::Hash>::from_engine(engine))
    }
}
