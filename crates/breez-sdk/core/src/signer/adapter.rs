use crate::SdkError;
use crate::signer::external_types::{MessageBytes, derivation_path_to_string};
use crate::signer::{BreezSigner, ExternalBreezSigner};
use bitcoin::bip32::DerivationPath;
use bitcoin::secp256k1;
use std::sync::Arc;

/// Adapter that wraps an `ExternalBreezSigner` and implements `BreezSigner`.
///
/// This adapter translates between the internal `BreezSigner` trait (using Rust types)
/// and the external `ExternalBreezSigner` trait (using FFI-compatible types).
pub struct ExternalBreezSignerAdapter {
    external: Arc<dyn ExternalBreezSigner>,
}

impl ExternalBreezSignerAdapter {
    pub fn new(external: Arc<dyn ExternalBreezSigner>) -> Self {
        Self { external }
    }
}

#[macros::async_trait]
impl BreezSigner for ExternalBreezSignerAdapter {
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
        message: secp256k1::Message,
        path: &DerivationPath,
    ) -> Result<secp256k1::ecdsa::Signature, SdkError> {
        let path_str = derivation_path_to_string(path);
        // Convert Message digest to MessageBytes
        let msg_bytes = MessageBytes::new(message.as_ref().to_vec());
        let sig_bytes = self
            .external
            .sign_ecdsa(msg_bytes, path_str)
            .await
            .map_err(|e| SdkError::Signer(format!("External signer sign_ecdsa failed: {e}")))?;
        sig_bytes.to_signature()
    }

    async fn sign_ecdsa_recoverable(
        &self,
        message: secp256k1::Message,
        path: &DerivationPath,
    ) -> Result<secp256k1::ecdsa::RecoverableSignature, SdkError> {
        let path_str = derivation_path_to_string(path);
        // Convert Message digest to MessageBytes
        let msg_bytes = MessageBytes::new(message.as_ref().to_vec());
        let sig_bytes = self
            .external
            .sign_ecdsa_recoverable(msg_bytes, path_str)
            .await
            .map_err(|e| {
                SdkError::Signer(format!(
                    "External signer sign_ecdsa_recoverable failed: {e}"
                ))
            })?;
        // Convert the 65-byte signature back to RecoverableSignature
        if sig_bytes.bytes.len() != 65 {
            return Err(SdkError::Signer(
                "Invalid recoverable signature length".to_string(),
            ));
        }
        let recovery_id = secp256k1::ecdsa::RecoveryId::from_i32(
            i32::from(sig_bytes.bytes[0]).saturating_sub(31),
        )
        .map_err(|e| SdkError::Signer(format!("Invalid recovery ID: {e}")))?;
        secp256k1::ecdsa::RecoverableSignature::from_compact(&sig_bytes.bytes[1..], recovery_id)
            .map_err(|e| SdkError::Signer(format!("Invalid recoverable signature: {e}")))
    }

    async fn encrypt_ecies(
        &self,
        message: &[u8],
        path: &DerivationPath,
    ) -> Result<Vec<u8>, SdkError> {
        let path_str = derivation_path_to_string(path);
        self.external
            .encrypt_ecies(message.to_vec(), path_str)
            .await
            .map_err(|e| SdkError::Signer(format!("External signer encrypt_ecies failed: {e}")))
    }

    async fn decrypt_ecies(
        &self,
        message: &[u8],
        path: &DerivationPath,
    ) -> Result<Vec<u8>, SdkError> {
        let path_str = derivation_path_to_string(path);
        self.external
            .decrypt_ecies(message.to_vec(), path_str)
            .await
            .map_err(|e| SdkError::Signer(format!("External signer decrypt_ecies failed: {e}")))
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
}
