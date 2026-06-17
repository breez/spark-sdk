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

#[macros::extern_wasm_bindgen(breez_sdk_spark::signer::external_types::SecretBytes)]
pub struct SecretBytes {
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

#[macros::extern_wasm_bindgen(breez_sdk_spark::signer::external_types::ExternalFrostSignatureShare)]
pub struct ExternalFrostSignatureShare {
    pub bytes: Vec<u8>,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::signer::external_types::ExternalFrostSignature)]
pub struct ExternalFrostSignature {
    pub bytes: Vec<u8>,
}

pub struct WasmExternalBreezSigner {
    pub inner: JsExternalBreezSigner,
}

// This assumes that we'll always be running in a single thread (true for Wasm environments)
unsafe impl Send for WasmExternalBreezSigner {}
unsafe impl Sync for WasmExternalBreezSigner {}

impl WasmExternalBreezSigner {
    pub fn new(inner: JsExternalBreezSigner) -> Self {
        Self { inner }
    }
}

/// A Rust-backed [`ExternalBreezSigner`] surfaced to JS as a signer object that
/// can be passed to `connectWithSigner` or `SdkBuilder.newWithSigner`. Produced
/// by `defaultExternalSigners` (seed) and `createTurnkeySigner` (Turnkey).
///
/// [`ExternalBreezSigner`]: breez_sdk_spark::signer::ExternalBreezSigner
#[wasm_bindgen]
#[derive(Clone)]
pub struct ExternalBreezSignerHandle {
    pub(crate) inner: std::sync::Arc<dyn breez_sdk_spark::signer::ExternalBreezSigner>,
}

// This assumes that we'll always be running in a single thread (true for Wasm environments)
unsafe impl Send for ExternalBreezSignerHandle {}
unsafe impl Sync for ExternalBreezSignerHandle {}

impl ExternalBreezSignerHandle {
    pub fn new(inner: std::sync::Arc<dyn breez_sdk_spark::signer::ExternalBreezSigner>) -> Self {
        Self { inner }
    }
}

#[wasm_bindgen]
impl ExternalBreezSignerHandle {
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

    #[wasm_bindgen(js_name = "encryptEcies")]
    pub async fn encrypt_ecies(&self, message: Vec<u8>, path: String) -> Result<Vec<u8>, JsValue> {
        self.inner
            .encrypt_ecies(message, path)
            .await
            .map_err(|e| JsValue::from_str(&format!("{e:?}")))
    }

    #[wasm_bindgen(js_name = "decryptEcies")]
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
impl breez_sdk_spark::signer::ExternalBreezSigner for ExternalBreezSignerHandle {
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

    async fn hmac_sha256(
        &self,
        message: Vec<u8>,
        path: String,
    ) -> Result<core_types::HashedMessageBytes, SignerError> {
        self.inner.hmac_sha256(message, path).await
    }
}

#[async_trait]
impl breez_sdk_spark::signer::ExternalBreezSigner for WasmExternalBreezSigner {
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
const SIGNER_INTERFACE: &'static str = r#"export interface ExternalBreezSigner {
    derivePublicKey(path: string): Promise<PublicKeyBytes>;
    signEcdsa(message: MessageBytes, path: string): Promise<EcdsaSignatureBytes>;
    signEcdsaRecoverable(message: MessageBytes, path: string): Promise<RecoverableEcdsaSignatureBytes>;
    encryptEcies(message: Uint8Array, path: string): Promise<Uint8Array>;
    decryptEcies(message: Uint8Array, path: string): Promise<Uint8Array>;
    signHashSchnorr(hash: Uint8Array, path: string): Promise<SchnorrSignatureBytes>;
    hmacSha256(message: Uint8Array, path: string): Promise<HashedMessageBytes>;
}"#;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "ExternalBreezSigner")]
    pub type JsExternalBreezSigner;

    #[wasm_bindgen(structural, method, js_name = "derivePublicKey", catch)]
    pub fn derive_public_key(
        this: &JsExternalBreezSigner,
        path: String,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "signEcdsa", catch)]
    pub fn sign_ecdsa(
        this: &JsExternalBreezSigner,
        message: MessageBytes,
        path: String,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "signEcdsaRecoverable", catch)]
    pub fn sign_ecdsa_recoverable(
        this: &JsExternalBreezSigner,
        message: MessageBytes,
        path: String,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "encryptEcies", catch)]
    pub fn encrypt_ecies(
        this: &JsExternalBreezSigner,
        message: Vec<u8>,
        path: String,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "decryptEcies", catch)]
    pub fn decrypt_ecies(
        this: &JsExternalBreezSigner,
        message: Vec<u8>,
        path: String,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "signHashSchnorr", catch)]
    pub fn sign_hash_schnorr(
        this: &JsExternalBreezSigner,
        hash: Vec<u8>,
        path: String,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "hmacSha256", catch)]
    pub fn hmac_sha256(
        this: &JsExternalBreezSigner,
        message: Vec<u8>,
        path: String,
    ) -> Result<Promise, JsValue>;
}

// ───────────────────── High-level Spark signer types ─────────────────────

#[macros::extern_wasm_bindgen(
    breez_sdk_spark::signer::external_spark_types::ExternalFrostDerivation
)]
pub enum ExternalFrostDerivation {
    SigningLeaf { leaf_id: ExternalTreeNodeId },
    StaticDeposit { index: u32 },
    HtlcPreimage,
    Identity,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::signer::external_spark_types::ExternalFrostJob)]
pub struct ExternalFrostJob {
    pub derivation: ExternalFrostDerivation,
    pub sighash: Vec<u8>,
    pub verifying_key: Vec<u8>,
    pub operator_commitments: Vec<IdentifierCommitmentPair>,
    pub adaptor_public_key: Option<Vec<u8>>,
}

#[macros::extern_wasm_bindgen(
    breez_sdk_spark::signer::external_spark_types::ExternalFrostShareResult
)]
pub struct ExternalFrostShareResult {
    pub commitment: ExternalFrostCommitments,
    pub signature_share: ExternalFrostSignatureShare,
}

#[macros::extern_wasm_bindgen(
    breez_sdk_spark::signer::external_spark_types::ExternalOperatorRecipient
)]
pub struct ExternalOperatorRecipient {
    pub id: u64,
    pub identifier: ExternalIdentifier,
    pub public_key: Vec<u8>,
}

#[macros::extern_wasm_bindgen(
    breez_sdk_spark::signer::external_spark_types::ExternalOperatorPackage
)]
pub struct ExternalOperatorPackage {
    pub operator_identifier: ExternalIdentifier,
    pub encrypted_package: Vec<u8>,
}

#[macros::extern_wasm_bindgen(
    breez_sdk_spark::signer::external_spark_types::ExternalTransferLeafInput
)]
pub struct ExternalTransferLeafInput {
    pub node_id: ExternalTreeNodeId,
    pub new_leaf_id: ExternalTreeNodeId,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::signer::external_spark_types::ExternalNewLeafKey)]
pub struct ExternalNewLeafKey {
    pub node_id: ExternalTreeNodeId,
    pub new_signing_public_key: Vec<u8>,
}

#[macros::extern_wasm_bindgen(
    breez_sdk_spark::signer::external_spark_types::ExternalPrepareTransferRequest
)]
pub struct ExternalPrepareTransferRequest {
    pub transfer_id: String,
    pub receiver_public_key: Vec<u8>,
    pub leaves: Vec<ExternalTransferLeafInput>,
    pub operator_recipients: Vec<ExternalOperatorRecipient>,
    pub threshold: u32,
}

#[macros::extern_wasm_bindgen(
    breez_sdk_spark::signer::external_spark_types::ExternalPreparedTransfer
)]
pub struct ExternalPreparedTransfer {
    pub operator_packages: Vec<ExternalOperatorPackage>,
    pub new_leaf_keys: Vec<ExternalNewLeafKey>,
    pub transfer_user_signature: EcdsaSignatureBytes,
}

#[macros::extern_wasm_bindgen(
    breez_sdk_spark::signer::external_spark_types::ExternalClaimLeafInput
)]
pub struct ExternalClaimLeafInput {
    pub node_id: ExternalTreeNodeId,
    pub sender_signature: Vec<u8>,
    pub leaf_key_ciphertext: Vec<u8>,
}

#[macros::extern_wasm_bindgen(
    breez_sdk_spark::signer::external_spark_types::ExternalPrepareClaimRequest
)]
pub struct ExternalPrepareClaimRequest {
    pub transfer_id: String,
    pub sender_identity_public_key: Vec<u8>,
    pub leaves: Vec<ExternalClaimLeafInput>,
    pub operator_recipients: Vec<ExternalOperatorRecipient>,
    pub threshold: u32,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::signer::external_spark_types::ExternalPreparedClaim)]
pub struct ExternalPreparedClaim {
    pub operator_packages: Vec<ExternalOperatorPackage>,
}

#[macros::extern_wasm_bindgen(
    breez_sdk_spark::signer::external_spark_types::ExternalPrepareLightningReceiveRequest
)]
pub struct ExternalPrepareLightningReceiveRequest {
    pub operator_recipients: Vec<ExternalOperatorRecipient>,
    pub threshold: u32,
}

#[macros::extern_wasm_bindgen(
    breez_sdk_spark::signer::external_spark_types::ExternalPreparedLightningReceive
)]
pub struct ExternalPreparedLightningReceive {
    pub payment_hash: Vec<u8>,
    pub operator_preimage_packages: Vec<ExternalOperatorPackage>,
}

#[macros::extern_wasm_bindgen(
    breez_sdk_spark::signer::external_spark_types::ExternalPrepareStaticDepositRequest
)]
pub struct ExternalPrepareStaticDepositRequest {
    pub index: u32,
    pub ssp_public_key: Vec<u8>,
    pub frost_jobs: Vec<ExternalFrostJob>,
}

#[macros::extern_wasm_bindgen(
    breez_sdk_spark::signer::external_spark_types::ExternalPreparedStaticDeposit
)]
pub struct ExternalPreparedStaticDeposit {
    pub exported_secret: Vec<u8>,
    pub frost_shares: Vec<ExternalFrostShareResult>,
}

#[macros::extern_wasm_bindgen(
    breez_sdk_spark::signer::external_spark_types::ExternalStartStaticDepositRefundRequest
)]
pub struct ExternalStartStaticDepositRefundRequest {
    pub index: u32,
    pub user_statement: Vec<u8>,
}

#[macros::extern_wasm_bindgen(
    breez_sdk_spark::signer::external_spark_types::ExternalStartedStaticDepositRefund
)]
pub struct ExternalStartedStaticDepositRefund {
    pub signing_public_key: Vec<u8>,
    pub nonce_commitment: ExternalFrostCommitments,
    pub user_signature: EcdsaSignatureBytes,
}

#[macros::extern_wasm_bindgen(
    breez_sdk_spark::signer::external_spark_types::ExternalSignStaticDepositRefundRequest
)]
pub struct ExternalSignStaticDepositRefundRequest {
    pub index: u32,
    pub sighash: Vec<u8>,
    pub verifying_key: Vec<u8>,
    pub nonce_commitment: ExternalFrostCommitments,
    pub statechain_commitments: Vec<IdentifierCommitmentPair>,
    pub statechain_signatures: Vec<IdentifierSignaturePair>,
    pub statechain_public_keys: Vec<IdentifierPublicKeyPair>,
}

#[macros::extern_wasm_bindgen(
    breez_sdk_spark::signer::external_spark_types::ExternalSparkInvoiceKind
)]
pub enum ExternalSparkInvoiceKind {
    Sats,
    Tokens,
}

#[macros::extern_wasm_bindgen(
    breez_sdk_spark::signer::external_spark_types::ExternalSignSparkInvoiceRequest
)]
pub struct ExternalSignSparkInvoiceRequest {
    pub kind: ExternalSparkInvoiceKind,
    pub invoice_hash: Vec<u8>,
}

#[macros::extern_wasm_bindgen(
    breez_sdk_spark::signer::external_spark_types::ExternalSignedSparkInvoice
)]
pub struct ExternalSignedSparkInvoice {
    pub signature: SchnorrSignatureBytes,
}

#[macros::extern_wasm_bindgen(
    breez_sdk_spark::signer::external_spark_types::ExternalTokenTransactionKind
)]
pub enum ExternalTokenTransactionKind {
    Freeze,
    Partial,
    Final,
}

#[macros::extern_wasm_bindgen(
    breez_sdk_spark::signer::external_spark_types::ExternalPrepareTokenTransactionRequest
)]
pub struct ExternalPrepareTokenTransactionRequest {
    pub kind: ExternalTokenTransactionKind,
    pub digest: Vec<u8>,
}

#[macros::extern_wasm_bindgen(
    breez_sdk_spark::signer::external_spark_types::ExternalPreparedTokenTransaction
)]
pub struct ExternalPreparedTokenTransaction {
    pub signature: SchnorrSignatureBytes,
}

#[macros::extern_wasm_bindgen(
    breez_sdk_spark::signer::external_spark_types::ExternalPrepareStaticDepositClaimRequest
)]
pub struct ExternalPrepareStaticDepositClaimRequest {
    pub index: u32,
    pub user_statement: Vec<u8>,
}

#[macros::extern_wasm_bindgen(
    breez_sdk_spark::signer::external_spark_types::ExternalPreparedStaticDepositClaim
)]
pub struct ExternalPreparedStaticDepositClaim {
    pub deposit_secret_key: SecretBytes,
    pub user_signature: EcdsaSignatureBytes,
}

use breez_sdk_spark::signer::external_spark_types as core_spark;

/// Wraps a JS object implementing the `ExternalSparkSigner` interface and
/// implements the core `ExternalSparkSigner` trait over it.
pub struct WasmExternalSparkSigner {
    pub inner: JsExternalSparkSigner,
}

// Single-threaded Wasm environment.
unsafe impl Send for WasmExternalSparkSigner {}
unsafe impl Sync for WasmExternalSparkSigner {}

impl WasmExternalSparkSigner {
    pub fn new(inner: JsExternalSparkSigner) -> Self {
        Self { inner }
    }
}

fn spark_js_err(e: impl std::fmt::Debug) -> SignerError {
    SignerError::Generic(format!("JS error: {e:?}"))
}

fn spark_de_err(e: impl std::fmt::Display) -> SignerError {
    SignerError::Generic(format!("Failed to deserialize signer response: {e}"))
}

#[async_trait]
impl breez_sdk_spark::signer::ExternalSparkSigner for WasmExternalSparkSigner {
    async fn get_identity_public_key(&self) -> Result<core_types::PublicKeyBytes, SignerError> {
        let promise = self.inner.get_identity_public_key().map_err(spark_js_err)?;
        let result = JsFuture::from(promise).await.map_err(spark_js_err)?;
        let v: PublicKeyBytes = serde_wasm_bindgen::from_value(result).map_err(spark_de_err)?;
        Ok(v.into())
    }

    async fn get_public_key_for_leaf(
        &self,
        leaf_id: core_types::ExternalTreeNodeId,
    ) -> Result<core_types::PublicKeyBytes, SignerError> {
        let wasm_leaf: ExternalTreeNodeId = leaf_id.into();
        let promise = self
            .inner
            .get_public_key_for_leaf(wasm_leaf)
            .map_err(spark_js_err)?;
        let result = JsFuture::from(promise).await.map_err(spark_js_err)?;
        let v: PublicKeyBytes = serde_wasm_bindgen::from_value(result).map_err(spark_de_err)?;
        Ok(v.into())
    }

    async fn get_static_deposit_public_key(
        &self,
        index: u32,
    ) -> Result<core_types::PublicKeyBytes, SignerError> {
        let promise = self
            .inner
            .get_static_deposit_public_key(index)
            .map_err(spark_js_err)?;
        let result = JsFuture::from(promise).await.map_err(spark_js_err)?;
        let v: PublicKeyBytes = serde_wasm_bindgen::from_value(result).map_err(spark_de_err)?;
        Ok(v.into())
    }

    async fn sign_authentication_challenge(
        &self,
        challenge: Vec<u8>,
    ) -> Result<core_types::EcdsaSignatureBytes, SignerError> {
        let promise = self
            .inner
            .sign_authentication_challenge(challenge)
            .map_err(spark_js_err)?;
        let result = JsFuture::from(promise).await.map_err(spark_js_err)?;
        let v: EcdsaSignatureBytes =
            serde_wasm_bindgen::from_value(result).map_err(spark_de_err)?;
        Ok(v.into())
    }

    async fn sign_message(
        &self,
        message: Vec<u8>,
    ) -> Result<core_types::EcdsaSignatureBytes, SignerError> {
        let promise = self.inner.sign_message(message).map_err(spark_js_err)?;
        let result = JsFuture::from(promise).await.map_err(spark_js_err)?;
        let v: EcdsaSignatureBytes =
            serde_wasm_bindgen::from_value(result).map_err(spark_de_err)?;
        Ok(v.into())
    }

    async fn sign_frost(
        &self,
        jobs: Vec<core_spark::ExternalFrostJob>,
    ) -> Result<Vec<core_spark::ExternalFrostShareResult>, SignerError> {
        let wasm_jobs: Vec<ExternalFrostJob> = jobs.into_iter().map(Into::into).collect();
        let jobs_value = serde_wasm_bindgen::to_value(&wasm_jobs).map_err(spark_de_err)?;
        let promise = self.inner.sign_frost(jobs_value).map_err(spark_js_err)?;
        let result = JsFuture::from(promise).await.map_err(spark_js_err)?;
        let v: Vec<ExternalFrostShareResult> =
            serde_wasm_bindgen::from_value(result).map_err(spark_de_err)?;
        Ok(v.into_iter().map(Into::into).collect())
    }

    async fn prepare_transfer(
        &self,
        request: core_spark::ExternalPrepareTransferRequest,
    ) -> Result<core_spark::ExternalPreparedTransfer, SignerError> {
        let req: ExternalPrepareTransferRequest = request.into();
        let promise = self.inner.prepare_transfer(req).map_err(spark_js_err)?;
        let result = JsFuture::from(promise).await.map_err(spark_js_err)?;
        let v: ExternalPreparedTransfer =
            serde_wasm_bindgen::from_value(result).map_err(spark_de_err)?;
        Ok(v.into())
    }

    async fn prepare_claim(
        &self,
        request: core_spark::ExternalPrepareClaimRequest,
    ) -> Result<core_spark::ExternalPreparedClaim, SignerError> {
        let req: ExternalPrepareClaimRequest = request.into();
        let promise = self.inner.prepare_claim(req).map_err(spark_js_err)?;
        let result = JsFuture::from(promise).await.map_err(spark_js_err)?;
        let v: ExternalPreparedClaim =
            serde_wasm_bindgen::from_value(result).map_err(spark_de_err)?;
        Ok(v.into())
    }

    async fn prepare_lightning_receive(
        &self,
        request: core_spark::ExternalPrepareLightningReceiveRequest,
    ) -> Result<core_spark::ExternalPreparedLightningReceive, SignerError> {
        let req: ExternalPrepareLightningReceiveRequest = request.into();
        let promise = self
            .inner
            .prepare_lightning_receive(req)
            .map_err(spark_js_err)?;
        let result = JsFuture::from(promise).await.map_err(spark_js_err)?;
        let v: ExternalPreparedLightningReceive =
            serde_wasm_bindgen::from_value(result).map_err(spark_de_err)?;
        Ok(v.into())
    }

    async fn prepare_static_deposit(
        &self,
        request: core_spark::ExternalPrepareStaticDepositRequest,
    ) -> Result<core_spark::ExternalPreparedStaticDeposit, SignerError> {
        let req: ExternalPrepareStaticDepositRequest = request.into();
        let promise = self
            .inner
            .prepare_static_deposit(req)
            .map_err(spark_js_err)?;
        let result = JsFuture::from(promise).await.map_err(spark_js_err)?;
        let v: ExternalPreparedStaticDeposit =
            serde_wasm_bindgen::from_value(result).map_err(spark_de_err)?;
        Ok(v.into())
    }

    async fn start_static_deposit_refund(
        &self,
        request: core_spark::ExternalStartStaticDepositRefundRequest,
    ) -> Result<core_spark::ExternalStartedStaticDepositRefund, SignerError> {
        let req: ExternalStartStaticDepositRefundRequest = request.into();
        let promise = self
            .inner
            .start_static_deposit_refund(req)
            .map_err(spark_js_err)?;
        let result = JsFuture::from(promise).await.map_err(spark_js_err)?;
        let v: ExternalStartedStaticDepositRefund =
            serde_wasm_bindgen::from_value(result).map_err(spark_de_err)?;
        Ok(v.into())
    }

    async fn sign_static_deposit_refund(
        &self,
        request: core_spark::ExternalSignStaticDepositRefundRequest,
    ) -> Result<core_types::ExternalFrostSignature, SignerError> {
        let req: ExternalSignStaticDepositRefundRequest = request.into();
        let promise = self
            .inner
            .sign_static_deposit_refund(req)
            .map_err(spark_js_err)?;
        let result = JsFuture::from(promise).await.map_err(spark_js_err)?;
        let v: ExternalFrostSignature =
            serde_wasm_bindgen::from_value(result).map_err(spark_de_err)?;
        Ok(v.into())
    }

    async fn sign_spark_invoice(
        &self,
        request: core_spark::ExternalSignSparkInvoiceRequest,
    ) -> Result<core_spark::ExternalSignedSparkInvoice, SignerError> {
        let req: ExternalSignSparkInvoiceRequest = request.into();
        let promise = self.inner.sign_spark_invoice(req).map_err(spark_js_err)?;
        let result = JsFuture::from(promise).await.map_err(spark_js_err)?;
        let v: ExternalSignedSparkInvoice =
            serde_wasm_bindgen::from_value(result).map_err(spark_de_err)?;
        Ok(v.into())
    }

    async fn prepare_token_transaction(
        &self,
        request: core_spark::ExternalPrepareTokenTransactionRequest,
    ) -> Result<core_spark::ExternalPreparedTokenTransaction, SignerError> {
        let req: ExternalPrepareTokenTransactionRequest = request.into();
        let promise = self
            .inner
            .prepare_token_transaction(req)
            .map_err(spark_js_err)?;
        let result = JsFuture::from(promise).await.map_err(spark_js_err)?;
        let v: ExternalPreparedTokenTransaction =
            serde_wasm_bindgen::from_value(result).map_err(spark_de_err)?;
        Ok(v.into())
    }

    async fn prepare_static_deposit_claim(
        &self,
        request: core_spark::ExternalPrepareStaticDepositClaimRequest,
    ) -> Result<core_spark::ExternalPreparedStaticDepositClaim, SignerError> {
        let req: ExternalPrepareStaticDepositClaimRequest = request.into();
        let promise = self
            .inner
            .prepare_static_deposit_claim(req)
            .map_err(spark_js_err)?;
        let result = JsFuture::from(promise).await.map_err(spark_js_err)?;
        let v: ExternalPreparedStaticDepositClaim =
            serde_wasm_bindgen::from_value(result).map_err(spark_de_err)?;
        Ok(v.into())
    }
}

#[wasm_bindgen(typescript_custom_section)]
const SPARK_SIGNER_INTERFACE: &'static str = r#"export interface ExternalSparkSigner {
    getIdentityPublicKey(): Promise<PublicKeyBytes>;
    getPublicKeyForLeaf(leafId: ExternalTreeNodeId): Promise<PublicKeyBytes>;
    getStaticDepositPublicKey(index: number): Promise<PublicKeyBytes>;
    signAuthenticationChallenge(challenge: Uint8Array): Promise<EcdsaSignatureBytes>;
    signMessage(message: Uint8Array): Promise<EcdsaSignatureBytes>;
    signFrost(jobs: ExternalFrostJob[]): Promise<ExternalFrostShareResult[]>;
    prepareTransfer(request: ExternalPrepareTransferRequest): Promise<ExternalPreparedTransfer>;
    prepareClaim(request: ExternalPrepareClaimRequest): Promise<ExternalPreparedClaim>;
    prepareLightningReceive(request: ExternalPrepareLightningReceiveRequest): Promise<ExternalPreparedLightningReceive>;
    prepareStaticDeposit(request: ExternalPrepareStaticDepositRequest): Promise<ExternalPreparedStaticDeposit>;
    startStaticDepositRefund(request: ExternalStartStaticDepositRefundRequest): Promise<ExternalStartedStaticDepositRefund>;
    signStaticDepositRefund(request: ExternalSignStaticDepositRefundRequest): Promise<ExternalFrostSignature>;
    signSparkInvoice(request: ExternalSignSparkInvoiceRequest): Promise<ExternalSignedSparkInvoice>;
    prepareTokenTransaction(request: ExternalPrepareTokenTransactionRequest): Promise<ExternalPreparedTokenTransaction>;
    prepareStaticDepositClaim(request: ExternalPrepareStaticDepositClaimRequest): Promise<ExternalPreparedStaticDepositClaim>;
}"#;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "ExternalSparkSigner")]
    pub type JsExternalSparkSigner;

    #[wasm_bindgen(structural, method, js_name = "getIdentityPublicKey", catch)]
    pub fn get_identity_public_key(this: &JsExternalSparkSigner) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "getPublicKeyForLeaf", catch)]
    pub fn get_public_key_for_leaf(
        this: &JsExternalSparkSigner,
        leaf_id: ExternalTreeNodeId,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "getStaticDepositPublicKey", catch)]
    pub fn get_static_deposit_public_key(
        this: &JsExternalSparkSigner,
        index: u32,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "signAuthenticationChallenge", catch)]
    pub fn sign_authentication_challenge(
        this: &JsExternalSparkSigner,
        challenge: Vec<u8>,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "signMessage", catch)]
    pub fn sign_message(this: &JsExternalSparkSigner, message: Vec<u8>)
    -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "signFrost", catch)]
    pub fn sign_frost(this: &JsExternalSparkSigner, jobs: JsValue) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "prepareTransfer", catch)]
    pub fn prepare_transfer(
        this: &JsExternalSparkSigner,
        request: ExternalPrepareTransferRequest,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "prepareClaim", catch)]
    pub fn prepare_claim(
        this: &JsExternalSparkSigner,
        request: ExternalPrepareClaimRequest,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "prepareLightningReceive", catch)]
    pub fn prepare_lightning_receive(
        this: &JsExternalSparkSigner,
        request: ExternalPrepareLightningReceiveRequest,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "prepareStaticDeposit", catch)]
    pub fn prepare_static_deposit(
        this: &JsExternalSparkSigner,
        request: ExternalPrepareStaticDepositRequest,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "startStaticDepositRefund", catch)]
    pub fn start_static_deposit_refund(
        this: &JsExternalSparkSigner,
        request: ExternalStartStaticDepositRefundRequest,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "signStaticDepositRefund", catch)]
    pub fn sign_static_deposit_refund(
        this: &JsExternalSparkSigner,
        request: ExternalSignStaticDepositRefundRequest,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "signSparkInvoice", catch)]
    pub fn sign_spark_invoice(
        this: &JsExternalSparkSigner,
        request: ExternalSignSparkInvoiceRequest,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "prepareTokenTransaction", catch)]
    pub fn prepare_token_transaction(
        this: &JsExternalSparkSigner,
        request: ExternalPrepareTokenTransactionRequest,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "prepareStaticDepositClaim", catch)]
    pub fn prepare_static_deposit_claim(
        this: &JsExternalSparkSigner,
        request: ExternalPrepareStaticDepositClaimRequest,
    ) -> Result<Promise, JsValue>;
}

/// A Rust-backed [`ExternalSparkSigner`] surfaced to JS as a signer object that
/// can be passed to `connectWithSigner` or `SdkBuilder.newWithSigner`. Produced
/// by `defaultExternalSigners` (seed) and `createTurnkeySigner` (Turnkey).
///
/// [`ExternalSparkSigner`]: breez_sdk_spark::signer::ExternalSparkSigner
#[wasm_bindgen]
#[derive(Clone)]
pub struct ExternalSparkSignerHandle {
    pub(crate) inner: std::sync::Arc<dyn breez_sdk_spark::signer::ExternalSparkSigner>,
}

// This assumes that we'll always be running in a single thread (true for Wasm environments)
unsafe impl Send for ExternalSparkSignerHandle {}
unsafe impl Sync for ExternalSparkSignerHandle {}

impl ExternalSparkSignerHandle {
    pub fn new(inner: std::sync::Arc<dyn breez_sdk_spark::signer::ExternalSparkSigner>) -> Self {
        Self { inner }
    }
}

fn spark_handle_js_err(e: impl std::fmt::Debug) -> JsValue {
    JsValue::from_str(&format!("{e:?}"))
}

#[wasm_bindgen]
impl ExternalSparkSignerHandle {
    #[wasm_bindgen(js_name = "getIdentityPublicKey")]
    pub async fn get_identity_public_key(&self) -> Result<PublicKeyBytes, JsValue> {
        self.inner
            .get_identity_public_key()
            .await
            .map(Into::into)
            .map_err(spark_handle_js_err)
    }

    #[wasm_bindgen(js_name = "getPublicKeyForLeaf")]
    pub async fn get_public_key_for_leaf(
        &self,
        leaf_id: ExternalTreeNodeId,
    ) -> Result<PublicKeyBytes, JsValue> {
        self.inner
            .get_public_key_for_leaf(leaf_id.into())
            .await
            .map(Into::into)
            .map_err(spark_handle_js_err)
    }

    #[wasm_bindgen(js_name = "getStaticDepositPublicKey")]
    pub async fn get_static_deposit_public_key(
        &self,
        index: u32,
    ) -> Result<PublicKeyBytes, JsValue> {
        self.inner
            .get_static_deposit_public_key(index)
            .await
            .map(Into::into)
            .map_err(spark_handle_js_err)
    }

    #[wasm_bindgen(js_name = "signAuthenticationChallenge")]
    pub async fn sign_authentication_challenge(
        &self,
        challenge: Vec<u8>,
    ) -> Result<EcdsaSignatureBytes, JsValue> {
        self.inner
            .sign_authentication_challenge(challenge)
            .await
            .map(Into::into)
            .map_err(spark_handle_js_err)
    }

    #[wasm_bindgen(js_name = "signMessage")]
    pub async fn sign_message(&self, message: Vec<u8>) -> Result<EcdsaSignatureBytes, JsValue> {
        self.inner
            .sign_message(message)
            .await
            .map(Into::into)
            .map_err(spark_handle_js_err)
    }

    #[wasm_bindgen(js_name = "signFrost")]
    pub async fn sign_frost(
        &self,
        jobs: Vec<ExternalFrostJob>,
    ) -> Result<Vec<ExternalFrostShareResult>, JsValue> {
        let results = self
            .inner
            .sign_frost(jobs.into_iter().map(Into::into).collect())
            .await
            .map_err(spark_handle_js_err)?;
        Ok(results.into_iter().map(Into::into).collect())
    }

    #[wasm_bindgen(js_name = "prepareTransfer")]
    pub async fn prepare_transfer(
        &self,
        request: ExternalPrepareTransferRequest,
    ) -> Result<ExternalPreparedTransfer, JsValue> {
        self.inner
            .prepare_transfer(request.into())
            .await
            .map(Into::into)
            .map_err(spark_handle_js_err)
    }

    #[wasm_bindgen(js_name = "prepareClaim")]
    pub async fn prepare_claim(
        &self,
        request: ExternalPrepareClaimRequest,
    ) -> Result<ExternalPreparedClaim, JsValue> {
        self.inner
            .prepare_claim(request.into())
            .await
            .map(Into::into)
            .map_err(spark_handle_js_err)
    }

    #[wasm_bindgen(js_name = "prepareLightningReceive")]
    pub async fn prepare_lightning_receive(
        &self,
        request: ExternalPrepareLightningReceiveRequest,
    ) -> Result<ExternalPreparedLightningReceive, JsValue> {
        self.inner
            .prepare_lightning_receive(request.into())
            .await
            .map(Into::into)
            .map_err(spark_handle_js_err)
    }

    #[wasm_bindgen(js_name = "prepareStaticDeposit")]
    pub async fn prepare_static_deposit(
        &self,
        request: ExternalPrepareStaticDepositRequest,
    ) -> Result<ExternalPreparedStaticDeposit, JsValue> {
        self.inner
            .prepare_static_deposit(request.into())
            .await
            .map(Into::into)
            .map_err(spark_handle_js_err)
    }

    #[wasm_bindgen(js_name = "startStaticDepositRefund")]
    pub async fn start_static_deposit_refund(
        &self,
        request: ExternalStartStaticDepositRefundRequest,
    ) -> Result<ExternalStartedStaticDepositRefund, JsValue> {
        self.inner
            .start_static_deposit_refund(request.into())
            .await
            .map(Into::into)
            .map_err(spark_handle_js_err)
    }

    #[wasm_bindgen(js_name = "signStaticDepositRefund")]
    pub async fn sign_static_deposit_refund(
        &self,
        request: ExternalSignStaticDepositRefundRequest,
    ) -> Result<ExternalFrostSignature, JsValue> {
        self.inner
            .sign_static_deposit_refund(request.into())
            .await
            .map(Into::into)
            .map_err(spark_handle_js_err)
    }

    #[wasm_bindgen(js_name = "signSparkInvoice")]
    pub async fn sign_spark_invoice(
        &self,
        request: ExternalSignSparkInvoiceRequest,
    ) -> Result<ExternalSignedSparkInvoice, JsValue> {
        self.inner
            .sign_spark_invoice(request.into())
            .await
            .map(Into::into)
            .map_err(spark_handle_js_err)
    }

    #[wasm_bindgen(js_name = "prepareTokenTransaction")]
    pub async fn prepare_token_transaction(
        &self,
        request: ExternalPrepareTokenTransactionRequest,
    ) -> Result<ExternalPreparedTokenTransaction, JsValue> {
        self.inner
            .prepare_token_transaction(request.into())
            .await
            .map(Into::into)
            .map_err(spark_handle_js_err)
    }

    #[wasm_bindgen(js_name = "prepareStaticDepositClaim")]
    pub async fn prepare_static_deposit_claim(
        &self,
        request: ExternalPrepareStaticDepositClaimRequest,
    ) -> Result<ExternalPreparedStaticDepositClaim, JsValue> {
        self.inner
            .prepare_static_deposit_claim(request.into())
            .await
            .map(Into::into)
            .map_err(spark_handle_js_err)
    }
}
