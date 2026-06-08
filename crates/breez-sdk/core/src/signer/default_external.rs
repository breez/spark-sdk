use crate::error::SignerError;
#[cfg(test)]
use crate::signer::external_types::derivation_path_to_string;
use crate::signer::external_types::{
    EcdsaSignatureBytes, HashedMessageBytes, MessageBytes, PublicKeyBytes,
    RecoverableEcdsaSignatureBytes, SchnorrSignatureBytes, string_to_derivation_path,
};
use crate::signer::{BreezSigner, ExternalBreezSigner, breez::BreezSignerImpl};
use crate::{Network, SdkError, Seed};

/// Derives the identity master Xpriv (the `BreezSigner` derivation root) from a
/// mnemonic seed. Key derivation lives in the Spark layer; the SDK-layer
/// `BreezSigner` just consumes the resulting master.
fn identity_master_key(
    seed: &Seed,
    network: Network,
    account_number: Option<u32>,
) -> Result<bitcoin::bip32::Xpriv, SdkError> {
    let seed_bytes = seed.to_bytes()?;
    spark_wallet::identity_master_key(&seed_bytes, network.into(), account_number)
        .map_err(|e| SdkError::Generic(e.to_string()))
}

/// Default implementation of `ExternalBreezSigner` that uses the internal `BreezSignerImpl`.
///
/// This provides a reference implementation and allows users to easily create a signer
/// from a mnemonic without implementing the trait themselves.
pub struct DefaultExternalSigner {
    inner: BreezSignerImpl,
}

impl DefaultExternalSigner {
    /// Creates a new `DefaultExternalSigner` from a mnemonic.
    ///
    /// # Arguments
    /// * `mnemonic` - BIP39 mnemonic phrase (12 or 24 words)
    /// * `passphrase` - Optional passphrase for the mnemonic
    /// * `network` - Network to use (Mainnet or Regtest)
    /// * `account_number` - Optional account number for key derivation
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub fn new(
        mnemonic: String,
        passphrase: Option<String>,
        network: Network,
        account_number: Option<u32>,
    ) -> Result<Self, SdkError> {
        let seed = Seed::Mnemonic {
            mnemonic,
            passphrase,
        };
        let master = identity_master_key(&seed, network, account_number)?;
        let inner = BreezSignerImpl::new(master);
        Ok(Self { inner })
    }
}

#[macros::async_trait]
impl ExternalBreezSigner for DefaultExternalSigner {
    async fn derive_public_key(&self, path: String) -> Result<PublicKeyBytes, SignerError> {
        let derivation_path =
            string_to_derivation_path(&path).map_err(|e| SignerError::Generic(e.to_string()))?;
        let pk = self
            .inner
            .derive_public_key(&derivation_path)
            .await
            .map_err(|e| SignerError::Generic(e.to_string()))?;
        Ok(PublicKeyBytes::from_public_key(&pk))
    }

    async fn sign_ecdsa(
        &self,
        message: MessageBytes,
        path: String,
    ) -> Result<EcdsaSignatureBytes, SignerError> {
        use bitcoin::secp256k1::Message;

        let derivation_path =
            string_to_derivation_path(&path).map_err(|e| SignerError::Generic(e.to_string()))?;
        let digest = message
            .to_digest()
            .map_err(|e| SignerError::Generic(e.to_string()))?;
        let msg = Message::from_digest(digest);
        let sig = self
            .inner
            .sign_ecdsa(msg, &derivation_path)
            .await
            .map_err(|e| SignerError::Generic(e.to_string()))?;
        Ok(EcdsaSignatureBytes::from_signature(&sig))
    }

    async fn sign_ecdsa_recoverable(
        &self,
        message: MessageBytes,
        path: String,
    ) -> Result<RecoverableEcdsaSignatureBytes, SignerError> {
        use bitcoin::secp256k1::Message;

        let derivation_path =
            string_to_derivation_path(&path).map_err(|e| SignerError::Generic(e.to_string()))?;
        let digest = message
            .to_digest()
            .map_err(|e| SignerError::Generic(e.to_string()))?;
        let msg = Message::from_digest(digest);
        let sig = self
            .inner
            .sign_ecdsa_recoverable(msg, &derivation_path)
            .await
            .map_err(|e| SignerError::Generic(e.to_string()))?;

        // Serialize the recoverable signature: recovery_id (31 + id) + 64-byte signature
        let (recovery_id, sig_bytes) = sig.serialize_compact();
        let mut bytes =
            vec![
                31u8.saturating_add(
                    u8::try_from(recovery_id.to_i32())
                        .map_err(|e| SignerError::Generic(e.to_string()))?,
                ),
            ];
        bytes.extend_from_slice(&sig_bytes);
        Ok(RecoverableEcdsaSignatureBytes::new(bytes))
    }

    async fn encrypt_ecies(&self, message: Vec<u8>, path: String) -> Result<Vec<u8>, SignerError> {
        let derivation_path =
            string_to_derivation_path(&path).map_err(|e| SignerError::Generic(e.to_string()))?;
        self.inner
            .encrypt_ecies(&message, &derivation_path)
            .await
            .map_err(|e| SignerError::Generic(e.to_string()))
    }

    async fn decrypt_ecies(&self, message: Vec<u8>, path: String) -> Result<Vec<u8>, SignerError> {
        let derivation_path =
            string_to_derivation_path(&path).map_err(|e| SignerError::Generic(e.to_string()))?;
        self.inner
            .decrypt_ecies(&message, &derivation_path)
            .await
            .map_err(|e| SignerError::Generic(e.to_string()))
    }

    async fn sign_hash_schnorr(
        &self,
        hash: Vec<u8>,
        path: String,
    ) -> Result<SchnorrSignatureBytes, SignerError> {
        let derivation_path =
            string_to_derivation_path(&path).map_err(|e| SignerError::Generic(e.to_string()))?;
        let sig = self
            .inner
            .sign_hash_schnorr(&hash, &derivation_path)
            .await
            .map_err(|e| SignerError::Generic(e.to_string()))?;
        Ok(SchnorrSignatureBytes::from_signature(&sig))
    }

    async fn hmac_sha256(
        &self,
        message: Vec<u8>,
        path: String,
    ) -> Result<HashedMessageBytes, SignerError> {
        let derivation_path =
            string_to_derivation_path(&path).map_err(|e| SignerError::Generic(e.to_string()))?;
        let sig = self
            .inner
            .hmac_sha256(&derivation_path, &message)
            .await
            .map_err(|e| SignerError::Generic(e.to_string()))?;
        Ok(HashedMessageBytes::from_hmac(&sig))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::bip32::DerivationPath;
    use bitcoin::hashes::Hash;
    use std::str::FromStr;

    fn create_test_signer() -> (DefaultExternalSigner, BreezSignerImpl) {
        let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".to_string();
        let network = Network::Regtest;
        let account_number = Some(0);

        let external = DefaultExternalSigner::new(mnemonic.clone(), None, network, account_number)
            .expect("Failed to create DefaultExternalSigner");

        let seed = Seed::Mnemonic {
            mnemonic,
            passphrase: None,
        };
        let master = identity_master_key(&seed, network, account_number)
            .expect("Failed to derive identity master key");
        let internal = BreezSignerImpl::new(master);

        (external, internal)
    }

    #[macros::async_test_all]
    async fn test_derive_public_key() {
        let (external, internal) = create_test_signer();

        let path = DerivationPath::from_str("m/0'/0'/0'").unwrap();
        let path_str = derivation_path_to_string(&path);

        let external_pk = external.derive_public_key(path_str).await.unwrap();
        let internal_pk = internal.derive_public_key(&path).await.unwrap();

        assert_eq!(
            external_pk.to_public_key().unwrap(),
            internal_pk,
            "Derived public keys should match"
        );
    }

    #[macros::async_test_all]
    async fn test_sign_ecdsa() {
        use bitcoin::secp256k1::Message;

        let (external, internal) = create_test_signer();

        let message = b"test message";
        let path = DerivationPath::from_str("m/0'/0'/0'").unwrap();
        let path_str = derivation_path_to_string(&path);

        // Hash the message first (as required by the new API)
        let hash = bitcoin::hashes::sha256::Hash::hash(message);
        let msg_bytes = MessageBytes::new(hash.to_byte_array().to_vec());
        let msg = Message::from_digest(hash.to_byte_array());

        let external_sig = external.sign_ecdsa(msg_bytes, path_str).await.unwrap();
        let internal_sig = internal.sign_ecdsa(msg, &path).await.unwrap();

        assert_eq!(
            external_sig.to_signature().unwrap(),
            internal_sig,
            "ECDSA signatures should match"
        );
    }

    #[macros::async_test_all]
    async fn test_sign_ecdsa_recoverable() {
        use bitcoin::secp256k1::Message;

        let (external, internal) = create_test_signer();

        let message = b"test message";
        let path = DerivationPath::from_str("m/0'/0'/0'").unwrap();
        let path_str = derivation_path_to_string(&path);

        // Double-hash the message (as it was done internally before)
        let hash = bitcoin::hashes::sha256::Hash::hash(
            bitcoin::hashes::sha256::Hash::hash(message).as_ref(),
        );
        let msg_bytes = MessageBytes::new(hash.to_byte_array().to_vec());
        let msg = Message::from_digest(hash.to_byte_array());

        let external_sig = external
            .sign_ecdsa_recoverable(msg_bytes, path_str)
            .await
            .unwrap();
        let internal_sig = internal.sign_ecdsa_recoverable(msg, &path).await.unwrap();

        // Serialize internal signature for comparison
        let (recovery_id, sig_bytes) = internal_sig.serialize_compact();
        let mut internal_bytes =
            vec![31u8.saturating_add(u8::try_from(recovery_id.to_i32()).unwrap())];
        internal_bytes.extend_from_slice(&sig_bytes);

        assert_eq!(
            external_sig.bytes, internal_bytes,
            "Recoverable ECDSA signatures should match"
        );
    }

    #[macros::async_test_all]
    async fn test_encrypt_decrypt_ecies() {
        let (external, internal) = create_test_signer();

        let message = b"secret message";
        let path = DerivationPath::from_str("m/0'/0'/0'").unwrap();
        let path_str = derivation_path_to_string(&path);

        // Test encryption
        let external_encrypted = external
            .encrypt_ecies(message.to_vec(), path_str.clone())
            .await
            .unwrap();
        let internal_encrypted = internal.encrypt_ecies(message, &path).await.unwrap();

        // Both should be able to decrypt
        let external_decrypted = external
            .decrypt_ecies(external_encrypted.clone(), path_str.clone())
            .await
            .unwrap();
        let internal_decrypted = internal
            .decrypt_ecies(&internal_encrypted, &path)
            .await
            .unwrap();

        assert_eq!(
            external_decrypted, message,
            "External decrypt should recover original message"
        );
        assert_eq!(
            internal_decrypted, message,
            "Internal decrypt should recover original message"
        );
    }

    #[macros::async_test_all]
    async fn test_sign_hash_schnorr() {
        use bitcoin::secp256k1::{Message, XOnlyPublicKey};

        let (external, internal) = create_test_signer();

        let hash = bitcoin::hashes::sha256::Hash::hash(b"test")
            .to_byte_array()
            .to_vec();
        let path = DerivationPath::from_str("m/0'/0'/0'").unwrap();
        let path_str = derivation_path_to_string(&path);

        // Get the public key for this derivation path
        let pubkey = internal.derive_public_key(&path).await.unwrap();
        let x_only_pubkey = XOnlyPublicKey::from(pubkey);

        // Sign with both signers
        let external_sig = external
            .sign_hash_schnorr(hash.clone(), path_str)
            .await
            .unwrap();
        let internal_sig = internal.sign_hash_schnorr(&hash, &path).await.unwrap();

        // Schnorr signatures use random nonces, so they won't be identical
        // but both should be valid signatures over the same message with the same key
        let external_schnorr = external_sig.to_signature().unwrap();

        // Verify both signatures are valid
        let message = Message::from_digest_slice(&hash).unwrap();
        let secp = bitcoin::secp256k1::Secp256k1::verification_only();

        secp.verify_schnorr(&external_schnorr, &message, &x_only_pubkey)
            .expect("External signature should be valid");
        secp.verify_schnorr(&internal_sig, &message, &x_only_pubkey)
            .expect("Internal signature should be valid");

        assert_eq!(
            external_sig.bytes.len(),
            64,
            "Schnorr signature should be 64 bytes"
        );
    }
}
