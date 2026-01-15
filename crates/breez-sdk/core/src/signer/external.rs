use crate::error::SignerError;

use super::external_types::{
    EcdsaSignatureBytes, ExternalAggregateFrostRequest, ExternalEncryptedPrivateKey,
    ExternalFrostCommitments, ExternalFrostSignature, ExternalFrostSignatureShare,
    ExternalPrivateKeySource, ExternalSecretToSplit, ExternalSignFrostRequest, ExternalTreeNodeId,
    ExternalVerifiableSecretShare, HashedMessageBytes, MessageBytes, PrivateKeyBytes,
    PublicKeyBytes, RecoverableEcdsaSignatureBytes, SchnorrSignatureBytes,
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
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
#[macros::async_trait]
pub trait ExternalSigner: Send + Sync {
    /// Returns the identity public key as 33 bytes (compressed secp256k1 key).
    fn identity_public_key(&self) -> Result<PublicKeyBytes, SignerError>;

    /// Derives a public key for the given BIP32 derivation path.
    ///
    /// # Arguments
    /// * `path` - BIP32 derivation path as a string (e.g., "m/44'/0'/0'/0/0")
    ///
    /// # Returns
    /// The derived public key as 33 bytes, or a `SignerError`
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
    async fn ecies_encrypt(&self, message: Vec<u8>, path: String) -> Result<Vec<u8>, SignerError>;

    /// Decrypts a message using ECIES at the given derivation path.
    ///
    /// # Arguments
    /// * `message` - The encrypted message
    /// * `path` - BIP32 derivation path for the decryption key
    ///
    /// # Returns
    /// Decrypted data, or a `SignerError`
    async fn ecies_decrypt(&self, message: Vec<u8>, path: String) -> Result<Vec<u8>, SignerError>;

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
    async fn hmac_sha256(
        &self,
        message: Vec<u8>,
        path: String,
    ) -> Result<HashedMessageBytes, SignerError>;
    /// Generates Frost signing commitments for multi-party signing.
    ///
    /// # Returns
    /// Frost commitments with nonces, or a `SignerError`
    async fn generate_frost_signing_commitments(
        &self,
    ) -> Result<ExternalFrostCommitments, SignerError>;

    /// Gets the public key for a specific tree node in the Spark wallet.
    ///
    /// # Arguments
    /// * `id` - The tree node identifier
    ///
    /// # Returns
    /// The public key for the node, or an error string
    async fn get_public_key_for_node(
        &self,
        id: ExternalTreeNodeId,
    ) -> Result<PublicKeyBytes, SignerError>;

    /// Generates a random private key.
    ///
    /// # Returns
    /// A randomly generated private key source, or an error string
    async fn generate_random_key(&self) -> Result<ExternalPrivateKeySource, SignerError>;

    /// Gets a static deposit private key source by index.
    ///
    /// # Arguments
    /// * `index` - The index of the static deposit key
    ///
    /// # Returns
    /// The private key source, or an error string
    async fn get_static_deposit_private_key_source(
        &self,
        index: u32,
    ) -> Result<ExternalPrivateKeySource, SignerError>;

    /// Gets a static deposit private key by index.
    ///
    /// # Arguments
    /// * `index` - The index of the static deposit key
    ///
    /// # Returns
    /// The 32-byte private key, or an error string
    async fn get_static_deposit_private_key(
        &self,
        index: u32,
    ) -> Result<PrivateKeyBytes, SignerError>;

    /// Gets a static deposit public key by index.
    ///
    /// # Arguments
    /// * `index` - The index of the static deposit key
    ///
    /// # Returns
    /// The 33-byte public key, or an error string
    async fn get_static_deposit_public_key(
        &self,
        index: u32,
    ) -> Result<PublicKeyBytes, SignerError>;

    /// Subtracts one private key from another.
    ///
    /// # Arguments
    /// * `signing_key` - The first private key source
    /// * `new_signing_key` - The second private key source to subtract
    ///
    /// # Returns
    /// The resulting private key source, or an error string
    async fn subtract_private_keys(
        &self,
        signing_key: ExternalPrivateKeySource,
        new_signing_key: ExternalPrivateKeySource,
    ) -> Result<ExternalPrivateKeySource, SignerError>;

    /// Splits a secret with proofs using Shamir's Secret Sharing.
    ///
    /// # Arguments
    /// * `secret` - The secret to split
    /// * `threshold` - Minimum number of shares needed to reconstruct
    /// * `num_shares` - Total number of shares to create
    ///
    /// # Returns
    /// Vector of verifiable secret shares, or an error string
    async fn split_secret(
        &self,
        secret: ExternalSecretToSplit,
        threshold: u32,
        num_shares: u32,
    ) -> Result<Vec<ExternalVerifiableSecretShare>, SignerError>;

    /// Encrypts a private key for a specific receiver's public key.
    ///
    /// # Arguments
    /// * `private_key` - The encrypted private key to re-encrypt
    /// * `receiver_public_key` - The receiver's 33-byte public key
    ///
    /// # Returns
    /// Encrypted data for the receiver, or an error string
    async fn encrypt_private_key_for_receiver(
        &self,
        private_key: ExternalEncryptedPrivateKey,
        receiver_public_key: PublicKeyBytes,
    ) -> Result<Vec<u8>, SignerError>;

    /// Gets the public key from a private key source.
    ///
    /// # Arguments
    /// * `private_key` - The private key source
    ///
    /// # Returns
    /// The corresponding 33-byte public key, or an error string
    async fn get_public_key_from_private_key_source(
        &self,
        private_key: ExternalPrivateKeySource,
    ) -> Result<PublicKeyBytes, SignerError>;

    /// Signs using Frost protocol (multi-party signing).
    ///
    /// # Arguments
    /// * `request` - The Frost signing request
    ///
    /// # Returns
    /// A signature share, or an error string
    async fn sign_frost(
        &self,
        request: ExternalSignFrostRequest,
    ) -> Result<ExternalFrostSignatureShare, SignerError>;

    /// Aggregates Frost signature shares into a final signature.
    ///
    /// # Arguments
    /// * `request` - The Frost aggregation request
    ///
    /// # Returns
    /// The aggregated Frost signature, or an error string
    async fn aggregate_frost_signatures(
        &self,
        request: ExternalAggregateFrostRequest,
    ) -> Result<ExternalFrostSignature, SignerError>;
}
