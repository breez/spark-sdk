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
    pub inner: ExternalSigner,
}

// This assumes that we'll always be running in a single thread (true for Wasm environments)
unsafe impl Send for WasmExternalSigner {}
unsafe impl Sync for WasmExternalSigner {}

/// Helper function to convert JS exceptions to String error
fn js_error_to_string(js_error: JsValue) -> String {
    if let Some(error_str) = js_error.as_string() {
        format!("JavaScript error: {}", error_str)
    } else if js_error.is_instance_of::<js_sys::Error>() {
        let error = js_sys::Error::from(js_error);
        format!("JavaScript error: {}", error.message())
    } else {
        "JavaScript signer operation failed".to_string()
    }
}

#[async_trait]
impl breez_sdk_spark::signer::ExternalSigner for WasmExternalSigner {
    fn identity_public_key(&self) -> core_types::PublicKeyBytes {
        let wasm_pubkey: PublicKeyBytes = self.inner.identity_public_key();
        wasm_pubkey.into()
    }

    fn derive_public_key(&self, path: String) -> Result<core_types::PublicKeyBytes, String> {
        let wasm_pubkey: PublicKeyBytes = self
            .inner
            .derive_public_key(path)
            .map_err(js_error_to_string)?;
        Ok(wasm_pubkey.into())
    }

    async fn sign_ecdsa(
        &self,
        message: Vec<u8>,
        path: String,
    ) -> Result<core_types::EcdsaSignatureBytes, String> {
        let promise = self
            .inner
            .sign_ecdsa(message, path)
            .map_err(js_error_to_string)?;
        let future = JsFuture::from(promise);
        let result = future.await.map_err(js_error_to_string)?;
        let wasm_sig: EcdsaSignatureBytes = serde_wasm_bindgen::from_value(result)
            .map_err(|e| format!("Failed to deserialize signature: {}", e))?;
        Ok(wasm_sig.into())
    }

    async fn sign_ecdsa_recoverable(
        &self,
        message: Vec<u8>,
        path: String,
    ) -> Result<Vec<u8>, String> {
        let promise = self
            .inner
            .sign_ecdsa_recoverable(message, path)
            .map_err(js_error_to_string)?;
        let future = JsFuture::from(promise);
        let result = future.await.map_err(js_error_to_string)?;
        serde_wasm_bindgen::from_value(result)
            .map_err(|e| format!("Failed to deserialize recoverable signature: {}", e))
    }

    async fn ecies_encrypt(&self, message: Vec<u8>, path: String) -> Result<Vec<u8>, String> {
        let promise = self
            .inner
            .ecies_encrypt(message, path)
            .map_err(js_error_to_string)?;
        let future = JsFuture::from(promise);
        let result = future.await.map_err(js_error_to_string)?;
        serde_wasm_bindgen::from_value(result)
            .map_err(|e| format!("Failed to deserialize encrypted data: {}", e))
    }

    async fn ecies_decrypt(&self, message: Vec<u8>, path: String) -> Result<Vec<u8>, String> {
        let promise = self
            .inner
            .ecies_decrypt(message, path)
            .map_err(js_error_to_string)?;
        let future = JsFuture::from(promise);
        let result = future.await.map_err(js_error_to_string)?;
        serde_wasm_bindgen::from_value(result)
            .map_err(|e| format!("Failed to deserialize decrypted data: {}", e))
    }

    async fn sign_hash_schnorr(
        &self,
        hash: Vec<u8>,
        path: String,
    ) -> Result<core_types::SchnorrSignatureBytes, String> {
        let promise = self
            .inner
            .sign_hash_schnorr(hash, path)
            .map_err(js_error_to_string)?;
        let future = JsFuture::from(promise);
        let result = future.await.map_err(js_error_to_string)?;
        let wasm_sig: SchnorrSignatureBytes = serde_wasm_bindgen::from_value(result)
            .map_err(|e| format!("Failed to deserialize schnorr signature: {}", e))?;
        Ok(wasm_sig.into())
    }

    async fn generate_frost_signing_commitments(
        &self,
    ) -> Result<core_types::ExternalFrostCommitments, String> {
        let promise = self
            .inner
            .generate_frost_signing_commitments()
            .map_err(js_error_to_string)?;
        let future = JsFuture::from(promise);
        let result = future.await.map_err(js_error_to_string)?;
        let wasm_commitments: ExternalFrostCommitments = serde_wasm_bindgen::from_value(result)
            .map_err(|e| format!("Failed to deserialize FROST commitments: {}", e))?;
        Ok(wasm_commitments.into())
    }

    async fn get_public_key_for_node(
        &self,
        id: core_types::ExternalTreeNodeId,
    ) -> Result<core_types::PublicKeyBytes, String> {
        let wasm_id: ExternalTreeNodeId = id.into();
        let promise = self
            .inner
            .get_public_key_for_node(wasm_id)
            .map_err(js_error_to_string)?;
        let future = JsFuture::from(promise);
        let result = future.await.map_err(js_error_to_string)?;
        let wasm_pubkey: PublicKeyBytes = serde_wasm_bindgen::from_value(result)
            .map_err(|e| format!("Failed to deserialize public key: {}", e))?;
        Ok(wasm_pubkey.into())
    }

    async fn generate_random_key(&self) -> Result<core_types::ExternalPrivateKeySource, String> {
        let promise = self
            .inner
            .generate_random_key()
            .map_err(js_error_to_string)?;
        let future = JsFuture::from(promise);
        let result = future.await.map_err(js_error_to_string)?;
        let wasm_source: ExternalPrivateKeySource = serde_wasm_bindgen::from_value(result)
            .map_err(|e| format!("Failed to deserialize private key source: {}", e))?;
        Ok(wasm_source.into())
    }

    async fn get_static_deposit_private_key_source(
        &self,
        index: u32,
    ) -> Result<core_types::ExternalPrivateKeySource, String> {
        let promise = self
            .inner
            .get_static_deposit_private_key_source(index)
            .map_err(js_error_to_string)?;
        let future = JsFuture::from(promise);
        let result = future.await.map_err(js_error_to_string)?;
        let wasm_source: ExternalPrivateKeySource = serde_wasm_bindgen::from_value(result)
            .map_err(|e| format!("Failed to deserialize private key source: {}", e))?;
        Ok(wasm_source.into())
    }

    async fn get_static_deposit_private_key(&self, index: u32) -> Result<Vec<u8>, String> {
        let promise = self
            .inner
            .get_static_deposit_private_key(index)
            .map_err(js_error_to_string)?;
        let future = JsFuture::from(promise);
        let result = future.await.map_err(js_error_to_string)?;
        serde_wasm_bindgen::from_value(result)
            .map_err(|e| format!("Failed to deserialize private key: {}", e))
    }

    async fn get_static_deposit_public_key(
        &self,
        index: u32,
    ) -> Result<core_types::PublicKeyBytes, String> {
        let promise = self
            .inner
            .get_static_deposit_public_key(index)
            .map_err(js_error_to_string)?;
        let future = JsFuture::from(promise);
        let result = future.await.map_err(js_error_to_string)?;
        let wasm_pubkey: PublicKeyBytes = serde_wasm_bindgen::from_value(result)
            .map_err(|e| format!("Failed to deserialize public key: {}", e))?;
        Ok(wasm_pubkey.into())
    }

    async fn subtract_private_keys(
        &self,
        signing_key: core_types::ExternalPrivateKeySource,
        new_signing_key: core_types::ExternalPrivateKeySource,
    ) -> Result<core_types::ExternalPrivateKeySource, String> {
        let wasm_signing_key: ExternalPrivateKeySource = signing_key.into();
        let wasm_new_signing_key: ExternalPrivateKeySource = new_signing_key.into();
        let promise = self
            .inner
            .subtract_private_keys(wasm_signing_key, wasm_new_signing_key)
            .map_err(js_error_to_string)?;
        let future = JsFuture::from(promise);
        let result = future.await.map_err(js_error_to_string)?;
        let wasm_result: ExternalPrivateKeySource = serde_wasm_bindgen::from_value(result)
            .map_err(|e| format!("Failed to deserialize private key source: {}", e))?;
        Ok(wasm_result.into())
    }

    async fn split_secret_with_proofs(
        &self,
        secret: core_types::ExternalSecretToSplit,
        threshold: u32,
        num_shares: u32,
    ) -> Result<Vec<core_types::ExternalVerifiableSecretShare>, String> {
        let wasm_secret: ExternalSecretToSplit = secret.into();
        let promise = self
            .inner
            .split_secret_with_proofs(wasm_secret, threshold, num_shares)
            .map_err(js_error_to_string)?;
        let future = JsFuture::from(promise);
        let result = future.await.map_err(js_error_to_string)?;
        let wasm_shares: Vec<ExternalVerifiableSecretShare> =
            serde_wasm_bindgen::from_value(result)
                .map_err(|e| format!("Failed to deserialize secret shares: {}", e))?;
        Ok(wasm_shares.into_iter().map(|s| s.into()).collect())
    }

    async fn encrypt_private_key_for_receiver(
        &self,
        private_key: core_types::ExternalEncryptedPrivateKey,
        receiver_public_key: core_types::PublicKeyBytes,
    ) -> Result<Vec<u8>, String> {
        let wasm_private_key: ExternalEncryptedPrivateKey = private_key.into();
        let wasm_receiver_pubkey: PublicKeyBytes = receiver_public_key.into();
        let promise = self
            .inner
            .encrypt_private_key_for_receiver(wasm_private_key, wasm_receiver_pubkey)
            .map_err(js_error_to_string)?;
        let future = JsFuture::from(promise);
        let result = future.await.map_err(js_error_to_string)?;
        serde_wasm_bindgen::from_value(result)
            .map_err(|e| format!("Failed to deserialize encrypted key: {}", e))
    }

    async fn get_public_key_from_private_key_source(
        &self,
        private_key: core_types::ExternalPrivateKeySource,
    ) -> Result<core_types::PublicKeyBytes, String> {
        let wasm_private_key: ExternalPrivateKeySource = private_key.into();
        let promise = self
            .inner
            .get_public_key_from_private_key_source(wasm_private_key)
            .map_err(js_error_to_string)?;
        let future = JsFuture::from(promise);
        let result = future.await.map_err(js_error_to_string)?;
        let wasm_pubkey: PublicKeyBytes = serde_wasm_bindgen::from_value(result)
            .map_err(|e| format!("Failed to deserialize public key: {}", e))?;
        Ok(wasm_pubkey.into())
    }

    async fn sign_frost(
        &self,
        request: core_types::ExternalSignFrostRequest,
    ) -> Result<core_types::ExternalFrostSignatureShare, String> {
        let wasm_request: ExternalSignFrostRequest = request.into();
        let promise = self
            .inner
            .sign_frost(wasm_request)
            .map_err(js_error_to_string)?;
        let future = JsFuture::from(promise);
        let result = future.await.map_err(js_error_to_string)?;
        let wasm_share: ExternalFrostSignatureShare = serde_wasm_bindgen::from_value(result)
            .map_err(|e| format!("Failed to deserialize FROST signature share: {}", e))?;
        Ok(wasm_share.into())
    }

    async fn aggregate_frost(
        &self,
        request: core_types::ExternalAggregateFrostRequest,
    ) -> Result<core_types::ExternalFrostSignature, String> {
        let wasm_request: ExternalAggregateFrostRequest = request.into();
        let promise = self
            .inner
            .aggregate_frost(wasm_request)
            .map_err(js_error_to_string)?;
        let future = JsFuture::from(promise);
        let result = future.await.map_err(js_error_to_string)?;
        let wasm_sig: ExternalFrostSignature = serde_wasm_bindgen::from_value(result)
            .map_err(|e| format!("Failed to deserialize FROST signature: {}", e))?;
        Ok(wasm_sig.into())
    }
}

#[wasm_bindgen(typescript_custom_section)]
const SIGNER_INTERFACE: &'static str = r#"export interface ExternalSigner {
    identityPublicKey(): PublicKeyBytes;
    derivePublicKey(path: string): PublicKeyBytes;
    signEcdsa(message: Uint8Array, path: string): Promise<EcdsaSignatureBytes>;
    signEcdsaRecoverable(message: Uint8Array, path: string): Promise<Uint8Array>;
    eciesEncrypt(message: Uint8Array, path: string): Promise<Uint8Array>;
    eciesDecrypt(message: Uint8Array, path: string): Promise<Uint8Array>;
    signHashSchnorr(hash: Uint8Array, path: string): Promise<SchnorrSignatureBytes>;
    generateFrostSigningCommitments(): Promise<ExternalFrostCommitments>;
    getPublicKeyForNode(id: ExternalTreeNodeId): Promise<PublicKeyBytes>;
    generateRandomKey(): Promise<ExternalPrivateKeySource>;
    getStaticDepositPrivateKeySource(index: number): Promise<ExternalPrivateKeySource>;
    getStaticDepositPrivateKey(index: number): Promise<Uint8Array>;
    getStaticDepositPublicKey(index: number): Promise<PublicKeyBytes>;
    subtractPrivateKeys(signingKey: ExternalPrivateKeySource, newSigningKey: ExternalPrivateKeySource): Promise<ExternalPrivateKeySource>;
    splitSecretWithProofs(secret: ExternalSecretToSplit, threshold: number, numShares: number): Promise<ExternalVerifiableSecretShare[]>;
    encryptPrivateKeyForReceiver(privateKey: ExternalEncryptedPrivateKey, receiverPublicKey: PublicKeyBytes): Promise<Uint8Array>;
    getPublicKeyFromPrivateKeySource(privateKey: ExternalPrivateKeySource): Promise<PublicKeyBytes>;
    signFrost(request: ExternalSignFrostRequest): Promise<ExternalFrostSignatureShare>;
    aggregateFrost(request: ExternalAggregateFrostRequest): Promise<ExternalFrostSignature>;
}"#;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "ExternalSigner")]
    pub type ExternalSigner;

    #[wasm_bindgen(structural, method, js_name = "identityPublicKey")]
    pub fn identity_public_key(this: &ExternalSigner) -> PublicKeyBytes;

    #[wasm_bindgen(structural, method, js_name = "derivePublicKey", catch)]
    pub fn derive_public_key(
        this: &ExternalSigner,
        path: String,
    ) -> Result<PublicKeyBytes, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "signEcdsa", catch)]
    pub fn sign_ecdsa(
        this: &ExternalSigner,
        message: Vec<u8>,
        path: String,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "signEcdsaRecoverable", catch)]
    pub fn sign_ecdsa_recoverable(
        this: &ExternalSigner,
        message: Vec<u8>,
        path: String,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "eciesEncrypt", catch)]
    pub fn ecies_encrypt(
        this: &ExternalSigner,
        message: Vec<u8>,
        path: String,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "eciesDecrypt", catch)]
    pub fn ecies_decrypt(
        this: &ExternalSigner,
        message: Vec<u8>,
        path: String,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "signHashSchnorr", catch)]
    pub fn sign_hash_schnorr(
        this: &ExternalSigner,
        hash: Vec<u8>,
        path: String,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "generateFrostSigningCommitments", catch)]
    pub fn generate_frost_signing_commitments(this: &ExternalSigner) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "getPublicKeyForNode", catch)]
    pub fn get_public_key_for_node(
        this: &ExternalSigner,
        id: ExternalTreeNodeId,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "generateRandomKey", catch)]
    pub fn generate_random_key(this: &ExternalSigner) -> Result<Promise, JsValue>;

    #[wasm_bindgen(
        structural,
        method,
        js_name = "getStaticDepositPrivateKeySource",
        catch
    )]
    pub fn get_static_deposit_private_key_source(
        this: &ExternalSigner,
        index: u32,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "getStaticDepositPrivateKey", catch)]
    pub fn get_static_deposit_private_key(
        this: &ExternalSigner,
        index: u32,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "getStaticDepositPublicKey", catch)]
    pub fn get_static_deposit_public_key(
        this: &ExternalSigner,
        index: u32,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "subtractPrivateKeys", catch)]
    pub fn subtract_private_keys(
        this: &ExternalSigner,
        signing_key: ExternalPrivateKeySource,
        new_signing_key: ExternalPrivateKeySource,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "splitSecretWithProofs", catch)]
    pub fn split_secret_with_proofs(
        this: &ExternalSigner,
        secret: ExternalSecretToSplit,
        threshold: u32,
        num_shares: u32,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "encryptPrivateKeyForReceiver", catch)]
    pub fn encrypt_private_key_for_receiver(
        this: &ExternalSigner,
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
        this: &ExternalSigner,
        private_key: ExternalPrivateKeySource,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "signFrost", catch)]
    pub fn sign_frost(
        this: &ExternalSigner,
        request: ExternalSignFrostRequest,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "aggregateFrost", catch)]
    pub fn aggregate_frost(
        this: &ExternalSigner,
        request: ExternalAggregateFrostRequest,
    ) -> Result<Promise, JsValue>;
}
