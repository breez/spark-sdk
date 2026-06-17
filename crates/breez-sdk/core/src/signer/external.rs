use crate::error::SignerError;

use super::external_types::{
    EcdsaSignatureBytes, HashedMessageBytes, MessageBytes, PublicKeyBytes,
    RecoverableEcdsaSignatureBytes, SchnorrSignatureBytes,
};

/// External signer trait that can be implemented by users and passed to the SDK.
///
/// This trait mirrors the `BreezSigner` trait but uses FFI-compatible types (bytes, strings)
/// instead of Rust-specific types. This allows it to be exposed through FFI and WASM bindings.
///
/// All methods accept and return simple types:
/// - Derivation paths as strings (e.g., "m/44'/0'/0'")
/// - Public keys, signatures, and other crypto primitives as Vec<u8>
/// - Spark-specific types as serialized representations
///
/// Errors are returned as `SignerError` for FFI compatibility.
#[cfg_attr(
    feature = "uniffi",
    uniffi::export(with_foreign, async_runtime = "tokio")
)]
#[macros::async_trait]
pub trait ExternalBreezSigner: Send + Sync {
    /// Derives a public key for the given BIP32 derivation path.
    ///
    /// # Arguments
    /// * `path` - BIP32 derivation path as a string (e.g., "m/44'/0'/0'/0/0")
    ///
    /// # Returns
    /// The derived public key as 33 bytes, or a `SignerError`
    ///
    /// See also: [JavaScript `getPublicKeyFromDerivation`](https://docs.spark.money/wallets/spark-signer#get-public-key-from-derivation)
    async fn derive_public_key(&self, path: String) -> Result<PublicKeyBytes, SignerError>;

    /// Signs a message using ECDSA at the given derivation path.
    ///
    /// The message should be a 32-byte digest (typically a hash of the original data).
    ///
    /// # Arguments
    /// * `message` - The 32-byte message digest to sign
    /// * `path` - BIP32 derivation path as a string
    ///
    /// # Returns
    /// 64-byte compact ECDSA signature, or a `SignerError`
    async fn sign_ecdsa(
        &self,
        message: MessageBytes,
        path: String,
    ) -> Result<EcdsaSignatureBytes, SignerError>;

    /// Signs a message using recoverable ECDSA at the given derivation path.
    ///
    /// The message should be a 32-byte digest (typically a hash of the original data).
    ///
    /// # Arguments
    /// * `message` - The 32-byte message digest to sign
    /// * `path` - BIP32 derivation path as a string
    ///
    /// # Returns
    /// 65 bytes: recovery ID (31 + `recovery_id`) + 64-byte signature, or a `SignerError`
    async fn sign_ecdsa_recoverable(
        &self,
        message: MessageBytes,
        path: String,
    ) -> Result<RecoverableEcdsaSignatureBytes, SignerError>;

    /// Encrypts a message using ECIES at the given derivation path.
    ///
    /// # Arguments
    /// * `message` - The message to encrypt
    /// * `path` - BIP32 derivation path for the encryption key
    ///
    /// # Returns
    /// Encrypted data, or a `SignerError`
    async fn encrypt_ecies(&self, message: Vec<u8>, path: String) -> Result<Vec<u8>, SignerError>;

    /// Decrypts a message using ECIES at the given derivation path.
    ///
    /// # Arguments
    /// * `message` - The encrypted message
    /// * `path` - BIP32 derivation path for the decryption key
    ///
    /// # Returns
    /// Decrypted data, or a `SignerError`
    ///
    /// See also: [JavaScript `decryptEcies`](https://docs.spark.money/wallets/spark-signer#decrypt-ecies)
    async fn decrypt_ecies(&self, message: Vec<u8>, path: String) -> Result<Vec<u8>, SignerError>;

    /// Signs a hash using Schnorr signature at the given derivation path.
    ///
    /// # Arguments
    /// * `hash` - The 32-byte hash to sign (must be 32 bytes)
    /// * `path` - BIP32 derivation path as a string
    ///
    /// # Returns
    /// 64-byte Schnorr signature, or a `SignerError`
    async fn sign_hash_schnorr(
        &self,
        hash: Vec<u8>,
        path: String,
    ) -> Result<SchnorrSignatureBytes, SignerError>;

    /// HMAC-SHA256 of a message at the given derivation path.
    ///
    /// # Arguments
    /// * `message` - The message to hash
    /// * `path` - BIP32 derivation path as a string
    ///
    /// # Returns
    /// 32-byte HMAC-SHA256, or a `SignerError`
    ///
    /// See also: [JavaScript `htlcHMAC`](https://docs.spark.money/wallets/spark-signer#generate-htlc-hmac)
    async fn hmac_sha256(
        &self,
        message: Vec<u8>,
        path: String,
    ) -> Result<HashedMessageBytes, SignerError>;
}
