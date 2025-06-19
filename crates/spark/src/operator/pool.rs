use bitcoin::secp256k1::PublicKey;
use frost_secp256k1_tr::Identifier;
use tonic::transport::Uri;

use super::OperatorError;

#[derive(Clone, Debug)]
pub struct OperatorPool {
    coordinator_index: usize,
    operators: Vec<Operator>,
}

impl OperatorPool {
    pub fn new(coordinator_index: usize, operators: Vec<Operator>) -> Result<Self, OperatorError> {
        if coordinator_index >= operators.len() {
            return Err(OperatorError::InvalidCoordinatorIndex);
        }

        // Ensure the operator ids make sense and are in order.
        for (index, operator) in operators.iter().enumerate() {
            if operator.id != index {
                return Err(OperatorError::InvalidOperatorId);
            }
        }

        Ok(Self {
            coordinator_index,
            operators,
        })
    }

    /// Returns the coordinator operator.
    pub fn get_coordinator(&self) -> &Operator {
        self.operators.get(self.coordinator_index).unwrap()
    }

    /// Returns an iterator over all operators except the coordinator.
    pub fn get_signing_operators(&self) -> impl Iterator<Item = &Operator> {
        self.operators
            .iter()
            .filter(|op| op.id != self.coordinator_index)
    }

    /// Returns the operator at the given index.
    pub fn get_operator_by_id(&self, id: usize) -> Option<&Operator> {
        self.operators.get(id)
    }
}

#[derive(Clone, Debug)]
pub struct Operator {
    pub id: usize,
    pub identifier: Identifier,
    pub address: Uri,
    pub identity_public_key: PublicKey,
}

impl Operator {}
