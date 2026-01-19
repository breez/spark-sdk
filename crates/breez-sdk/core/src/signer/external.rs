use crate::error::SignerError;

use super::external_types::{
    EcdsaSignatureBytes, ExternalAggregateFrostRequest, ExternalEncryptedSecret,
    ExternalFrostCommitments, ExternalFrostSignature, ExternalFrostSignatureShare,
    ExternalSecretSource, ExternalSecretToSplit, ExternalSignFrostRequest, ExternalTreeNodeId,
    ExternalVerifiableSecretShare, HashedMessageBytes, MessageBytes, PublicKeyBytes,
    RecoverableEcdsaSignatureBytes, SchnorrSignatureBytes, SecretBytes,
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
    ///
    /// See also: [JavaScript `getIdentityPublicKey`](https://docs.spark.money/wallets/spark-signer#get-identity-public-key)
    fn identity_public_key(&self) -> Result<PublicKeyBytes, SignerError>;

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
    /// Generates Frost signing commitments for multi-party signing.
    ///
    /// # Returns
    /// Frost commitments with nonces, or a `SignerError`
    ///
    /// See also: [JavaScript `getRandomSigningCommitment`](https://docs.spark.money/wallets/spark-signer#get-random-signing-commitment)
    async fn generate_random_signing_commitment(
        &self,
    ) -> Result<ExternalFrostCommitments, SignerError>;

    /// Gets the public key for a specific tree node in the Spark wallet.
    ///
    /// # Arguments
    /// * `id` - The tree node identifier
    ///
    /// # Returns
    /// The public key for the node, or a `SignerError`
    async fn get_public_key_for_node(
        &self,
        id: ExternalTreeNodeId,
    ) -> Result<PublicKeyBytes, SignerError>;

    /// Generates a random secret that is encrypted and known only to the signer.
    ///
    /// This method creates a new random secret and returns it in encrypted form.
    /// The plaintext secret never leaves the signer boundary, providing a secure way
    /// to create secrets that can be referenced in subsequent operations without
    /// exposing them.
    ///
    /// This is conceptually similar to Spark's key derivation system where secrets
    /// are represented by opaque references (like tree node IDs or Random) rather than raw values.
    /// The encrypted secret can be passed to other signer methods that need to operate
    /// on it, while keeping the actual secret material protected within the signer.
    ///
    /// # Returns
    /// An encrypted secret that can be used in subsequent signer operations,
    /// or a `SignerError` if generation fails.
    ///
    /// See also: [Key Derivation System](https://docs.spark.money/wallets/spark-signer#the-keyderivation-system)
    async fn generate_random_secret(&self) -> Result<ExternalEncryptedSecret, SignerError>;

    /// Gets an encrypted static deposit secret by index.
    ///
    /// # Arguments
    /// * `index` - The index of the static deposit secret
    ///
    /// # Returns
    /// The encrypted secret, or a `SignerError`
    ///
    /// This is the encrypted version of: [JavaScript `getStaticDepositSecretKey`](https://docs.spark.money/wallets/spark-signer#get-static-deposit-secret-key)
    async fn static_deposit_secret_encrypted(
        &self,
        index: u32,
    ) -> Result<ExternalSecretSource, SignerError>;

    /// Gets a static deposit secret by index.
    ///
    /// # Arguments
    /// * `index` - The index of the static deposit secret
    ///
    /// # Returns
    /// The 32-byte secret, or a `SignerError`
    ///
    /// See also: [JavaScript `getStaticDepositSecretKey`](https://docs.spark.money/wallets/spark-signer#get-static-deposit-secret-key)
    async fn static_deposit_secret(&self, index: u32) -> Result<SecretBytes, SignerError>;

    /// Gets a static deposit signing public key by index.
    ///
    /// # Arguments
    /// * `index` - The index of the static deposit public signing key
    ///
    /// # Returns
    /// The 33-byte public key, or a `SignerError`
    ///
    /// See also: [JavaScript `getStaticDepositSigningKey`](https://docs.spark.money/wallets/spark-signer#get-static-deposit-signing-key)
    async fn static_deposit_signing_key(&self, index: u32) -> Result<PublicKeyBytes, SignerError>;

    /// Subtracts one secret from another.
    ///
    /// # Arguments
    /// * `signing_key` - The first secret
    /// * `new_signing_key` - The second secret to subtract
    ///
    /// # Returns
    /// The resulting secret, or a `SignerError`
    ///
    /// See also: [JavaScript `subtractSplitAndEncrypt`](https://docs.spark.money/wallets/spark-signer#subtract,-split,-and-encrypt)
    /// (this method provides the subtraction step of that higher-level operation)
    async fn subtract_secrets(
        &self,
        signing_key: ExternalSecretSource,
        new_signing_key: ExternalSecretSource,
    ) -> Result<ExternalSecretSource, SignerError>;

    /// Splits a secret with proofs using Shamir's Secret Sharing.
    ///
    /// # Arguments
    /// * `secret` - The secret to split
    /// * `threshold` - Minimum number of shares needed to reconstruct
    /// * `num_shares` - Total number of shares to create
    ///
    /// # Returns
    /// Vector of verifiable secret shares, or a `SignerError`
    ///
    /// See also: [JavaScript `splitSecretWithProofs`](https://docs.spark.money/wallets/spark-signer#split-secret-with-proofs)
    async fn split_secret_with_proofs(
        &self,
        secret: ExternalSecretToSplit,
        threshold: u32,
        num_shares: u32,
    ) -> Result<Vec<ExternalVerifiableSecretShare>, SignerError>;

    /// Encrypts a secret for a specific receiver's public key.
    ///
    /// # Arguments
    /// * `encrypted_secret` - The encrypted secret to re-encrypt
    /// * `receiver_public_key` - The receiver's 33-byte public key
    ///
    /// # Returns
    /// Encrypted data for the receiver, or a `SignerError`
    async fn encrypt_secret_for_receiver(
        &self,
        encrypted_secret: ExternalEncryptedSecret,
        receiver_public_key: PublicKeyBytes,
    ) -> Result<Vec<u8>, SignerError>;

    /// Gets the public key from a secret.
    ///
    /// # Arguments
    /// * `secret` - The secret
    ///
    /// # Returns
    /// The corresponding 33-byte public key, or a `SignerError`
    ///
    /// See also: [JavaScript `getPublicKeyFromDerivation`](https://docs.spark.money/wallets/spark-signer#get-public-key-from-derivation)
    async fn public_key_from_secret(
        &self,
        secret: ExternalSecretSource,
    ) -> Result<PublicKeyBytes, SignerError>;

    /// Signs using Frost protocol (multi-party signing).
    ///
    /// # Arguments
    /// * `request` - The Frost signing request
    ///
    /// # Returns
    /// A signature share, or a `SignerError`
    ///
    /// See also: [JavaScript `signFrost`](https://docs.spark.money/wallets/spark-signer#frost-signing)
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
    /// The aggregated Frost signature, or a `SignerError`
    ///
    /// See also: [JavaScript `aggregateFrost`](https://docs.spark.money/wallets/spark-signer#aggregate-frost-signatures)
    async fn aggregate_frost(
        &self,
        request: ExternalAggregateFrostRequest,
    ) -> Result<ExternalFrostSignature, SignerError>;
}
