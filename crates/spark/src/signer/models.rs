use k256::Scalar;

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
    pub proofs: Vec<Vec<u8>>,
}
