use crate::SdkError;
use bitcoin::bip32::DerivationPath;
use bitcoin::hashes::{Hmac, sha256};
use bitcoin::secp256k1::{self, Message, ecdsa::RecoverableSignature};

#[macros::async_trait]
pub trait BreezSigner: Send + Sync {
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
mod default_external_spark;

// External spark signer support - private adapter
mod external_spark_adapter;

// Public external signer API
pub mod external;
pub mod external_spark;
pub mod external_spark_types;
pub mod external_types;

// Re-export only the external signer traits and types
pub use external::ExternalBreezSigner;
pub use external_spark::ExternalSparkSigner;
pub use external_spark_types::*;
pub use external_types::*;

// Internal-only exports (used by adapter and builder)
pub(crate) use adapter::ExternalBreezSignerAdapter;
pub(crate) use default_external::DefaultExternalSigner;
pub(crate) use default_external_spark::DefaultExternalSparkSigner;
pub(crate) use external_spark_adapter::ExternalSparkSignerAdapter;
pub mod breez;
pub mod lnurl_auth;
pub mod rtsync;
