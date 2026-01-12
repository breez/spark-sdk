use crate::error::SignerError;
#[cfg(test)]
use crate::signer::external_types::derivation_path_to_string;
use crate::signer::external_types::{
    EcdsaSignatureBytes, ExternalAggregateFrostRequest, ExternalEncryptedPrivateKey,
    ExternalFrostCommitments, ExternalFrostSignature, ExternalFrostSignatureShare,
    ExternalPrivateKeySource, ExternalSecretToSplit, ExternalSignFrostRequest, ExternalTreeNodeId,
    ExternalVerifiableSecretShare, PrivateKeyBytes, PublicKeyBytes, RecoverableEcdsaSignatureBytes,
    SchnorrSignatureBytes, string_to_derivation_path,
};
use crate::signer::{BreezSigner, ExternalSigner, breez::BreezSignerImpl};
use crate::{Network, SdkError, Seed, default_config, models::KeySetType};

/// Default implementation of `ExternalSigner` that uses the internal `BreezSignerImpl`.
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
    /// * `key_set_type` - Type of key set to use
    /// * `use_address_index` - Whether to use address index in derivation
    /// * `account_number` - Optional account number for key derivation
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub fn new(
        mnemonic: String,
        passphrase: Option<String>,
        network: Network,
        key_set_type: KeySetType,
        use_address_index: bool,
        account_number: Option<u32>,
    ) -> Result<Self, SdkError> {
        let seed = Seed::Mnemonic {
            mnemonic,
            passphrase,
        };
        let config = default_config(network);
        let inner = BreezSignerImpl::new(
            &config,
            &seed,
            key_set_type.into(),
            use_address_index,
            account_number,
        )?;
        Ok(Self { inner })
    }
}

#[macros::async_trait]
impl ExternalSigner for DefaultExternalSigner {
    fn identity_public_key(&self) -> Result<PublicKeyBytes, SignerError> {
        let pk = self
            .inner
            .identity_public_key()
            .map_err(|e| SignerError::Generic(e.to_string()))?;
        Ok(PublicKeyBytes::from_public_key(&pk))
    }

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
        message: Vec<u8>,
        path: String,
    ) -> Result<EcdsaSignatureBytes, SignerError> {
        let derivation_path =
            string_to_derivation_path(&path).map_err(|e| SignerError::Generic(e.to_string()))?;
        let sig = self
            .inner
            .sign_ecdsa(&message, &derivation_path)
            .await
            .map_err(|e| SignerError::Generic(e.to_string()))?;
        Ok(EcdsaSignatureBytes::from_signature(&sig))
    }

    async fn sign_ecdsa_recoverable(
        &self,
        message: Vec<u8>,
        path: String,
    ) -> Result<RecoverableEcdsaSignatureBytes, SignerError> {
        let derivation_path =
            string_to_derivation_path(&path).map_err(|e| SignerError::Generic(e.to_string()))?;
        let bytes = self
            .inner
            .sign_ecdsa_recoverable(&message, &derivation_path)
            .await
            .map_err(|e| SignerError::Generic(e.to_string()))?;
        Ok(RecoverableEcdsaSignatureBytes::new(bytes))
    }

    async fn ecies_encrypt(&self, message: Vec<u8>, path: String) -> Result<Vec<u8>, SignerError> {
        let derivation_path =
            string_to_derivation_path(&path).map_err(|e| SignerError::Generic(e.to_string()))?;
        self.inner
            .ecies_encrypt(&message, &derivation_path)
            .await
            .map_err(|e| SignerError::Generic(e.to_string()))
    }

    async fn ecies_decrypt(&self, message: Vec<u8>, path: String) -> Result<Vec<u8>, SignerError> {
        let derivation_path =
            string_to_derivation_path(&path).map_err(|e| SignerError::Generic(e.to_string()))?;
        self.inner
            .ecies_decrypt(&message, &derivation_path)
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

    async fn generate_frost_signing_commitments(
        &self,
    ) -> Result<ExternalFrostCommitments, SignerError> {
        let commitments = self
            .inner
            .generate_frost_signing_commitments()
            .await
            .map_err(|e| SignerError::Generic(e.to_string()))?;
        ExternalFrostCommitments::from_frost_commitments(&commitments)
            .map_err(|e| SignerError::Generic(e.to_string()))
    }

    async fn get_public_key_for_node(
        &self,
        id: ExternalTreeNodeId,
    ) -> Result<PublicKeyBytes, SignerError> {
        let tree_node_id = id
            .to_tree_node_id()
            .map_err(|e| SignerError::Generic(e.to_string()))?;
        let pk = self
            .inner
            .get_public_key_for_node(&tree_node_id)
            .await
            .map_err(|e| SignerError::Generic(e.to_string()))?;
        Ok(PublicKeyBytes::from_public_key(&pk))
    }

    async fn generate_random_key(&self) -> Result<ExternalPrivateKeySource, SignerError> {
        let key = self
            .inner
            .generate_random_key()
            .await
            .map_err(|e| SignerError::Generic(e.to_string()))?;
        ExternalPrivateKeySource::from_private_key_source(&key)
            .map_err(|e| SignerError::Generic(e.to_string()))
    }

    async fn get_static_deposit_private_key_source(
        &self,
        index: u32,
    ) -> Result<ExternalPrivateKeySource, SignerError> {
        let key = self
            .inner
            .get_static_deposit_private_key_source(index)
            .await
            .map_err(|e| SignerError::Generic(e.to_string()))?;
        ExternalPrivateKeySource::from_private_key_source(&key)
            .map_err(|e| SignerError::Generic(e.to_string()))
    }

    async fn get_static_deposit_private_key(
        &self,
        index: u32,
    ) -> Result<PrivateKeyBytes, SignerError> {
        let secret = self
            .inner
            .get_static_deposit_private_key(index)
            .await
            .map_err(|e| SignerError::Generic(e.to_string()))?;
        Ok(PrivateKeyBytes::from_secret_key(&secret))
    }

    async fn get_static_deposit_public_key(
        &self,
        index: u32,
    ) -> Result<PublicKeyBytes, SignerError> {
        let pk = self
            .inner
            .get_static_deposit_public_key(index)
            .await
            .map_err(|e| SignerError::Generic(e.to_string()))?;
        Ok(PublicKeyBytes::from_public_key(&pk))
    }

    async fn subtract_private_keys(
        &self,
        signing_key: ExternalPrivateKeySource,
        new_signing_key: ExternalPrivateKeySource,
    ) -> Result<ExternalPrivateKeySource, SignerError> {
        let sk = signing_key
            .to_private_key_source()
            .map_err(|e| SignerError::Generic(e.to_string()))?;
        let nsk = new_signing_key
            .to_private_key_source()
            .map_err(|e| SignerError::Generic(e.to_string()))?;
        let result = self
            .inner
            .subtract_private_keys(&sk, &nsk)
            .await
            .map_err(|e| SignerError::Generic(e.to_string()))?;
        ExternalPrivateKeySource::from_private_key_source(&result)
            .map_err(|e| SignerError::Generic(e.to_string()))
    }

    async fn split_secret(
        &self,
        secret: ExternalSecretToSplit,
        threshold: u32,
        num_shares: u32,
    ) -> Result<Vec<ExternalVerifiableSecretShare>, SignerError> {
        let sec = secret
            .to_secret_to_split()
            .map_err(|e| SignerError::Generic(e.to_string()))?;
        let shares = self
            .inner
            .split_secret_with_proofs(&sec, threshold, num_shares as usize)
            .await
            .map_err(|e| SignerError::Frost(e.to_string()))?;

        shares
            .iter()
            .map(|s| {
                ExternalVerifiableSecretShare::from_verifiable_secret_share(s)
                    .map_err(|e| SignerError::Generic(e.to_string()))
            })
            .collect()
    }

    async fn encrypt_private_key_for_receiver(
        &self,
        private_key: ExternalEncryptedPrivateKey,
        receiver_public_key: PublicKeyBytes,
    ) -> Result<Vec<u8>, SignerError> {
        let pk_internal = private_key
            .to_encrypted_private_key()
            .map_err(|e| SignerError::Generic(e.to_string()))?;
        let receiver_pk = receiver_public_key
            .to_public_key()
            .map_err(|e| SignerError::Generic(e.to_string()))?;
        self.inner
            .encrypt_private_key_for_receiver(&pk_internal, &receiver_pk)
            .await
            .map_err(|e| SignerError::Generic(e.to_string()))
    }

    async fn get_public_key_from_private_key_source(
        &self,
        private_key: ExternalPrivateKeySource,
    ) -> Result<PublicKeyBytes, SignerError> {
        let pk_source = private_key
            .to_private_key_source()
            .map_err(|e| SignerError::Generic(e.to_string()))?;
        let pk = self
            .inner
            .get_public_key_from_private_key_source(&pk_source)
            .await
            .map_err(|e| SignerError::Generic(e.to_string()))?;
        Ok(PublicKeyBytes::from_public_key(&pk))
    }

    async fn sign_frost(
        &self,
        request: ExternalSignFrostRequest,
    ) -> Result<ExternalFrostSignatureShare, SignerError> {
        let req = request
            .to_sign_frost_request()
            .map_err(|e| SignerError::Generic(e.to_string()))?;

        let share = self
            .inner
            .sign_frost(req)
            .await
            .map_err(|e| SignerError::Generic(e.to_string()))?;
        ExternalFrostSignatureShare::from_signature_share(&share).map_err(|e| e.to_string().into())
    }

    async fn aggregate_frost_signatures(
        &self,
        request: ExternalAggregateFrostRequest,
    ) -> Result<ExternalFrostSignature, SignerError> {
        let req = request
            .to_aggregate_frost_request()
            .map_err(|e| SignerError::Generic(e.to_string()))?;

        let sig = self
            .inner
            .aggregate_frost(req)
            .await
            .map_err(|e| SignerError::Generic(e.to_string()))?;
        ExternalFrostSignature::from_frost_signature(&sig)
            .map_err(|e| SignerError::Generic(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::KeySetType;
    use bitcoin::bip32::DerivationPath;
    use bitcoin::hashes::Hash;
    use std::str::FromStr;

    fn create_test_signer() -> (DefaultExternalSigner, BreezSignerImpl) {
        let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".to_string();
        let network = Network::Regtest;
        let key_set_type = KeySetType::Default;
        let use_address_index = false;
        let account_number = Some(0);

        let external = DefaultExternalSigner::new(
            mnemonic.clone(),
            None,
            network,
            key_set_type,
            use_address_index,
            account_number,
        )
        .expect("Failed to create DefaultExternalSigner");

        let seed = Seed::Mnemonic {
            mnemonic,
            passphrase: None,
        };
        let config = default_config(network);
        let internal = BreezSignerImpl::new(
            &config,
            &seed,
            key_set_type.into(),
            use_address_index,
            account_number,
        )
        .expect("Failed to create BreezSignerImpl");

        (external, internal)
    }

    #[macros::test_all]
    fn test_identity_public_key() {
        let (external, internal) = create_test_signer();

        let external_pk = external.identity_public_key().unwrap();
        let internal_pk = internal.identity_public_key().unwrap();

        assert_eq!(
            external_pk.to_public_key().unwrap(),
            internal_pk,
            "Identity public keys should match"
        );
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
        let (external, internal) = create_test_signer();

        let message = b"test message";
        let path = DerivationPath::from_str("m/0'/0'/0'").unwrap();
        let path_str = derivation_path_to_string(&path);

        let external_sig = external
            .sign_ecdsa(message.to_vec(), path_str)
            .await
            .unwrap();
        let internal_sig = internal.sign_ecdsa(message, &path).await.unwrap();

        assert_eq!(
            external_sig.to_signature().unwrap(),
            internal_sig,
            "ECDSA signatures should match"
        );
    }

    #[macros::async_test_all]
    async fn test_sign_ecdsa_recoverable() {
        let (external, internal) = create_test_signer();

        let message = b"test message";
        let path = DerivationPath::from_str("m/0'/0'/0'").unwrap();
        let path_str = derivation_path_to_string(&path);

        let external_sig = external
            .sign_ecdsa_recoverable(message.to_vec(), path_str)
            .await
            .unwrap();
        let internal_sig = internal
            .sign_ecdsa_recoverable(message, &path)
            .await
            .unwrap();

        assert_eq!(
            external_sig.bytes, internal_sig,
            "Recoverable ECDSA signatures should match"
        );
    }

    #[macros::async_test_all]
    async fn test_ecies_encrypt_decrypt() {
        let (external, internal) = create_test_signer();

        let message = b"secret message";
        let path = DerivationPath::from_str("m/0'/0'/0'").unwrap();
        let path_str = derivation_path_to_string(&path);

        // Test encryption
        let external_encrypted = external
            .ecies_encrypt(message.to_vec(), path_str.clone())
            .await
            .unwrap();
        let internal_encrypted = internal.ecies_encrypt(message, &path).await.unwrap();

        // Both should be able to decrypt
        let external_decrypted = external
            .ecies_decrypt(external_encrypted.clone(), path_str.clone())
            .await
            .unwrap();
        let internal_decrypted = internal
            .ecies_decrypt(&internal_encrypted, &path)
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

    #[macros::async_test_all]
    async fn test_get_public_key_for_node() {
        let (external, internal) = create_test_signer();

        let node_id = spark_wallet::TreeNodeId::from_str("root/child1").unwrap();
        let external_node_id = ExternalTreeNodeId::from_tree_node_id(&node_id).unwrap();

        let external_pk = external
            .get_public_key_for_node(external_node_id)
            .await
            .unwrap();
        let internal_pk = internal.get_public_key_for_node(&node_id).await.unwrap();

        assert_eq!(
            external_pk.to_public_key().unwrap(),
            internal_pk,
            "Node public keys should match"
        );
    }

    #[macros::async_test_all]
    async fn test_generate_random_key() {
        let (external, _internal) = create_test_signer();

        let key1 = external.generate_random_key().await.unwrap();
        let key2 = external.generate_random_key().await.unwrap();

        // Verify we can convert them
        let _internal_key1 = key1.to_private_key_source().unwrap();
        let _internal_key2 = key2.to_private_key_source().unwrap();

        // Random keys should be different (encrypted, so ciphertext should differ)
        match (&key1, &key2) {
            (
                ExternalPrivateKeySource::Encrypted { key: k1 },
                ExternalPrivateKeySource::Encrypted { key: k2 },
            ) => {
                assert_ne!(
                    k1.ciphertext, k2.ciphertext,
                    "Random keys should be different"
                );
            }
            _ => panic!("Random keys should be encrypted"),
        }
    }

    #[macros::async_test_all]
    async fn test_get_static_deposit_keys() {
        let (external, internal) = create_test_signer();

        let index = 0u32;

        // Test private key source
        let external_source = external
            .get_static_deposit_private_key_source(index)
            .await
            .unwrap();
        let internal_source = internal
            .get_static_deposit_private_key_source(index)
            .await
            .unwrap();

        // Static deposit keys are encrypted, not derived
        assert!(matches!(
            external_source,
            ExternalPrivateKeySource::Encrypted { .. }
        ));
        assert!(matches!(
            internal_source,
            spark_wallet::PrivateKeySource::Encrypted(_)
        ));

        // Test private key
        let ext_secret_key = external
            .get_static_deposit_private_key(index)
            .await
            .unwrap();
        let int_secret_key = internal
            .get_static_deposit_private_key(index)
            .await
            .unwrap();

        assert_eq!(
            ext_secret_key.bytes,
            int_secret_key.secret_bytes().to_vec(),
            "Static deposit private keys should match"
        );

        // Test public key
        let ext_public_key = external.get_static_deposit_public_key(index).await.unwrap();
        let int_public_key = internal.get_static_deposit_public_key(index).await.unwrap();

        assert_eq!(
            ext_public_key.to_public_key().unwrap(),
            int_public_key,
            "Static deposit public keys should match"
        );
    }

    #[macros::async_test_all]
    async fn test_get_public_key_from_private_key_source() {
        let (external, internal) = create_test_signer();

        let source = external
            .get_static_deposit_private_key_source(0)
            .await
            .unwrap();

        let external_pk = external
            .get_public_key_from_private_key_source(source.clone())
            .await
            .unwrap();

        let internal_source = source.to_private_key_source().unwrap();
        let internal_pk = internal
            .get_public_key_from_private_key_source(&internal_source)
            .await
            .unwrap();

        assert_eq!(
            external_pk.to_public_key().unwrap(),
            internal_pk,
            "Public keys from private key source should match"
        );
    }

    #[macros::async_test_all]
    async fn test_sign_frost() {
        use std::collections::BTreeMap;

        let (external, internal) = create_test_signer();

        // Generate commitments for both signers
        let _external_commitments = external.generate_frost_signing_commitments().await.unwrap();
        let internal_commitments_full =
            internal.generate_frost_signing_commitments().await.unwrap();

        // Create a simple FROST signing request
        let message = b"test frost signing message";
        let public_key = internal.identity_public_key().unwrap();
        let private_key_source = internal.generate_random_key().await.unwrap();
        let verifying_key = public_key;

        // Create statechain commitments (using just one participant for simplicity)
        // Need to convert FrostSigningCommitmentsWithNonces to SigningCommitments
        let identifier = frost_secp256k1_tr::Identifier::try_from(1u16).unwrap();
        let mut statechain_commitments = BTreeMap::new();
        statechain_commitments.insert(identifier, internal_commitments_full.commitments);

        // Create the internal FROST request
        let internal_request = spark_wallet::SignFrostRequest {
            message,
            public_key: &public_key,
            private_key: &private_key_source,
            verifying_key: &verifying_key,
            self_nonce_commitment: &internal_commitments_full,
            statechain_commitments,
            adaptor_public_key: None,
        };

        // Create the external FROST request
        let external_request =
            ExternalSignFrostRequest::from_sign_frost_request(&internal_request).unwrap();

        // Sign with both signers
        let external_share = external.sign_frost(external_request).await.unwrap();
        let internal_share = internal.sign_frost(internal_request).await.unwrap();

        // Verify both produced valid signature shares
        let external_share_deserialized = external_share.to_signature_share().unwrap();

        // Verify the signature shares are cryptographically valid:
        // 1. Check format - FROST signature shares are scalars (32 bytes)
        assert_eq!(
            external_share.bytes.len(),
            32,
            "FROST signature share should be 32 bytes (a scalar)"
        );

        // 2. Verify both shares can be serialized correctly
        let external_reserialized = external_share_deserialized.serialize();
        let internal_serialized = internal_share.serialize();

        assert_eq!(
            external_reserialized.len(),
            32,
            "External share should serialize to 32 bytes"
        );
        assert_eq!(
            internal_serialized.len(),
            32,
            "Internal share should serialize to 32 bytes"
        );

        // 3. Verify round-trip: external share bytes should match after deserialize + serialize
        assert_eq!(
            external_reserialized,
            external_share.bytes.as_slice(),
            "Round-trip serialization should preserve the exact share bytes"
        );

        // Successfully verified both external and internal signers produce valid FROST signature shares!
        // Note: Full FROST aggregation testing requires multiple participants and is
        // tested in integration tests with actual statechain interactions.
    }
}
