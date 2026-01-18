use breez_sdk_spark::signer::external_types as core_types;
use macros::async_trait;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_futures::js_sys::Promise;

// WASM-compatible wrapper types
// These mirror the core external_types but are wasm_bindgen compatible

#[macros::extern_wasm_bindgen(breez_sdk_spark::signer::external_types::PublicKeyBytes)]
pub struct PublicKeyBytes {
    pub bytes: Vec<u8>,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::signer::external_types::EcdsaSignatureBytes)]
pub struct EcdsaSignatureBytes {
    pub bytes: Vec<u8>,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::signer::external_types::SchnorrSignatureBytes)]
pub struct SchnorrSignatureBytes {
    pub bytes: Vec<u8>,
}

#[macros::extern_wasm_bindgen(
    breez_sdk_spark::signer::external_types::RecoverableEcdsaSignatureBytes
)]
pub struct RecoverableEcdsaSignatureBytes {
    pub bytes: Vec<u8>,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::signer::external_types::PrivateKeyBytes)]
pub struct PrivateKeyBytes {
    pub bytes: Vec<u8>,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::signer::external_types::HashedMessageBytes)]
pub struct HashedMessageBytes {
    pub bytes: Vec<u8>,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::signer::external_types::MessageBytes)]
pub struct MessageBytes {
    pub bytes: Vec<u8>,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::signer::external_types::ExternalTreeNodeId)]
pub struct ExternalTreeNodeId {
    pub id: String,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::signer::external_types::ExternalEncryptedPrivateKey)]
pub struct ExternalEncryptedPrivateKey {
    pub ciphertext: Vec<u8>,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::signer::external_types::ExternalPrivateKeySource)]
pub enum ExternalPrivateKeySource {
    Derived { node_id: ExternalTreeNodeId },
    Encrypted { key: ExternalEncryptedPrivateKey },
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::signer::external_types::ExternalSecretToSplit)]
pub enum ExternalSecretToSplit {
    PrivateKey { source: ExternalPrivateKeySource },
    Preimage { data: Vec<u8> },
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::signer::external_types::ExternalScalar)]
pub struct ExternalScalar {
    pub bytes: Vec<u8>,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::signer::external_types::ExternalSecretShare)]
pub struct ExternalSecretShare {
    pub threshold: u32,
    pub index: ExternalScalar,
    pub share: ExternalScalar,
}

#[macros::extern_wasm_bindgen(
    breez_sdk_spark::signer::external_types::ExternalVerifiableSecretShare
)]
pub struct ExternalVerifiableSecretShare {
    pub secret_share: ExternalSecretShare,
    pub proofs: Vec<Vec<u8>>,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::signer::external_types::ExternalFrostCommitments)]
pub struct ExternalFrostCommitments {
    pub hiding_commitment: Vec<u8>,
    pub binding_commitment: Vec<u8>,
    pub nonces_ciphertext: Vec<u8>,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::signer::external_types::ExternalIdentifier)]
pub struct ExternalIdentifier {
    pub bytes: Vec<u8>,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::signer::external_types::ExternalSigningCommitments)]
pub struct ExternalSigningCommitments {
    pub hiding: Vec<u8>,
    pub binding: Vec<u8>,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::signer::external_types::IdentifierCommitmentPair)]
pub struct IdentifierCommitmentPair {
    pub identifier: ExternalIdentifier,
    pub commitment: ExternalSigningCommitments,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::signer::external_types::IdentifierSignaturePair)]
pub struct IdentifierSignaturePair {
    pub identifier: ExternalIdentifier,
    pub signature: ExternalFrostSignatureShare,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::signer::external_types::IdentifierPublicKeyPair)]
pub struct IdentifierPublicKeyPair {
    pub identifier: ExternalIdentifier,
    pub public_key: Vec<u8>,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::signer::external_types::ExternalSignFrostRequest)]
pub struct ExternalSignFrostRequest {
    pub message: Vec<u8>,
    pub public_key: Vec<u8>,
    pub private_key: ExternalPrivateKeySource,
    pub verifying_key: Vec<u8>,
    pub self_nonce_commitment: ExternalFrostCommitments,
    pub statechain_commitments: Vec<IdentifierCommitmentPair>,
    pub adaptor_public_key: Option<Vec<u8>>,
}

#[macros::extern_wasm_bindgen(
    breez_sdk_spark::signer::external_types::ExternalAggregateFrostRequest
)]
pub struct ExternalAggregateFrostRequest {
    pub message: Vec<u8>,
    pub statechain_signatures: Vec<IdentifierSignaturePair>,
    pub statechain_public_keys: Vec<IdentifierPublicKeyPair>,
    pub verifying_key: Vec<u8>,
    pub statechain_commitments: Vec<IdentifierCommitmentPair>,
    pub self_commitment: ExternalSigningCommitments,
    pub public_key: Vec<u8>,
    pub self_signature: ExternalFrostSignatureShare,
    pub adaptor_public_key: Option<Vec<u8>>,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::signer::external_types::ExternalFrostSignatureShare)]
pub struct ExternalFrostSignatureShare {
    pub bytes: Vec<u8>,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::signer::external_types::ExternalFrostSignature)]
pub struct ExternalFrostSignature {
    pub bytes: Vec<u8>,
}

pub struct WasmExternalSigner {
    pub inner: JsExternalSigner,
}

// This assumes that we'll always be running in a single thread (true for Wasm environments)
unsafe impl Send for WasmExternalSigner {}
unsafe impl Sync for WasmExternalSigner {}

impl WasmExternalSigner {
    pub fn new(inner: JsExternalSigner) -> Self {
        Self { inner }
    }
}

/// A default signer implementation that wraps the core SDK's ExternalSigner.
/// This is returned by `defaultExternalSigner` and can be passed to `connectWithSigner`.
#[wasm_bindgen]
pub struct DefaultSigner {
    pub(crate) inner: std::sync::Arc<dyn breez_sdk_spark::signer::ExternalSigner>,
}

// This assumes that we'll always be running in a single thread (true for Wasm environments)
unsafe impl Send for DefaultSigner {}
unsafe impl Sync for DefaultSigner {}

impl DefaultSigner {
    pub fn new(inner: std::sync::Arc<dyn breez_sdk_spark::signer::ExternalSigner>) -> Self {
        Self { inner }
    }
}

#[wasm_bindgen]
impl DefaultSigner {
    #[wasm_bindgen(js_name = "identityPublicKey")]
    pub fn identity_public_key(&self) -> Result<PublicKeyBytes, JsValue> {
        self.inner
            .identity_public_key()
            .map(|pk| pk.into())
            .map_err(|e| JsValue::from_str(&format!("{e:?}")))
    }

    #[wasm_bindgen(js_name = "derivePublicKey")]
    pub async fn derive_public_key(&self, path: String) -> Result<PublicKeyBytes, JsValue> {
        self.inner
            .derive_public_key(path)
            .await
            .map(|pk| pk.into())
            .map_err(|e| JsValue::from_str(&format!("{e:?}")))
    }

    #[wasm_bindgen(js_name = "signEcdsa")]
    pub async fn sign_ecdsa(
        &self,
        message: MessageBytes,
        path: String,
    ) -> Result<EcdsaSignatureBytes, JsValue> {
        self.inner
            .sign_ecdsa(message.into(), path)
            .await
            .map(|sig| sig.into())
            .map_err(|e| JsValue::from_str(&format!("{e:?}")))
    }

    #[wasm_bindgen(js_name = "signEcdsaRecoverable")]
    pub async fn sign_ecdsa_recoverable(
        &self,
        message: MessageBytes,
        path: String,
    ) -> Result<RecoverableEcdsaSignatureBytes, JsValue> {
        self.inner
            .sign_ecdsa_recoverable(message.into(), path)
            .await
            .map(|sig| sig.into())
            .map_err(|e| JsValue::from_str(&format!("{e:?}")))
    }

    #[wasm_bindgen(js_name = "eciesEncrypt")]
    pub async fn encrypt_ecies(&self, message: Vec<u8>, path: String) -> Result<Vec<u8>, JsValue> {
        self.inner
            .encrypt_ecies(message, path)
            .await
            .map_err(|e| JsValue::from_str(&format!("{e:?}")))
    }

    #[wasm_bindgen(js_name = "eciesDecrypt")]
    pub async fn decrypt_ecies(&self, message: Vec<u8>, path: String) -> Result<Vec<u8>, JsValue> {
        self.inner
            .decrypt_ecies(message, path)
            .await
            .map_err(|e| JsValue::from_str(&format!("{e:?}")))
    }

    #[wasm_bindgen(js_name = "signHashSchnorr")]
    pub async fn sign_hash_schnorr(
        &self,
        hash: Vec<u8>,
        path: String,
    ) -> Result<SchnorrSignatureBytes, JsValue> {
        self.inner
            .sign_hash_schnorr(hash, path)
            .await
            .map(|sig| sig.into())
            .map_err(|e| JsValue::from_str(&format!("{e:?}")))
    }

    #[wasm_bindgen(js_name = "generateFrostSigningCommitments")]
    pub async fn generate_random_signing_commitment(
        &self,
    ) -> Result<ExternalFrostCommitments, JsValue> {
        self.inner
            .generate_random_signing_commitment()
            .await
            .map(|c| c.into())
            .map_err(|e| JsValue::from_str(&format!("{e:?}")))
    }

    #[wasm_bindgen(js_name = "getPublicKeyForNode")]
    pub async fn get_public_key_for_node(
        &self,
        id: ExternalTreeNodeId,
    ) -> Result<PublicKeyBytes, JsValue> {
        self.inner
            .get_public_key_for_node(id.into())
            .await
            .map(|pk| pk.into())
            .map_err(|e| JsValue::from_str(&format!("{e:?}")))
    }

    #[wasm_bindgen(js_name = "generateRandomKey")]
    pub async fn generate_random_key(&self) -> Result<ExternalPrivateKeySource, JsValue> {
        self.inner
            .generate_random_key()
            .await
            .map(|k| k.into())
            .map_err(|e| JsValue::from_str(&format!("{e:?}")))
    }

    #[wasm_bindgen(js_name = "getStaticDepositPrivateKeySource")]
    pub async fn get_static_deposit_private_key_source(
        &self,
        index: u32,
    ) -> Result<ExternalPrivateKeySource, JsValue> {
        self.inner
            .get_static_deposit_private_key_source(index)
            .await
            .map(|k| k.into())
            .map_err(|e| JsValue::from_str(&format!("{e:?}")))
    }

    #[wasm_bindgen(js_name = "getStaticDepositPrivateKey")]
    pub async fn static_deposit_secret_key(&self, index: u32) -> Result<PrivateKeyBytes, JsValue> {
        self.inner
            .static_deposit_secret_key(index)
            .await
            .map(|k| k.into())
            .map_err(|e| JsValue::from_str(&format!("{e:?}")))
    }

    #[wasm_bindgen(js_name = "getStaticDepositPublicKey")]
    pub async fn static_deposit_signing_key(&self, index: u32) -> Result<PublicKeyBytes, JsValue> {
        self.inner
            .static_deposit_signing_key(index)
            .await
            .map(|pk| pk.into())
            .map_err(|e| JsValue::from_str(&format!("{e:?}")))
    }

    #[wasm_bindgen(js_name = "subtractPrivateKeys")]
    pub async fn subtract_secret_keys(
        &self,
        signing_key: ExternalPrivateKeySource,
        new_signing_key: ExternalPrivateKeySource,
    ) -> Result<ExternalPrivateKeySource, JsValue> {
        self.inner
            .subtract_secret_keys(signing_key.into(), new_signing_key.into())
            .await
            .map(|k| k.into())
            .map_err(|e| JsValue::from_str(&format!("{e:?}")))
    }

    #[wasm_bindgen(js_name = "splitSecretWithProofs")]
    pub async fn split_secret_with_proofs(
        &self,
        secret: ExternalSecretToSplit,
        threshold: u32,
        num_shares: u32,
    ) -> Result<Box<[ExternalVerifiableSecretShare]>, JsValue> {
        self.inner
            .split_secret_with_proofs(secret.into(), threshold, num_shares)
            .await
            .map(|shares| {
                shares
                    .into_iter()
                    .map(|s| s.into())
                    .collect::<Vec<_>>()
                    .into_boxed_slice()
            })
            .map_err(|e| JsValue::from_str(&format!("{e:?}")))
    }

    #[wasm_bindgen(js_name = "encryptPrivateKeyForReceiver")]
    pub async fn encrypt_private_key_for_receiver(
        &self,
        private_key: ExternalEncryptedPrivateKey,
        receiver_public_key: PublicKeyBytes,
    ) -> Result<Vec<u8>, JsValue> {
        self.inner
            .encrypt_private_key_for_receiver(private_key.into(), receiver_public_key.into())
            .await
            .map_err(|e| JsValue::from_str(&format!("{e:?}")))
    }

    #[wasm_bindgen(js_name = "getPublicKeyFromPrivateKeySource")]
    pub async fn get_public_key_from_private_key_source(
        &self,
        private_key: ExternalPrivateKeySource,
    ) -> Result<PublicKeyBytes, JsValue> {
        self.inner
            .get_public_key_from_private_key_source(private_key.into())
            .await
            .map(|pk| pk.into())
            .map_err(|e| JsValue::from_str(&format!("{e:?}")))
    }

    #[wasm_bindgen(js_name = "signFrost")]
    pub async fn sign_frost(
        &self,
        request: ExternalSignFrostRequest,
    ) -> Result<ExternalFrostSignatureShare, JsValue> {
        self.inner
            .sign_frost(request.into())
            .await
            .map(|sig| sig.into())
            .map_err(|e| JsValue::from_str(&format!("{e:?}")))
    }

    #[wasm_bindgen(js_name = "aggregateFrost")]
    pub async fn aggregate_frost(
        &self,
        request: ExternalAggregateFrostRequest,
    ) -> Result<ExternalFrostSignature, JsValue> {
        self.inner
            .aggregate_frost(request.into())
            .await
            .map(|sig| sig.into())
            .map_err(|e| JsValue::from_str(&format!("{e:?}")))
    }

    #[wasm_bindgen(js_name = "hmacSha256")]
    pub async fn hmac_sha256(
        &self,
        message: Vec<u8>,
        path: String,
    ) -> Result<HashedMessageBytes, JsValue> {
        self.inner
            .hmac_sha256(message, path)
            .await
            .map(|h| h.into())
            .map_err(|e| JsValue::from_str(&format!("{e:?}")))
    }
}

use breez_sdk_spark::SignerError;

#[async_trait]
impl breez_sdk_spark::signer::ExternalSigner for DefaultSigner {
    fn identity_public_key(&self) -> Result<core_types::PublicKeyBytes, SignerError> {
        self.inner.identity_public_key()
    }

    async fn derive_public_key(
        &self,
        path: String,
    ) -> Result<core_types::PublicKeyBytes, SignerError> {
        self.inner.derive_public_key(path).await
    }

    async fn sign_ecdsa(
        &self,
        message: core_types::MessageBytes,
        path: String,
    ) -> Result<core_types::EcdsaSignatureBytes, SignerError> {
        self.inner.sign_ecdsa(message, path).await
    }

    async fn sign_ecdsa_recoverable(
        &self,
        message: core_types::MessageBytes,
        path: String,
    ) -> Result<core_types::RecoverableEcdsaSignatureBytes, SignerError> {
        self.inner.sign_ecdsa_recoverable(message, path).await
    }

    async fn encrypt_ecies(&self, message: Vec<u8>, path: String) -> Result<Vec<u8>, SignerError> {
        self.inner.encrypt_ecies(message, path).await
    }

    async fn decrypt_ecies(&self, message: Vec<u8>, path: String) -> Result<Vec<u8>, SignerError> {
        self.inner.decrypt_ecies(message, path).await
    }

    async fn sign_hash_schnorr(
        &self,
        hash: Vec<u8>,
        path: String,
    ) -> Result<core_types::SchnorrSignatureBytes, SignerError> {
        self.inner.sign_hash_schnorr(hash, path).await
    }

    async fn generate_random_signing_commitment(
        &self,
    ) -> Result<core_types::ExternalFrostCommitments, SignerError> {
        self.inner.generate_random_signing_commitment().await
    }

    async fn get_public_key_for_node(
        &self,
        id: core_types::ExternalTreeNodeId,
    ) -> Result<core_types::PublicKeyBytes, SignerError> {
        self.inner.get_public_key_for_node(id).await
    }

    async fn generate_random_key(
        &self,
    ) -> Result<core_types::ExternalPrivateKeySource, SignerError> {
        self.inner.generate_random_key().await
    }

    async fn get_static_deposit_private_key_source(
        &self,
        index: u32,
    ) -> Result<core_types::ExternalPrivateKeySource, SignerError> {
        self.inner
            .get_static_deposit_private_key_source(index)
            .await
    }

    async fn static_deposit_secret_key(
        &self,
        index: u32,
    ) -> Result<core_types::PrivateKeyBytes, SignerError> {
        self.inner.static_deposit_secret_key(index).await
    }

    async fn static_deposit_signing_key(
        &self,
        index: u32,
    ) -> Result<core_types::PublicKeyBytes, SignerError> {
        self.inner.static_deposit_signing_key(index).await
    }

    async fn subtract_secret_keys(
        &self,
        signing_key: core_types::ExternalPrivateKeySource,
        new_signing_key: core_types::ExternalPrivateKeySource,
    ) -> Result<core_types::ExternalPrivateKeySource, SignerError> {
        self.inner
            .subtract_secret_keys(signing_key, new_signing_key)
            .await
    }

    async fn split_secret_with_proofs(
        &self,
        secret: core_types::ExternalSecretToSplit,
        threshold: u32,
        num_shares: u32,
    ) -> Result<Vec<core_types::ExternalVerifiableSecretShare>, SignerError> {
        self.inner
            .split_secret_with_proofs(secret, threshold, num_shares)
            .await
    }

    async fn encrypt_private_key_for_receiver(
        &self,
        private_key: core_types::ExternalEncryptedPrivateKey,
        receiver_public_key: core_types::PublicKeyBytes,
    ) -> Result<Vec<u8>, SignerError> {
        self.inner
            .encrypt_private_key_for_receiver(private_key, receiver_public_key)
            .await
    }

    async fn get_public_key_from_private_key_source(
        &self,
        private_key: core_types::ExternalPrivateKeySource,
    ) -> Result<core_types::PublicKeyBytes, SignerError> {
        self.inner
            .get_public_key_from_private_key_source(private_key)
            .await
    }

    async fn sign_frost(
        &self,
        request: core_types::ExternalSignFrostRequest,
    ) -> Result<core_types::ExternalFrostSignatureShare, SignerError> {
        self.inner.sign_frost(request).await
    }

    async fn aggregate_frost(
        &self,
        request: core_types::ExternalAggregateFrostRequest,
    ) -> Result<core_types::ExternalFrostSignature, SignerError> {
        self.inner.aggregate_frost(request).await
    }

    async fn hmac_sha256(
        &self,
        message: Vec<u8>,
        path: String,
    ) -> Result<core_types::HashedMessageBytes, SignerError> {
        self.inner.hmac_sha256(message, path).await
    }
}

#[async_trait]
impl breez_sdk_spark::signer::ExternalSigner for WasmExternalSigner {
    fn identity_public_key(&self) -> Result<core_types::PublicKeyBytes, SignerError> {
        let wasm_pubkey: PublicKeyBytes = self
            .inner
            .identity_public_key()
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        Ok(wasm_pubkey.into())
    }

    async fn derive_public_key(
        &self,
        path: String,
    ) -> Result<core_types::PublicKeyBytes, SignerError> {
        //let wasm_pubkey: PublicKeyBytes = self
        let promise = self
            .inner
            .derive_public_key(path)
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        let future = JsFuture::from(promise);
        let result = future
            .await
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        let wasm_pubkey: PublicKeyBytes = serde_wasm_bindgen::from_value(result).map_err(|e| {
            SignerError::Generic(format!("Failed to deserialize public key: {}", e))
        })?;
        Ok(wasm_pubkey.into())
    }

    async fn sign_ecdsa(
        &self,
        message: core_types::MessageBytes,
        path: String,
    ) -> Result<core_types::EcdsaSignatureBytes, SignerError> {
        let wasm_msg: MessageBytes = message.into();
        let promise = self
            .inner
            .sign_ecdsa(wasm_msg, path)
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        let future = JsFuture::from(promise);
        let result = future
            .await
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        let wasm_sig: EcdsaSignatureBytes = serde_wasm_bindgen::from_value(result)
            .map_err(|e| SignerError::Generic(format!("Failed to deserialize signature: {}", e)))?;
        Ok(wasm_sig.into())
    }

    async fn sign_ecdsa_recoverable(
        &self,
        message: core_types::MessageBytes,
        path: String,
    ) -> Result<core_types::RecoverableEcdsaSignatureBytes, SignerError> {
        let wasm_msg: MessageBytes = message.into();
        let promise = self
            .inner
            .sign_ecdsa_recoverable(wasm_msg, path)
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        let future = JsFuture::from(promise);
        let result = future
            .await
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        let bytes: Vec<u8> = serde_wasm_bindgen::from_value(result).map_err(|e| {
            SignerError::Generic(format!(
                "Failed to deserialize recoverable signature: {}",
                e
            ))
        })?;
        Ok(core_types::RecoverableEcdsaSignatureBytes::new(bytes))
    }

    async fn encrypt_ecies(&self, message: Vec<u8>, path: String) -> Result<Vec<u8>, SignerError> {
        let promise = self
            .inner
            .encrypt_ecies(message, path)
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        let future = JsFuture::from(promise);
        let result = future
            .await
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        serde_wasm_bindgen::from_value(result).map_err(|e| {
            SignerError::Generic(format!("Failed to deserialize encrypted data: {}", e))
        })
    }

    async fn decrypt_ecies(&self, message: Vec<u8>, path: String) -> Result<Vec<u8>, SignerError> {
        let promise = self
            .inner
            .decrypt_ecies(message, path)
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        let future = JsFuture::from(promise);
        let result = future
            .await
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        serde_wasm_bindgen::from_value(result)
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))
    }

    async fn sign_hash_schnorr(
        &self,
        hash: Vec<u8>,
        path: String,
    ) -> Result<core_types::SchnorrSignatureBytes, SignerError> {
        let promise = self
            .inner
            .sign_hash_schnorr(hash, path)
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        let future = JsFuture::from(promise);
        let result = future
            .await
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        let wasm_sig: SchnorrSignatureBytes =
            serde_wasm_bindgen::from_value(result).map_err(|e| {
                SignerError::Generic(format!("Failed to deserialize schnorr signature: {}", e))
            })?;
        Ok(wasm_sig.into())
    }

    async fn generate_random_signing_commitment(
        &self,
    ) -> Result<core_types::ExternalFrostCommitments, SignerError> {
        let promise = self
            .inner
            .generate_random_signing_commitment()
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        let future = JsFuture::from(promise);
        let result = future
            .await
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        let wasm_commitments: ExternalFrostCommitments = serde_wasm_bindgen::from_value(result)
            .map_err(|e| {
                SignerError::Generic(format!("Failed to deserialize FROST commitments: {}", e))
            })?;
        Ok(wasm_commitments.into())
    }

    async fn get_public_key_for_node(
        &self,
        id: core_types::ExternalTreeNodeId,
    ) -> Result<core_types::PublicKeyBytes, SignerError> {
        let wasm_id: ExternalTreeNodeId = id.into();
        let promise = self
            .inner
            .get_public_key_for_node(wasm_id)
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        let future = JsFuture::from(promise);
        let result = future
            .await
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        let wasm_pubkey: PublicKeyBytes = serde_wasm_bindgen::from_value(result).map_err(|e| {
            SignerError::Generic(format!("Failed to deserialize public key: {}", e))
        })?;
        Ok(wasm_pubkey.into())
    }

    async fn generate_random_key(
        &self,
    ) -> Result<core_types::ExternalPrivateKeySource, SignerError> {
        let promise = self
            .inner
            .generate_random_key()
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        let future = JsFuture::from(promise);
        let result = future
            .await
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        let wasm_source: ExternalPrivateKeySource = serde_wasm_bindgen::from_value(result)
            .map_err(|e| {
                SignerError::Generic(format!("Failed to deserialize private key source: {}", e))
            })?;
        Ok(wasm_source.into())
    }

    async fn get_static_deposit_private_key_source(
        &self,
        index: u32,
    ) -> Result<core_types::ExternalPrivateKeySource, SignerError> {
        let promise = self
            .inner
            .get_static_deposit_private_key_source(index)
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        let future = JsFuture::from(promise);
        let result = future
            .await
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        let wasm_source: ExternalPrivateKeySource = serde_wasm_bindgen::from_value(result)
            .map_err(|e| {
                SignerError::Generic(format!("Failed to deserialize private key source: {}", e))
            })?;
        Ok(wasm_source.into())
    }

    async fn static_deposit_secret_key(
        &self,
        index: u32,
    ) -> Result<core_types::PrivateKeyBytes, SignerError> {
        let promise = self
            .inner
            .static_deposit_secret_key(index)
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        let future = JsFuture::from(promise);
        let result = future
            .await
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        let bytes: Vec<u8> = serde_wasm_bindgen::from_value(result).map_err(|e| {
            SignerError::Generic(format!("Failed to deserialize private key: {}", e))
        })?;
        Ok(core_types::PrivateKeyBytes { bytes })
    }

    async fn static_deposit_signing_key(
        &self,
        index: u32,
    ) -> Result<core_types::PublicKeyBytes, SignerError> {
        let promise = self
            .inner
            .static_deposit_signing_key(index)
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        let future = JsFuture::from(promise);
        let result = future
            .await
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        let wasm_pubkey: PublicKeyBytes = serde_wasm_bindgen::from_value(result).map_err(|e| {
            SignerError::Generic(format!("Failed to deserialize public key: {}", e))
        })?;
        Ok(wasm_pubkey.into())
    }

    async fn subtract_secret_keys(
        &self,
        signing_key: core_types::ExternalPrivateKeySource,
        new_signing_key: core_types::ExternalPrivateKeySource,
    ) -> Result<core_types::ExternalPrivateKeySource, SignerError> {
        let wasm_signing_key: ExternalPrivateKeySource = signing_key.into();
        let wasm_new_signing_key: ExternalPrivateKeySource = new_signing_key.into();
        let promise = self
            .inner
            .subtract_secret_keys(wasm_signing_key, wasm_new_signing_key)
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        let future = JsFuture::from(promise);
        let result = future
            .await
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        let wasm_result: ExternalPrivateKeySource = serde_wasm_bindgen::from_value(result)
            .map_err(|e| {
                SignerError::Generic(format!("Failed to deserialize private key source: {}", e))
            })?;
        Ok(wasm_result.into())
    }

    async fn split_secret_with_proofs(
        &self,
        secret: core_types::ExternalSecretToSplit,
        threshold: u32,
        num_shares: u32,
    ) -> Result<Vec<core_types::ExternalVerifiableSecretShare>, SignerError> {
        let wasm_secret: ExternalSecretToSplit = secret.into();
        let promise = self
            .inner
            .split_secret_with_proofs(wasm_secret, threshold, num_shares)
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        let future = JsFuture::from(promise);
        let result = future
            .await
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        let wasm_shares: Vec<ExternalVerifiableSecretShare> =
            serde_wasm_bindgen::from_value(result).map_err(|e| {
                SignerError::Generic(format!("Failed to deserialize secret shares: {}", e))
            })?;
        Ok(wasm_shares.into_iter().map(|s| s.into()).collect())
    }

    async fn encrypt_private_key_for_receiver(
        &self,
        private_key: core_types::ExternalEncryptedPrivateKey,
        receiver_public_key: core_types::PublicKeyBytes,
    ) -> Result<Vec<u8>, SignerError> {
        let wasm_private_key: ExternalEncryptedPrivateKey = private_key.into();
        let wasm_receiver_pubkey: PublicKeyBytes = receiver_public_key.into();
        let promise = self
            .inner
            .encrypt_private_key_for_receiver(wasm_private_key, wasm_receiver_pubkey)
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        let future = JsFuture::from(promise);
        let result = future
            .await
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        serde_wasm_bindgen::from_value(result).map_err(|e| {
            SignerError::Generic(format!("Failed to deserialize encrypted key: {}", e))
        })
    }

    async fn get_public_key_from_private_key_source(
        &self,
        private_key: core_types::ExternalPrivateKeySource,
    ) -> Result<core_types::PublicKeyBytes, SignerError> {
        let wasm_private_key: ExternalPrivateKeySource = private_key.into();
        let promise = self
            .inner
            .get_public_key_from_private_key_source(wasm_private_key)
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        let future = JsFuture::from(promise);
        let result = future
            .await
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        let wasm_pubkey: PublicKeyBytes = serde_wasm_bindgen::from_value(result).map_err(|e| {
            SignerError::Generic(format!("Failed to deserialize public key: {}", e))
        })?;
        Ok(wasm_pubkey.into())
    }

    async fn sign_frost(
        &self,
        request: core_types::ExternalSignFrostRequest,
    ) -> Result<core_types::ExternalFrostSignatureShare, SignerError> {
        let wasm_request: ExternalSignFrostRequest = request.into();
        let promise = self
            .inner
            .sign_frost(wasm_request)
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        let future = JsFuture::from(promise);
        let result = future
            .await
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        let wasm_share: ExternalFrostSignatureShare = serde_wasm_bindgen::from_value(result)
            .map_err(|e| {
                SignerError::Generic(format!(
                    "Failed to deserialize FROST signature share: {}",
                    e
                ))
            })?;
        Ok(wasm_share.into())
    }

    async fn aggregate_frost(
        &self,
        request: core_types::ExternalAggregateFrostRequest,
    ) -> Result<core_types::ExternalFrostSignature, SignerError> {
        let wasm_request: ExternalAggregateFrostRequest = request.into();
        let promise = self
            .inner
            .aggregate_frost(wasm_request)
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        let future = JsFuture::from(promise);
        let result = future
            .await
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        let wasm_sig: ExternalFrostSignature =
            serde_wasm_bindgen::from_value(result).map_err(|e| {
                SignerError::Generic(format!("Failed to deserialize FROST signature: {}", e))
            })?;
        Ok(wasm_sig.into())
    }

    async fn hmac_sha256(
        &self,
        message: Vec<u8>,
        path: String,
    ) -> Result<core_types::HashedMessageBytes, SignerError> {
        let promise = self
            .inner
            .hmac_sha256(message, path)
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        let future = JsFuture::from(promise);
        let result = future
            .await
            .map_err(|e| SignerError::Generic(format!("JS error: {e:?}")))?;
        let wasm_hash: HashedMessageBytes =
            serde_wasm_bindgen::from_value(result).map_err(|e| {
                SignerError::Generic(format!("Failed to deserialize HMAC-SHA256 hash: {}", e))
            })?;
        Ok(wasm_hash.into())
    }
}

#[wasm_bindgen(typescript_custom_section)]
const SIGNER_INTERFACE: &'static str = r#"export interface ExternalSigner {
    identityPublicKey(): PublicKeyBytes;
    derivePublicKey(path: string): Promise<PublicKeyBytes>;
    signEcdsa(message: MessageBytes, path: string): Promise<EcdsaSignatureBytes>;
    signEcdsaRecoverable(message: MessageBytes, path: string): Promise<RecoverableEcdsaSignatureBytes>;
    eciesEncrypt(message: Uint8Array, path: string): Promise<Uint8Array>;
    eciesDecrypt(message: Uint8Array, path: string): Promise<Uint8Array>;
    signHashSchnorr(hash: Uint8Array, path: string): Promise<SchnorrSignatureBytes>;
    generateFrostSigningCommitments(): Promise<ExternalFrostCommitments>;
    getPublicKeyForNode(id: ExternalTreeNodeId): Promise<PublicKeyBytes>;
    generateRandomKey(): Promise<ExternalPrivateKeySource>;
    getStaticDepositPrivateKeySource(index: number): Promise<ExternalPrivateKeySource>;
    getStaticDepositPrivateKey(index: number): Promise<PrivateKeyBytes>;
    getStaticDepositPublicKey(index: number): Promise<PublicKeyBytes>;
    subtractPrivateKeys(signingKey: ExternalPrivateKeySource, newSigningKey: ExternalPrivateKeySource): Promise<ExternalPrivateKeySource>;
    splitSecretWithProofs(secret: ExternalSecretToSplit, threshold: number, numShares: number): Promise<ExternalVerifiableSecretShare[]>;
    encryptPrivateKeyForReceiver(privateKey: ExternalEncryptedPrivateKey, receiverPublicKey: PublicKeyBytes): Promise<Uint8Array>;
    getPublicKeyFromPrivateKeySource(privateKey: ExternalPrivateKeySource): Promise<PublicKeyBytes>;
    signFrost(request: ExternalSignFrostRequest): Promise<ExternalFrostSignatureShare>;
    aggregateFrost(request: ExternalAggregateFrostRequest): Promise<ExternalFrostSignature>;
    hmacSha256(message: Uint8Array, path: string): Promise<HashedMessageBytes>;
}"#;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "ExternalSigner")]
    pub type JsExternalSigner;

    #[wasm_bindgen(structural, method, js_name = "identityPublicKey", catch)]
    pub fn identity_public_key(this: &JsExternalSigner) -> Result<PublicKeyBytes, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "derivePublicKey", catch)]
    pub fn derive_public_key(this: &JsExternalSigner, path: String) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "signEcdsa", catch)]
    pub fn sign_ecdsa(
        this: &JsExternalSigner,
        message: MessageBytes,
        path: String,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "signEcdsaRecoverable", catch)]
    pub fn sign_ecdsa_recoverable(
        this: &JsExternalSigner,
        message: MessageBytes,
        path: String,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "eciesEncrypt", catch)]
    pub fn encrypt_ecies(
        this: &JsExternalSigner,
        message: Vec<u8>,
        path: String,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "eciesDecrypt", catch)]
    pub fn decrypt_ecies(
        this: &JsExternalSigner,
        message: Vec<u8>,
        path: String,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "signHashSchnorr", catch)]
    pub fn sign_hash_schnorr(
        this: &JsExternalSigner,
        hash: Vec<u8>,
        path: String,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "generateFrostSigningCommitments", catch)]
    pub fn generate_random_signing_commitment(this: &JsExternalSigner) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "getPublicKeyForNode", catch)]
    pub fn get_public_key_for_node(
        this: &JsExternalSigner,
        id: ExternalTreeNodeId,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "generateRandomKey", catch)]
    pub fn generate_random_key(this: &JsExternalSigner) -> Result<Promise, JsValue>;

    #[wasm_bindgen(
        structural,
        method,
        js_name = "getStaticDepositPrivateKeySource",
        catch
    )]
    pub fn get_static_deposit_private_key_source(
        this: &JsExternalSigner,
        index: u32,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "getStaticDepositPrivateKey", catch)]
    pub fn static_deposit_secret_key(
        this: &JsExternalSigner,
        index: u32,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "getStaticDepositPublicKey", catch)]
    pub fn static_deposit_signing_key(
        this: &JsExternalSigner,
        index: u32,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "subtractPrivateKeys", catch)]
    pub fn subtract_secret_keys(
        this: &JsExternalSigner,
        signing_key: ExternalPrivateKeySource,
        new_signing_key: ExternalPrivateKeySource,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "splitSecretWithProofs", catch)]
    pub fn split_secret_with_proofs(
        this: &JsExternalSigner,
        secret: ExternalSecretToSplit,
        threshold: u32,
        num_shares: u32,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "encryptPrivateKeyForReceiver", catch)]
    pub fn encrypt_private_key_for_receiver(
        this: &JsExternalSigner,
        private_key: ExternalEncryptedPrivateKey,
        receiver_public_key: PublicKeyBytes,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(
        structural,
        method,
        js_name = "getPublicKeyFromPrivateKeySource",
        catch
    )]
    pub fn get_public_key_from_private_key_source(
        this: &JsExternalSigner,
        private_key: ExternalPrivateKeySource,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "signFrost", catch)]
    pub fn sign_frost(
        this: &JsExternalSigner,
        request: ExternalSignFrostRequest,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "aggregateFrost", catch)]
    pub fn aggregate_frost(
        this: &JsExternalSigner,
        request: ExternalAggregateFrostRequest,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "hmacSha256", catch)]
    pub fn hmac_sha256(
        this: &JsExternalSigner,
        message: Vec<u8>,
        path: String,
    ) -> Result<Promise, JsValue>;
}
