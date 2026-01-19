use crate::{Seed, error::SdkError, models::Config};
use bitcoin::bip32::DerivationPath;
use bitcoin::hashes::{Hash, HashEngine, Hmac, HmacEngine, sha256};
use bitcoin::secp256k1::{self, Message, Secp256k1, rand::thread_rng};
use spark_wallet::{DefaultSigner, KeySet, KeySetType, Signer};

use super::BreezSigner;

pub struct BreezSignerImpl {
    key_set: KeySet,
    secp: Secp256k1<secp256k1::All>,
    spark_signer: DefaultSigner,
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
            key_set: key_set.clone(),
            secp: Secp256k1::new(),
            spark_signer: DefaultSigner::from_key_set(key_set),
        })
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
        ecies::encrypt(&rc_pub, message)
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
        ecies::decrypt(&rc_prv, message)
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

    async fn generate_random_signing_commitment(
        &self,
    ) -> Result<spark_wallet::FrostSigningCommitmentsWithNonces, SdkError> {
        self.spark_signer
            .generate_random_signing_commitment()
            .await
            .map_err(|e| SdkError::Generic(e.to_string()))
    }

    async fn get_public_key_for_node(
        &self,
        id: &spark_wallet::TreeNodeId,
    ) -> Result<secp256k1::PublicKey, SdkError> {
        self.spark_signer
            .get_public_key_for_node(id)
            .await
            .map_err(|e| SdkError::Generic(e.to_string()))
    }

    async fn generate_random_key(&self) -> Result<spark_wallet::SecretKeySource, SdkError> {
        self.spark_signer
            .generate_random_key()
            .await
            .map_err(|e| SdkError::Generic(e.to_string()))
    }

    async fn static_deposit_secret_key_encrypted(
        &self,
        index: u32,
    ) -> Result<spark_wallet::SecretKeySource, SdkError> {
        self.spark_signer
            .static_deposit_secret_key_encrypted(index)
            .await
            .map_err(|e| SdkError::Generic(e.to_string()))
    }

    async fn static_deposit_secret_key(
        &self,
        index: u32,
    ) -> Result<secp256k1::SecretKey, SdkError> {
        self.spark_signer
            .static_deposit_secret_key(index)
            .await
            .map_err(|e| SdkError::Generic(e.to_string()))
    }

    async fn static_deposit_signing_key(
        &self,
        index: u32,
    ) -> Result<secp256k1::PublicKey, SdkError> {
        self.spark_signer
            .static_deposit_signing_key(index)
            .await
            .map_err(|e| SdkError::Generic(e.to_string()))
    }

    async fn subtract_secret_keys(
        &self,
        signing_key: &spark_wallet::SecretKeySource,
        new_signing_key: &spark_wallet::SecretKeySource,
    ) -> Result<spark_wallet::SecretKeySource, SdkError> {
        self.spark_signer
            .subtract_secret_keys(signing_key, new_signing_key)
            .await
            .map_err(|e| SdkError::Generic(e.to_string()))
    }

    async fn split_secret_with_proofs(
        &self,
        secret: &spark_wallet::SecretToSplit,
        threshold: u32,
        num_shares: usize,
    ) -> Result<Vec<spark_wallet::VerifiableSecretShare>, SdkError> {
        self.spark_signer
            .split_secret_with_proofs(secret, threshold, num_shares)
            .await
            .map_err(|e| SdkError::Generic(e.to_string()))
    }

    async fn encrypt_secret_key_for_receiver(
        &self,
        private_key: &spark_wallet::EncryptedPrivateKey,
        receiver_public_key: &secp256k1::PublicKey,
    ) -> Result<Vec<u8>, SdkError> {
        self.spark_signer
            .encrypt_secret_key_for_receiver(private_key, receiver_public_key)
            .await
            .map_err(|e| SdkError::Generic(e.to_string()))
    }

    async fn public_key_from_secret_key_source(
        &self,
        private_key: &spark_wallet::SecretKeySource,
    ) -> Result<secp256k1::PublicKey, SdkError> {
        self.spark_signer
            .public_key_from_secret_key_source(private_key)
            .await
            .map_err(|e| SdkError::Generic(e.to_string()))
    }

    async fn sign_frost<'a>(
        &self,
        request: spark_wallet::SignFrostRequest<'a>,
    ) -> Result<frost_secp256k1_tr::round2::SignatureShare, SdkError> {
        self.spark_signer
            .sign_frost(request)
            .await
            .map_err(|e| SdkError::Generic(e.to_string()))
    }

    async fn aggregate_frost<'a>(
        &self,
        request: spark_wallet::AggregateFrostRequest<'a>,
    ) -> Result<frost_secp256k1_tr::Signature, SdkError> {
        self.spark_signer
            .aggregate_frost(request)
            .await
            .map_err(|e| SdkError::Generic(e.to_string()))
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
