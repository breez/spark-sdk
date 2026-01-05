use crate::SdkError;
use crate::signer::external_types::{
    ExternalAggregateFrostRequest, ExternalEncryptedPrivateKey, ExternalPrivateKeySource,
    ExternalSecretToSplit, ExternalSignFrostRequest, ExternalTreeNodeId, PublicKeyBytes,
    derivation_path_to_string,
};
use crate::signer::{BreezSigner, ExternalSigner};
use bitcoin::bip32::DerivationPath;
use bitcoin::secp256k1;
use std::sync::Arc;

/// Adapter that wraps an `ExternalSigner` and implements `BreezSigner`.
///
/// This adapter translates between the internal `BreezSigner` trait (using Rust types)
/// and the external `ExternalSigner` trait (using FFI-compatible types).
pub struct ExternalSignerAdapter {
    external: Arc<dyn ExternalSigner>,
}

impl ExternalSignerAdapter {
    pub fn new(external: Arc<dyn ExternalSigner>) -> Self {
        Self { external }
    }
}

#[macros::async_trait]
impl BreezSigner for ExternalSignerAdapter {
    fn identity_public_key(&self) -> Result<secp256k1::PublicKey, SdkError> {
        let pk_bytes = self.external.identity_public_key().map_err(|e| {
            SdkError::Signer(format!("External signer identity_public_key failed: {e}"))
        })?;
        pk_bytes
            .to_public_key()
            .map_err(|e| SdkError::Signer(e.to_string()))
    }

    async fn derive_public_key(
        &self,
        path: &DerivationPath,
    ) -> Result<secp256k1::PublicKey, SdkError> {
        let path_str = derivation_path_to_string(path);
        let pk_bytes = self
            .external
            .derive_public_key(path_str)
            .await
            .map_err(|e| {
                SdkError::Signer(format!("External signer derive_public_key failed: {e}"))
            })?;
        pk_bytes.to_public_key()
    }

    async fn sign_ecdsa(
        &self,
        message: &[u8],
        path: &DerivationPath,
    ) -> Result<secp256k1::ecdsa::Signature, SdkError> {
        let path_str = derivation_path_to_string(path);
        let sig_bytes = self
            .external
            .sign_ecdsa(message.to_vec(), path_str)
            .await
            .map_err(|e| SdkError::Signer(format!("External signer sign_ecdsa failed: {e}")))?;
        sig_bytes.to_signature()
    }

    async fn sign_ecdsa_recoverable(
        &self,
        message: &[u8],
        path: &DerivationPath,
    ) -> Result<Vec<u8>, SdkError> {
        let path_str = derivation_path_to_string(path);
        let sig_bytes = self
            .external
            .sign_ecdsa_recoverable(message.to_vec(), path_str)
            .await
            .map_err(|e| {
                SdkError::Signer(format!(
                    "External signer sign_ecdsa_recoverable failed: {e}"
                ))
            })?;
        Ok(sig_bytes.bytes)
    }

    async fn ecies_encrypt(
        &self,
        message: &[u8],
        path: &DerivationPath,
    ) -> Result<Vec<u8>, SdkError> {
        let path_str = derivation_path_to_string(path);
        self.external
            .ecies_encrypt(message.to_vec(), path_str)
            .await
            .map_err(|e| SdkError::Signer(format!("External signer ecies_encrypt failed: {e}")))
    }

    async fn ecies_decrypt(
        &self,
        message: &[u8],
        path: &DerivationPath,
    ) -> Result<Vec<u8>, SdkError> {
        let path_str = derivation_path_to_string(path);
        self.external
            .ecies_decrypt(message.to_vec(), path_str)
            .await
            .map_err(|e| SdkError::Signer(format!("External signer ecies_decrypt failed: {e}")))
    }

    async fn sign_hash_schnorr(
        &self,
        hash: &[u8],
        path: &DerivationPath,
    ) -> Result<secp256k1::schnorr::Signature, SdkError> {
        let path_str = derivation_path_to_string(path);
        let sig_bytes = self
            .external
            .sign_hash_schnorr(hash.to_vec(), path_str)
            .await
            .map_err(|e| {
                SdkError::Signer(format!("External signer sign_hash_schnorr failed: {e}"))
            })?;
        sig_bytes.to_signature()
    }

    async fn hmac_sha256(
        &self,
        key_path: &DerivationPath,
        input: &[u8],
    ) -> Result<bitcoin::hashes::Hmac<bitcoin::hashes::sha256::Hash>, SdkError> {
        let path_str = derivation_path_to_string(key_path);
        let hash_bytes = self
            .external
            .hmac_sha256(input.to_vec(), path_str)
            .await
            .map_err(|e| SdkError::Signer(format!("External signer hmac_sha256 failed: {e}")))?;
        hash_bytes.to_hmac()
    }

    async fn generate_frost_signing_commitments(
        &self,
    ) -> Result<spark_wallet::FrostSigningCommitmentsWithNonces, SdkError> {
        let commitments_ext = self
            .external
            .generate_frost_signing_commitments()
            .await
            .map_err(|e| {
                SdkError::Signer(format!(
                    "External signer generate_frost_signing_commitments failed: {e}"
                ))
            })?;
        commitments_ext.to_frost_commitments()
    }

    async fn get_public_key_for_node(
        &self,
        id: &spark_wallet::TreeNodeId,
    ) -> Result<secp256k1::PublicKey, SdkError> {
        let id_ext = ExternalTreeNodeId::from_tree_node_id(id)?;
        let pk_bytes = self
            .external
            .get_public_key_for_node(id_ext)
            .await
            .map_err(|e| {
                SdkError::Signer(format!(
                    "External signer get_public_key_for_node failed: {e}"
                ))
            })?;
        pk_bytes.to_public_key()
    }

    async fn generate_random_key(&self) -> Result<spark_wallet::PrivateKeySource, SdkError> {
        let key_ext = self.external.generate_random_key().await.map_err(|e| {
            SdkError::Signer(format!("External signer generate_random_key failed: {e}"))
        })?;
        key_ext.to_private_key_source()
    }

    async fn get_static_deposit_private_key_source(
        &self,
        index: u32,
    ) -> Result<spark_wallet::PrivateKeySource, SdkError> {
        let key_ext = self
            .external
            .get_static_deposit_private_key_source(index)
            .await
            .map_err(|e| {
                SdkError::Signer(format!(
                    "External signer get_static_deposit_private_key_source failed: {e}"
                ))
            })?;
        key_ext.to_private_key_source()
    }

    async fn get_static_deposit_private_key(
        &self,
        index: u32,
    ) -> Result<secp256k1::SecretKey, SdkError> {
        let key_bytes = self
            .external
            .get_static_deposit_private_key(index)
            .await
            .map_err(|e| {
                SdkError::Signer(format!(
                    "External signer get_static_deposit_private_key failed: {e}"
                ))
            })?;

        key_bytes
            .to_secret_key()
            .map_err(|e| SdkError::Signer(format!("Invalid private key bytes: {e}")))
    }

    async fn get_static_deposit_public_key(
        &self,
        index: u32,
    ) -> Result<secp256k1::PublicKey, SdkError> {
        let pk_bytes = self
            .external
            .get_static_deposit_public_key(index)
            .await
            .map_err(|e| {
                SdkError::Signer(format!(
                    "External signer get_static_deposit_public_key failed: {e}"
                ))
            })?;
        pk_bytes.to_public_key()
    }

    async fn subtract_private_keys(
        &self,
        signing_key: &spark_wallet::PrivateKeySource,
        new_signing_key: &spark_wallet::PrivateKeySource,
    ) -> Result<spark_wallet::PrivateKeySource, SdkError> {
        let signing_key_ext = ExternalPrivateKeySource::from_private_key_source(signing_key)?;
        let new_signing_key_ext =
            ExternalPrivateKeySource::from_private_key_source(new_signing_key)?;

        let result_ext = self
            .external
            .subtract_private_keys(signing_key_ext, new_signing_key_ext)
            .await
            .map_err(|e| {
                SdkError::Signer(format!("External signer subtract_private_keys failed: {e}"))
            })?;
        result_ext.to_private_key_source()
    }

    async fn split_secret_with_proofs(
        &self,
        secret: &spark_wallet::SecretToSplit,
        threshold: u32,
        num_shares: usize,
    ) -> Result<Vec<spark_wallet::VerifiableSecretShare>, SdkError> {
        let secret_ext = ExternalSecretToSplit::from_secret_to_split(secret)?;
        let num_shares_u32 = num_shares
            .try_into()
            .map_err(|_| SdkError::Generic("num_shares value too large".to_string()))?;
        let shares_ext = self
            .external
            .split_secret(secret_ext, threshold, num_shares_u32)
            .await
            .map_err(|e| SdkError::Signer(format!("External signer split_secret failed: {e}")))?;

        shares_ext
            .into_iter()
            .map(|share_ext| share_ext.to_verifiable_secret_share())
            .collect()
    }

    async fn encrypt_private_key_for_receiver(
        &self,
        private_key: &spark_wallet::EncryptedPrivateKey,
        receiver_public_key: &secp256k1::PublicKey,
    ) -> Result<Vec<u8>, SdkError> {
        let private_key_ext = ExternalEncryptedPrivateKey::from_encrypted_private_key(private_key)?;
        let receiver_pk_bytes = PublicKeyBytes::from_public_key(receiver_public_key);

        self.external
            .encrypt_private_key_for_receiver(private_key_ext, receiver_pk_bytes)
            .await
            .map_err(|e| {
                SdkError::Signer(format!(
                    "External signer encrypt_private_key_for_receiver failed: {e}"
                ))
            })
    }

    async fn get_public_key_from_private_key_source(
        &self,
        private_key: &spark_wallet::PrivateKeySource,
    ) -> Result<secp256k1::PublicKey, SdkError> {
        let private_key_ext = ExternalPrivateKeySource::from_private_key_source(private_key)?;
        let pk_bytes = self
            .external
            .get_public_key_from_private_key_source(private_key_ext)
            .await
            .map_err(|e| {
                SdkError::Signer(format!(
                    "External signer get_public_key_from_private_key_source failed: {e}"
                ))
            })?;
        pk_bytes.to_public_key()
    }

    async fn sign_frost<'a>(
        &self,
        request: spark_wallet::SignFrostRequest<'a>,
    ) -> Result<frost_secp256k1_tr::round2::SignatureShare, SdkError> {
        let request_ext = ExternalSignFrostRequest::from_sign_frost_request(&request)?;

        let share_ext = self
            .external
            .sign_frost(request_ext)
            .await
            .map_err(|e| SdkError::Signer(format!("External signer sign_frost failed: {e}")))?;
        share_ext.to_signature_share()
    }

    async fn aggregate_frost<'a>(
        &self,
        request: spark_wallet::AggregateFrostRequest<'a>,
    ) -> Result<frost_secp256k1_tr::Signature, SdkError> {
        let request_ext = ExternalAggregateFrostRequest::from_aggregate_frost_request(&request)?;

        let sig_ext = self
            .external
            .aggregate_frost_signatures(request_ext)
            .await
            .map_err(|e| {
                SdkError::Signer(format!("External signer aggregate_frost failed: {e}"))
            })?;
        sig_ext.to_frost_signature()
    }
}
