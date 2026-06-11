mod default_signer;
mod error;
mod models;
mod secret_sharing;
mod spark_signer;
mod spark_signer_adapter;

use bitcoin::bip32::DerivationPath;
use bitcoin::secp256k1::ecdsa::Signature;
use bitcoin::secp256k1::{PublicKey, SecretKey, schnorr};
use frost_secp256k1_tr::round2::SignatureShare;

pub use default_signer::{
    DefaultSigner, DefaultSignerError, account_master_key, default_account_number,
    identity_master_key, identity_public_key,
};
pub use error::SignerError;
pub use models::*;
pub use spark_signer::*;
pub use spark_signer_adapter::SparkSignerAdapter;

#[cfg(test)]
pub(crate) use default_signer::tests::create_test_signer;

#[macros::async_trait]
pub trait Signer: Send + Sync + 'static {
    /// Public key of the key derived at `path` under the master.
    async fn derive_public_key(&self, path: &DerivationPath) -> Result<PublicKey, SignerError>;

    /// Raw secret key derived at `path` under the master.
    async fn secret_key(&self, path: &DerivationPath) -> Result<SecretKey, SignerError>;

    /// ECDSA-sign `message` (hashed with SHA-256 internally) with the key at `path`.
    async fn sign_message_ecdsa(
        &self,
        path: &DerivationPath,
        message: &[u8],
    ) -> Result<Signature, SignerError>;

    /// Schnorr-sign a 32-byte `hash` with the key at `path`.
    async fn sign_hash_schnorr(
        &self,
        path: &DerivationPath,
        hash: &[u8],
    ) -> Result<schnorr::Signature, SignerError>;

    async fn generate_random_signing_commitment(
        &self,
    ) -> Result<FrostSigningCommitmentsWithNonces, SignerError>;

    async fn generate_random_secret(&self) -> Result<EncryptedSecret, SignerError>;

    /// Subtract two private keys
    ///
    /// Returns the resulting private key (encrypted)
    async fn subtract_secrets(
        &self,
        signing_key: &SecretSource,
        new_signing_key: &SecretSource,
    ) -> Result<SecretSource, SignerError>;

    /// Split a secret into threshold shares with proofs
    async fn split_secret_with_proofs(
        &self,
        secret: &SecretToSplit,
        threshold: u32,
        num_shares: usize,
    ) -> Result<Vec<VerifiableSecretShare>, SignerError>;

    /// Decrypts `secret` (held by us) and re-encrypts it for `receiver_public_key`.
    async fn encrypt_secret_for_receiver(
        &self,
        secret: &SecretSource,
        receiver_public_key: &PublicKey,
    ) -> Result<Vec<u8>, SignerError>;

    async fn public_key_from_secret(
        &self,
        private_key: &SecretSource,
    ) -> Result<PublicKey, SignerError>;

    /// Creates a FROST signature share for threshold signing
    ///
    /// This function generates a partial signature (signature share) that will be combined
    /// with other shares from statechain participants to create a complete threshold signature.
    /// It uses pre-generated nonce commitments and the corresponding signing key.
    ///
    /// # Parameters
    /// * `message` - The message being signed
    /// * `public_key` - The public key associated with the local signing key
    /// * `private_key` - Private key used for signing
    /// * `verifying_key` - The group's verifying key (threshold public key)
    /// * `self_commitment` - The local user's previously generated commitment
    /// * `statechain_commitments` - Map of identifier to commitment values from statechain participants
    /// * `adaptor_public_key` - Optional adaptor public key for adaptor signatures
    ///
    /// # Returns
    /// A signature share that can be combined with other shares to form a complete signature
    ///
    /// # Errors
    /// * `UnknownNonceCommitment` - If the provided commitment doesn't match any stored nonce
    /// * `UnknownKey` - If the public key doesn't correspond to any known private key
    /// * `SerializationError` - If there are issues serializing cryptographic components
    async fn sign_frost<'a>(
        &self,
        request: SignFrostRequest<'a>,
    ) -> Result<SignatureShare, SignerError>;
}
