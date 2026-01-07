use spark_wallet::{
    AggregateFrostRequest, EncryptedPrivateKey, FrostSigningCommitmentsWithNonces,
    PrivateKeySource, SecretToSplit, SignFrostRequest, Signer, SignerError, TreeNodeId,
    VerifiableSecretShare,
};
use std::sync::Arc;

use crate::signer::BreezSigner;
use async_trait::async_trait;
use bitcoin::{
    bip32::DerivationPath,
    secp256k1::{self, PublicKey, SecretKey, schnorr},
};
use frost_secp256k1_tr::round2::SignatureShare;

pub struct SparkSigner {
    signer: Arc<dyn BreezSigner>,
}

impl SparkSigner {
    pub fn new(signer: Arc<dyn BreezSigner>) -> Self {
        Self { signer }
    }
}

#[async_trait]
impl Signer for SparkSigner {
    async fn sign_message_ecdsa_with_identity_key(
        &self,
        message: &[u8],
    ) -> Result<secp256k1::ecdsa::Signature, SignerError> {
        let identity_path = DerivationPath::master();
        self.signer
            .sign_ecdsa(message, &identity_path)
            .await
            .map_err(|e| SignerError::Generic(e.to_string()))
    }

    async fn sign_hash_schnorr_with_identity_key(
        &self,
        hash: &[u8],
    ) -> Result<schnorr::Signature, SignerError> {
        let identity_path = DerivationPath::master();
        self.signer
            .sign_hash_schnorr(hash, &identity_path)
            .await
            .map_err(|e| SignerError::Generic(e.to_string()))
    }

    async fn generate_frost_signing_commitments(
        &self,
    ) -> Result<FrostSigningCommitmentsWithNonces, SignerError> {
        self.signer
            .generate_frost_signing_commitments()
            .await
            .map_err(|e| SignerError::Generic(e.to_string()))
    }

    async fn get_public_key_for_node(&self, id: &TreeNodeId) -> Result<PublicKey, SignerError> {
        self.signer
            .get_public_key_for_node(id)
            .await
            .map_err(|e| SignerError::Generic(e.to_string()))
    }

    async fn generate_random_key(&self) -> Result<PrivateKeySource, SignerError> {
        self.signer
            .generate_random_key()
            .await
            .map_err(|e| SignerError::Generic(e.to_string()))
    }

    async fn get_identity_public_key(&self) -> Result<PublicKey, SignerError> {
        Ok(self.signer.identity_public_key())
    }

    async fn get_static_deposit_private_key_source(
        &self,
        index: u32,
    ) -> Result<PrivateKeySource, SignerError> {
        self.signer
            .get_static_deposit_private_key_source(index)
            .await
            .map_err(|e| SignerError::Generic(e.to_string()))
    }

    async fn get_static_deposit_private_key(&self, index: u32) -> Result<SecretKey, SignerError> {
        self.signer
            .get_static_deposit_private_key(index)
            .await
            .map_err(|e| SignerError::Generic(e.to_string()))
    }

    async fn get_static_deposit_public_key(&self, index: u32) -> Result<PublicKey, SignerError> {
        self.signer
            .get_static_deposit_public_key(index)
            .await
            .map_err(|e| SignerError::Generic(e.to_string()))
    }

    async fn subtract_private_keys(
        &self,
        signing_key: &PrivateKeySource,
        new_signing_key: &PrivateKeySource,
    ) -> Result<PrivateKeySource, SignerError> {
        self.signer
            .subtract_private_keys(signing_key, new_signing_key)
            .await
            .map_err(|e| SignerError::Generic(e.to_string()))
    }

    async fn split_secret_with_proofs(
        &self,
        secret: &SecretToSplit,
        threshold: u32,
        num_shares: usize,
    ) -> Result<Vec<VerifiableSecretShare>, SignerError> {
        self.signer
            .split_secret_with_proofs(secret, threshold, num_shares)
            .await
            .map_err(|e| SignerError::Generic(e.to_string()))
    }

    async fn encrypt_private_key_for_receiver(
        &self,
        private_key: &EncryptedPrivateKey,
        receiver_public_key: &PublicKey,
    ) -> Result<Vec<u8>, SignerError> {
        self.signer
            .encrypt_private_key_for_receiver(private_key, receiver_public_key)
            .await
            .map_err(|e| SignerError::Generic(e.to_string()))
    }

    async fn get_public_key_from_private_key_source(
        &self,
        private_key: &PrivateKeySource,
    ) -> Result<PublicKey, SignerError> {
        self.signer
            .get_public_key_from_private_key_source(private_key)
            .await
            .map_err(|e| SignerError::Generic(e.to_string()))
    }

    async fn sign_frost<'a>(
        &self,
        request: SignFrostRequest<'a>,
    ) -> Result<SignatureShare, SignerError> {
        self.signer
            .sign_frost(request)
            .await
            .map_err(|e| SignerError::Generic(e.to_string()))
    }

    async fn aggregate_frost<'a>(
        &self,
        request: AggregateFrostRequest<'a>,
    ) -> Result<frost_secp256k1_tr::Signature, SignerError> {
        self.signer
            .aggregate_frost(request)
            .await
            .map_err(|e| SignerError::Generic(e.to_string()))
    }
}
