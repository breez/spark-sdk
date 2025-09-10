use std::sync::Arc;

use bitcoin::secp256k1::PublicKey;
use frost_secp256k1_tr::Identifier;
use serde::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, serde_as};

use crate::{
    operator::{
        SessionManager,
        rpc::{ConnectionManager, OperatorRpcError, SparkRpcClient},
    },
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
    pub address: String,
    #[serde_as(as = "DisplayFromStr")]
    pub identity_public_key: PublicKey,
}

impl OperatorConfig {}

#[derive(Clone)]
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

impl<S: Signer> OperatorPool<S> {
    pub async fn connect(
        config: &OperatorPoolConfig,
        connection_manager: &ConnectionManager,
        session_manager: Arc<dyn SessionManager>,
        signer: Arc<S>,
    ) -> Result<Self, OperatorRpcError> {
        let mut operators = Vec::new();
        for operator in &config.operators {
            let transport = connection_manager.get_transport(operator).await?;
            let client = SparkRpcClient::new(
                transport,
                Arc::clone(&signer),
                operator.identity_public_key,
                session_manager.clone(),
            );
            operators.push(Operator {
                client,
                id: operator.id,
                identifier: operator.identifier,
                identity_public_key: operator.identity_public_key,
            });
        }

        Ok(Self {
            coordinator_index: config.coordinator_index,
            operators,
        })
    }
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
