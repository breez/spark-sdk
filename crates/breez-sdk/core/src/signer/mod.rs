use crate::SdkError;
use bitcoin::bip32::DerivationPath;
use bitcoin::hashes::{Hmac, sha256};
use bitcoin::secp256k1::{self, Message, ecdsa::RecoverableSignature};

#[macros::async_trait]
pub trait BreezSigner: Send + Sync {
    /// Returns the identity public key.
    fn identity_public_key(&self) -> Result<secp256k1::PublicKey, SdkError>;

    /// Signs a pre-hashed message using ECDSA at the given derivation path.
    ///
    /// The caller must create the Message from a 32-byte hash digest before calling this method.
    async fn sign_ecdsa(
        &self,
        message: Message,
        path: &DerivationPath,
    ) -> Result<secp256k1::ecdsa::Signature, SdkError>;

    /// Signs a pre-hashed message using recoverable ECDSA at the given derivation path.
    ///
    /// The caller must create the Message from a 32-byte hash digest before calling.
    async fn sign_ecdsa_recoverable(
        &self,
        message: Message,
        path: &DerivationPath,
    ) -> Result<RecoverableSignature, SdkError>;

    async fn encrypt_ecies(
        &self,
        message: &[u8],
        path: &DerivationPath,
    ) -> Result<Vec<u8>, SdkError>;

    async fn decrypt_ecies(
        &self,
        message: &[u8],
        path: &DerivationPath,
    ) -> Result<Vec<u8>, SdkError>;

    async fn sign_hash_schnorr(
        &self,
        hash: &[u8],
        path: &DerivationPath,
    ) -> Result<secp256k1::schnorr::Signature, SdkError>;

    async fn derive_public_key(
        &self,
        path: &DerivationPath,
    ) -> Result<secp256k1::PublicKey, SdkError>;

    async fn generate_random_signing_commitment(
        &self,
    ) -> Result<spark_wallet::FrostSigningCommitmentsWithNonces, SdkError>;

    async fn get_public_key_for_node(
        &self,
        id: &spark_wallet::TreeNodeId,
    ) -> Result<secp256k1::PublicKey, SdkError>;

    async fn generate_random_key(&self) -> Result<spark_wallet::SecretSource, SdkError>;

    async fn static_deposit_secret_encrypted(
        &self,
        index: u32,
    ) -> Result<spark_wallet::SecretSource, SdkError>;

    async fn static_deposit_secret(&self, index: u32) -> Result<secp256k1::SecretKey, SdkError>;

    async fn static_deposit_signing_key(
        &self,
        index: u32,
    ) -> Result<secp256k1::PublicKey, SdkError>;

    async fn subtract_secrets(
        &self,
        signing_key: &spark_wallet::SecretSource,
        new_signing_key: &spark_wallet::SecretSource,
    ) -> Result<spark_wallet::SecretSource, SdkError>;

    async fn split_secret_with_proofs(
        &self,
        secret: &spark_wallet::SecretToSplit,
        threshold: u32,
        num_shares: usize,
    ) -> Result<Vec<spark_wallet::VerifiableSecretShare>, SdkError>;

    async fn encrypt_secret_for_receiver(
        &self,
        private_key: &spark_wallet::EncryptedSecret,
        receiver_public_key: &secp256k1::PublicKey,
    ) -> Result<Vec<u8>, SdkError>;

    async fn public_key_from_secret(
        &self,
        private_key: &spark_wallet::SecretSource,
    ) -> Result<secp256k1::PublicKey, SdkError>;

    async fn sign_frost<'a>(
        &self,
        request: spark_wallet::SignFrostRequest<'a>,
    ) -> Result<frost_secp256k1_tr::round2::SignatureShare, SdkError>;

    async fn aggregate_frost<'a>(
        &self,
        request: spark_wallet::AggregateFrostRequest<'a>,
    ) -> Result<frost_secp256k1_tr::Signature, SdkError>;

    /// Computes HMAC-SHA256 using a key derived at the given path.
    async fn hmac_sha256(
        &self,
        key_path: &DerivationPath,
        input: &[u8],
    ) -> Result<Hmac<sha256::Hash>, SdkError>;
}

// External signer support - private adapter
mod adapter;
mod default_external;

// Public external signer API
pub mod external;
pub mod external_types;

// Re-export only the external signer trait and types
pub use external::ExternalSigner;
pub use external_types::*;

// Internal-only exports (used by adapter and builder)
pub(crate) use adapter::ExternalSignerAdapter;
pub(crate) use default_external::DefaultExternalSigner;
pub mod breez;
pub mod lnurl_auth;
pub mod nostr;
pub mod rtsync;
pub mod spark;
