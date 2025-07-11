mod default_signer;
mod error;
mod models;
mod secret_sharing;


use crate::tree::TreeNodeId;
use bitcoin::secp256k1::PublicKey;
use bitcoin::secp256k1::ecdsa::Signature;
use frost_secp256k1_tr::{round1::SigningCommitments, round2::SignatureShare};

pub use default_signer::DefaultSigner;
pub use error::SignerError;
pub use models::*;
pub(crate) use secret_sharing::from_bytes_to_scalar;

#[async_trait::async_trait]
pub trait Signer {
    fn sign_message_ecdsa_with_identity_key<T: AsRef<[u8]>>(
        &self,
        message: T,
    ) -> Result<Signature, SignerError>;

    async fn generate_frost_signing_commitments(&self) -> Result<SigningCommitments, SignerError>;

    fn get_public_key_for_node(&self, id: &TreeNodeId) -> Result<PublicKey, SignerError>;

    fn generate_random_key(&self) -> Result<PrivateKeySource, SignerError>;

    fn get_identity_public_key(&self) -> Result<PublicKey, SignerError>;

    /// Subtract two private keys
    ///
    /// Returns the resulting private key (encrypted)
    fn subtract_private_keys(
        &self,
        signing_key: &PrivateKeySource,
        new_signing_key: &PrivateKeySource,
    ) -> Result<PrivateKeySource, SignerError>;

    /// Split a secret into threshold shares with proofs
    fn split_secret_with_proofs(
        &self,
        secret: &SecretToSplit,
        threshold: u32,
        num_shares: usize,
    ) -> Result<Vec<VerifiableSecretShare>, SignerError>;

    /// Takes an encrypted private key (encrypted for us) and returns an encrypted private key (encrypted for receiver)
    fn encrypt_private_key_for_receiver(
        &self,
        private_key: &EncryptedPrivateKey,
        receiver_public_key: &PublicKey,
    ) -> Result<Vec<u8>, SignerError>;

    fn get_public_key_from_private_key_source(
        &self,
        private_key: &PrivateKeySource,
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

    /// Aggregates FROST (Flexible Round-Optimized Schnorr Threshold) signature shares into a complete signature
    ///
    /// This function takes signature shares from multiple parties (statechain and user),
    /// combines them with the corresponding public keys and commitments, and produces
    /// a single aggregated threshold signature that can be verified using the group's verifying key.
    ///
    /// # Parameters
    /// * `message` - The message being signed
    /// * `statechain_signatures` - Map of identifier to signature shares from statechain participants
    /// * `statechain_public_keys` - Map of identifier to public keys from statechain participants
    /// * `verifying_key` - The group's verifying key used to validate the final signature
    /// * `statechain_commitments` - Map of identifier to commitment values from statechain participants
    /// * `self_commitment` - The local user's commitment value
    /// * `public_key` - The local user's public key
    /// * `self_signature` - The local user's signature share
    /// * `adaptor_public_key` - Optional adaptor public key for adaptor signatures
    ///
    /// # Returns
    /// A complete FROST signature that can be verified against the group's public key
    async fn aggregate_frost<'a>(
        &self,
        request: AggregateFrostRequest<'a>,
    ) -> Result<frost_secp256k1_tr::Signature, SignerError>;
}
