use bitcoin::secp256k1::PublicKey;
use frost_secp256k1_tr::Identifier;
use serde::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, serde_as};
use tonic::transport::Uri;

use crate::{
    operator::rpc::{ConnectionManager, OperatorRpcError, SparkRpcClient},
    signer::Signer,
};

use super::OperatorError;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OperatorPoolConfig {
    coordinator_index: usize,
    operators: Vec<OperatorConfig>,
}

impl OperatorPoolConfig {
    pub fn new(
        coordinator_index: usize,
        operators: Vec<OperatorConfig>,
    ) -> Result<Self, OperatorError> {
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

    pub async fn connect<S>(
        &self,
        connection_manager: &ConnectionManager,
        signer: &S,
    ) -> Result<OperatorPool<S>, OperatorRpcError>
    where
        S: Signer + Clone,
    {
        let mut operators = Vec::new();
        for operator in &self.operators {
            let channel = connection_manager.get_channel(operator).await?;
            let client = SparkRpcClient::new(channel, signer.clone());
            operators.push(Operator {
                client,
                id: operator.id,
                identifier: operator.identifier,
                identity_public_key: operator.identity_public_key,
            });
        }

        Ok(OperatorPool {
            coordinator_index: self.coordinator_index,
            operators,
        })
    }

    /// Returns the coordinator operator.
    pub fn get_coordinator(&self) -> &OperatorConfig {
        self.operators.get(self.coordinator_index).unwrap()
    }

    /// Returns an iterator over all operators, including the coordinator.
    pub fn get_all_operators(&self) -> impl Iterator<Item = &OperatorConfig> {
        self.operators.iter()
    }

    /// Returns an iterator over all operators except the coordinator.
    pub fn get_non_coordinator_operators(&self) -> impl Iterator<Item = &OperatorConfig> {
        self.operators
            .iter()
            .filter(|op| op.id != self.coordinator_index)
    }

    /// Returns the operator at the given index.
    pub fn get_operator_by_id(&self, id: usize) -> Option<&OperatorConfig> {
        self.operators.get(id)
    }
}

#[serde_as]
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OperatorConfig {
    pub id: usize,
    pub identifier: Identifier,
    #[serde_as(as = "DisplayFromStr")]
    pub address: Uri,
    pub identity_public_key: PublicKey,
}

impl OperatorConfig {}

pub struct Operator<S> {
    pub client: SparkRpcClient<S>,
    pub id: usize,
    pub identifier: Identifier,
    pub identity_public_key: PublicKey,
}

pub struct OperatorPool<S> {
    coordinator_index: usize,
    operators: Vec<Operator<S>>,
}

impl<S> OperatorPool<S> {
    /// Returns the coordinator operator.
    pub fn get_coordinator(&self) -> &Operator<S> {
        self.operators.get(self.coordinator_index).unwrap()
    }

    /// Returns an iterator over all operators, including the coordinator.
    pub fn get_all_operators(&self) -> impl Iterator<Item = &Operator<S>> {
        self.operators.iter()
    }

    /// Returns an iterator over all operators except the coordinator.
    pub fn get_non_coordinator_operators(&self) -> impl Iterator<Item = &Operator<S>> {
        self.operators
            .iter()
            .filter(|op| op.id != self.coordinator_index)
    }

    /// Returns the operator at the given index.
    pub fn get_operator_by_id(&self, id: usize) -> Option<&Operator<S>> {
        self.operators.get(id)
    }

    /// Returns the operator at the given identifier.
    pub fn get_operator_by_identifier(&self, identifier: &Identifier) -> Option<&Operator<S>> {
        self.operators
            .iter()
            .find(|op| &op.identifier == identifier)
    }

    pub fn is_empty(&self) -> bool {
        self.operators.is_empty()
    }

    pub fn len(&self) -> usize {
        self.operators.len()
    }
}
