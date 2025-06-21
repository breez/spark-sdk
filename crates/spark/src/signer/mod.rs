mod default_signer;
mod error;

use std::collections::BTreeMap;

use bitcoin::secp256k1::ecdsa::Signature;
pub use default_signer::DefaultSigner;
pub use error::SignerError;

use bitcoin::{hashes::sha256, secp256k1::PublicKey};
use frost_secp256k1_tr::{Identifier, round1::SigningCommitments, round2::SignatureShare};

use crate::core::Network;

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
        adaptor_pub_key: Option<PublicKey>,
    ) -> Result<frost_secp256k1_tr::Signature, SignerError>;
    fn sign_message_ecdsa_with_identity_key<T: AsRef<[u8]>>(
        &self,
        message: T,
        apply_hashing: bool,
        network: Network,
    ) -> Result<Signature, SignerError>;
    async fn generate_frost_signing_commitments(&self) -> Result<SigningCommitments, SignerError>;
    fn generate_public_key(&self, hash: sha256::Hash) -> Result<PublicKey, SignerError>;
    fn get_identity_public_key(&self, account_index: u32) -> Result<PublicKey, SignerError>;
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
