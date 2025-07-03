mod default_signer;
mod error;
mod models;

use std::collections::BTreeMap;

use crate::tree::TreeNodeId;
use bitcoin::secp256k1::PublicKey;
use bitcoin::secp256k1::ecdsa::Signature;
use frost_secp256k1_tr::{Identifier, round1::SigningCommitments, round2::SignatureShare};

pub use default_signer::DefaultSigner;
pub use error::SignerError;
pub use models::VerifiableSecretShare;

pub enum Secret {
    PublicKey(PublicKey),
    Other(Vec<u8>),
}

#[async_trait::async_trait]
pub trait Signer {
    async fn aggregate_frost(
        &self,
        message: &[u8],
        statechain_signatures: BTreeMap<Identifier, SignatureShare>,
        statechain_public_keys: BTreeMap<Identifier, PublicKey>,
        verifying_key: &PublicKey,
        statechain_commitments: BTreeMap<Identifier, SigningCommitments>,
        self_commitment: &SigningCommitments,
        public_key: &PublicKey,
        self_signature: &SignatureShare,
        adaptor_public_key: Option<&PublicKey>,
    ) -> Result<frost_secp256k1_tr::Signature, SignerError>;
    fn sign_message_ecdsa_with_identity_key<T: AsRef<[u8]>>(
        &self,
        message: T,
    ) -> Result<Signature, SignerError>;
    async fn generate_frost_signing_commitments(&self) -> Result<SigningCommitments, SignerError>;
    // TODO: Create a method generate_public_key function that takes a leaf id.
    fn get_public_key_for_node(&self, id: &TreeNodeId) -> Result<PublicKey, SignerError>;
    fn generate_random_public_key(&self) -> Result<PublicKey, SignerError>;
    fn get_identity_public_key(&self) -> Result<PublicKey, SignerError>;

    /// Subtract two private keys given their public keys, returning the public key of the difference
    fn subtract_private_keys_given_public_keys(
        &self,
        signing_public_key: &PublicKey,
        new_signing_public_key: &PublicKey,
    ) -> Result<PublicKey, SignerError>;

    /// Split a secret into threshold shares with proofs
    ///
    /// If secret is a public key, the private key that matches the provided public key is used as secret.
    fn split_secret_with_proofs(
        &self,
        secret: &Secret,
        threshold: u32,
        num_shares: usize,
    ) -> Result<Vec<VerifiableSecretShare>, SignerError>;

    /// Encrypt the private key that matches the provided public key using ECIES for the receiver
    fn encrypt_leaf_private_key_ecies(
        &self,
        receiver_public_key: &PublicKey,
        public_key: &PublicKey,
    ) -> Result<Vec<u8>, SignerError>;

    /// Decrypt ECIES encrypted private key using the identity private key
    ///
    /// Persists the private key and returns matching public key
    fn decrypt_leaf_private_key_ecies(
        &self,
        encrypted_data: &[u8],
    ) -> Result<PublicKey, SignerError>;

    async fn sign_frost(
        &self,
        message: &[u8],
        public_key: &PublicKey,
        private_as_public_key: &PublicKey,
        verifying_key: &PublicKey,
        self_commitment: &SigningCommitments,
        statechain_commitments: BTreeMap<Identifier, SigningCommitments>,
        adaptor_public_key: Option<&PublicKey>,
    ) -> Result<SignatureShare, SignerError>;
}
