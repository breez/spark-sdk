use std::collections::BTreeMap;

use bitcoin::secp256k1::PublicKey;
use frost_secp256k1_tr::{Identifier, round1::SigningCommitments, round2::SignatureShare};
use k256::{PublicKey as k256PublicKey, Scalar};

use crate::tree::TreeNodeId;

#[derive(Debug, Clone)]
pub struct SecretShare {
    /// Number of shares required to recover the secret
    pub threshold: usize,

    /// Index (x-coordinate) of the share
    pub index: Scalar,

    /// Share value (y-coordinate)
    pub share: Scalar,
}

#[derive(Debug, Clone)]
pub struct VerifiableSecretShare {
    /// Base secret share containing threshold, index, and share value
    pub secret_share: SecretShare,

    /// Cryptographic proofs for share verification
    pub proofs: Vec<k256PublicKey>,
}

#[derive(Clone, Debug)]
pub struct EncryptedSecret(Vec<u8>);

impl EncryptedSecret {
    pub fn new(ciphertext: Vec<u8>) -> Self {
        Self(ciphertext)
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub enum SecretSource {
    Derived(TreeNodeId),
    Encrypted(EncryptedSecret),
}

impl SecretSource {
    pub fn new_encrypted(ciphertext: Vec<u8>) -> Self {
        Self::Encrypted(EncryptedSecret::new(ciphertext))
    }
}

pub enum SecretToSplit {
    SecretSource(SecretSource),
    Preimage(Vec<u8>),
}

#[derive(Debug, Clone)]
pub struct FrostSigningCommitmentsWithNonces {
    pub commitments: SigningCommitments,
    pub nonces_ciphertext: Vec<u8>,
}

pub struct SignFrostRequest<'a> {
    pub message: &'a [u8],
    pub public_key: &'a PublicKey,
    pub private_key: &'a SecretSource,
    pub verifying_key: &'a PublicKey,
    pub self_nonce_commitment: &'a FrostSigningCommitmentsWithNonces,
    pub statechain_commitments: BTreeMap<Identifier, SigningCommitments>,
    pub adaptor_public_key: Option<&'a PublicKey>,
}

pub struct AggregateFrostRequest<'a> {
    pub message: &'a [u8],
    pub statechain_signatures: BTreeMap<Identifier, SignatureShare>,
    pub statechain_public_keys: BTreeMap<Identifier, PublicKey>,
    pub verifying_key: &'a PublicKey,
    pub statechain_commitments: BTreeMap<Identifier, SigningCommitments>,
    pub self_commitment: &'a SigningCommitments,
    pub public_key: &'a PublicKey,
    pub self_signature: &'a SignatureShare,
    pub adaptor_public_key: Option<&'a PublicKey>,
}
