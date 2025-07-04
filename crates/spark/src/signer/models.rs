use k256::{PublicKey, Scalar};

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
    pub proofs: Vec<PublicKey>,
}

#[derive(Clone, Debug)]
pub struct EncryptedPrivateKey(Vec<u8>);

impl EncryptedPrivateKey {
    pub fn new(ciphertext: Vec<u8>) -> Self {
        Self(ciphertext)
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub enum PrivateKeySource {
    Derived(TreeNodeId),
    Encrypted(EncryptedPrivateKey),
}

impl PrivateKeySource {
    pub fn new_encrypted(ciphertext: Vec<u8>) -> Self {
        Self::Encrypted(EncryptedPrivateKey::new(ciphertext))
    }
}

pub enum SplitSecretWithProofSecretType {
    PrivateKey(PrivateKeySource),
    Other(Vec<u8>),
}
