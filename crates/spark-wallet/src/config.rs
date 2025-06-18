use spark::Network;

pub struct SparkWalletConfig {
    pub network: Network,
    pub operators: Vec<SparkOperator>,
}

#[derive(Debug, Clone)]
pub struct SparkOperator {
    /// The index of the signing operator.
    pub id: u32,

    /// Identifier is the FROST identifier of the signing operator, which will be index + 1 in 32 bytes big endian hex string.
    /// Used as shamir secret share identifier in DKG key shares.
    pub frost_identifier: String,

    /// Address of the signing operator
    pub address: String,

    /// Public key of the signing operator
    pub identity_public_key: String,

    pub is_coordinator: bool,
}

impl SparkWalletConfig {
    pub fn get_coordinator(&self) -> SparkOperator {
        self.operators
            .iter()
            .find(|op| op.is_coordinator)
            .unwrap()
            .clone()
    }
}
