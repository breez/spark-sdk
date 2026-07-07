use crate::SdkError;
use crate::signer::external_types::{MessageBytes, derivation_path_to_string};
use crate::signer::{
    BreezSigner, EciesSigner, ExternalBreezSigner, ExternalSigningSigner, HmacSigner,
};
use bitcoin::bip32::DerivationPath;
use bitcoin::secp256k1;
use std::sync::Arc;

/// Recovers a `RecoverableSignature` from the external trait's 65-byte layout
/// (`[31 + recovery_id]` followed by the 64-byte compact signature).
fn recoverable_from_bytes(
    bytes: &[u8],
) -> Result<secp256k1::ecdsa::RecoverableSignature, SdkError> {
    if bytes.len() != 65 {
        return Err(SdkError::Signer(
            "Invalid recoverable signature length".to_string(),
        ));
    }
    let recovery_id =
        secp256k1::ecdsa::RecoveryId::from_i32(i32::from(bytes[0]).saturating_sub(31))
            .map_err(|e| SdkError::Signer(format!("Invalid recovery ID: {e}")))?;
    secp256k1::ecdsa::RecoverableSignature::from_compact(&bytes[1..], recovery_id)
        .map_err(|e| SdkError::Signer(format!("Invalid recoverable signature: {e}")))
}

/// Adapter that wraps an `ExternalBreezSigner` and implements the internal
/// signing, ECIES, and HMAC traits.
///
/// This adapter translates between the internal traits (using Rust types) and
/// the external `ExternalBreezSigner` trait (using FFI-compatible types).
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
        recoverable_from_bytes(&sig_bytes.bytes)
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
}

#[macros::async_trait]
impl EciesSigner for ExternalBreezSignerAdapter {
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
}

#[macros::async_trait]
impl HmacSigner for ExternalBreezSignerAdapter {
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

/// Adapter that wraps an `ExternalSigningSigner` (signing only, no ECIES/HMAC)
/// and implements the internal `BreezSigner` trait. Used for signers that can't
/// release key material for the SDK's local ECIES/HMAC operations.
pub struct ExternalSigningSignerAdapter {
    external: Arc<dyn ExternalSigningSigner>,
}

impl ExternalSigningSignerAdapter {
    pub fn new(external: Arc<dyn ExternalSigningSigner>) -> Self {
        Self { external }
    }
}

#[macros::async_trait]
impl BreezSigner for ExternalSigningSignerAdapter {
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
        recoverable_from_bytes(&sig_bytes.bytes)
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
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;
    use std::sync::Arc;

    use bitcoin::bip32::DerivationPath;
    use bitcoin::hashes::{Hash, sha256};
    use bitcoin::secp256k1::{Message, Secp256k1, XOnlyPublicKey};

    use super::ExternalSigningSignerAdapter;
    use crate::Network;
    use crate::error::SignerError;
    use crate::signer::breez::BreezSignerImpl;
    use crate::signer::external_types::{
        EcdsaSignatureBytes, MessageBytes, PublicKeyBytes, RecoverableEcdsaSignatureBytes,
        SchnorrSignatureBytes,
    };
    use crate::signer::{
        BreezSigner, DefaultExternalSigner, ExternalBreezSigner, ExternalSigningSigner,
    };

    const MNEMONIC: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    /// A signing-only external signer delegating to a seed-derived
    /// `DefaultExternalSigner` (whose first four methods are the signing ones).
    struct SigningOnlyView(DefaultExternalSigner);

    #[macros::async_trait]
    impl ExternalSigningSigner for SigningOnlyView {
        async fn derive_public_key(&self, path: String) -> Result<PublicKeyBytes, SignerError> {
            self.0.derive_public_key(path).await
        }
        async fn sign_ecdsa(
            &self,
            message: MessageBytes,
            path: String,
        ) -> Result<EcdsaSignatureBytes, SignerError> {
            self.0.sign_ecdsa(message, path).await
        }
        async fn sign_ecdsa_recoverable(
            &self,
            message: MessageBytes,
            path: String,
        ) -> Result<RecoverableEcdsaSignatureBytes, SignerError> {
            self.0.sign_ecdsa_recoverable(message, path).await
        }
        async fn sign_hash_schnorr(
            &self,
            hash: Vec<u8>,
            path: String,
        ) -> Result<SchnorrSignatureBytes, SignerError> {
            self.0.sign_hash_schnorr(hash, path).await
        }
    }

    fn reference_signer() -> BreezSignerImpl {
        let seed = crate::Seed::Mnemonic {
            mnemonic: MNEMONIC.to_string(),
            passphrase: None,
        };
        let seed_bytes = seed.to_bytes().unwrap();
        let master =
            spark_wallet::identity_master_key(&seed_bytes, Network::Regtest.into(), Some(0))
                .unwrap();
        BreezSignerImpl::new(master)
    }

    /// The signing-only adapter's `BreezSigner` methods delegate through the FFI
    /// conversions and produce the same keys/signatures as the reference signer.
    #[macros::async_test_all]
    async fn signing_only_adapter_matches_reference_signer() {
        let external =
            DefaultExternalSigner::new(MNEMONIC.to_string(), None, Network::Regtest, Some(0))
                .unwrap();
        let adapter = ExternalSigningSignerAdapter::new(Arc::new(SigningOnlyView(external)));
        let reference = reference_signer();

        let path = DerivationPath::from_str("m/0'/0'/0'").unwrap();

        // derive_public_key round-trips through the string-path + bytes conversion.
        assert_eq!(
            adapter.derive_public_key(&path).await.unwrap(),
            reference.derive_public_key(&path).await.unwrap(),
        );

        // sign_ecdsa is deterministic (low-r), so the signatures match exactly.
        let digest = sha256::Hash::hash(b"external signer round trip").to_byte_array();
        let msg = Message::from_digest(digest);
        assert_eq!(
            adapter.sign_ecdsa(msg, &path).await.unwrap(),
            reference.sign_ecdsa(msg, &path).await.unwrap(),
        );

        // sign_ecdsa_recoverable exercises the 65-byte `[31 + recovery_id] +
        // compact` round-trip through `recoverable_from_bytes`; deterministic
        // (RFC6979), so it matches the reference exactly.
        assert_eq!(
            adapter
                .sign_ecdsa_recoverable(msg, &path)
                .await
                .unwrap()
                .serialize_compact(),
            reference
                .sign_ecdsa_recoverable(msg, &path)
                .await
                .unwrap()
                .serialize_compact(),
        );

        // Schnorr uses random nonces, so verify the adapter's signature is valid
        // over the same digest and key.
        let sig = adapter.sign_hash_schnorr(&digest, &path).await.unwrap();
        let pk = reference.derive_public_key(&path).await.unwrap();
        let secp = Secp256k1::verification_only();
        secp.verify_schnorr(
            &sig,
            &Message::from_digest(digest),
            &XOnlyPublicKey::from(pk),
        )
        .expect("adapter schnorr signature must verify");
    }
}
