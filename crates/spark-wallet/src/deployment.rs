//! The JSON a Spark deployment publishes: which operators sign for it, which
//! service provider serves it, and the terms its token withdrawals take. One
//! file configures every wallet that connects to that deployment.

use serde::{Deserialize, Serialize};

use crate::{Network, OperatorPoolConfig, SparkWalletConfig, SparkWalletError};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SparkDeployment {
    /// Hex-encoded identifier of the operator that coordinates signing.
    pub coordinator_identifier: String,
    /// The FROST signing threshold, such as 2 of 3.
    pub threshold: u32,
    pub signing_operators: Vec<SparkDeploymentOperator>,
    pub ssp_config: SparkDeploymentSsp,
    pub expected_withdraw_bond_sats: u64,
    pub expected_withdraw_relative_block_locktime: u64,
    /// Cap on the inputs a single token transaction may spend. Unset keeps the
    /// wallet's own default.
    pub max_token_transaction_inputs: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SparkDeploymentOperator {
    pub id: u32,
    /// Hex-encoded 32-byte FROST identifier.
    pub identifier: String,
    pub address: String,
    pub identity_public_key: String,
    /// The certificate authority the operator's certificate is signed by, for a
    /// deployment that serves its own. Unset trusts the host's roots.
    pub ca_cert_pem: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SparkDeploymentSsp {
    pub base_url: String,
    pub identity_public_key: String,
    pub schema_endpoint: Option<String>,
}

impl SparkDeployment {
    /// The wallet configuration this deployment describes, over the wallet's
    /// defaults for `network`.
    pub fn wallet_config(&self, network: Network) -> Result<SparkWalletConfig, SparkWalletError> {
        let coordinator_index = self
            .signing_operators
            .iter()
            .position(|operator| operator.identifier == self.coordinator_identifier)
            .ok_or_else(|| {
                SparkWalletError::ValidationError(
                    "coordinator_identifier does not match any signing operator".to_string(),
                )
            })?;

        let operators = self
            .signing_operators
            .iter()
            .map(|operator| {
                SparkWalletConfig::create_operator_config(
                    operator.id as usize,
                    &operator.identifier,
                    &operator.address,
                    operator.ca_cert_pem.as_ref().map(String::as_bytes),
                    &operator.identity_public_key,
                )
            })
            .collect::<Result<_, _>>()?;

        let mut config = SparkWalletConfig::default_config(network);
        config.operator_pool = OperatorPoolConfig::new(coordinator_index, operators)
            .map_err(|e| SparkWalletError::ValidationError(e.to_string()))?;
        config.split_secret_threshold = self.threshold;
        config.service_provider_config = SparkWalletConfig::create_service_provider_config(
            &self.ssp_config.base_url,
            &self.ssp_config.identity_public_key,
            self.ssp_config.schema_endpoint.clone(),
        )?;
        config.tokens_config.expected_withdraw_bond_sats = self.expected_withdraw_bond_sats;
        config
            .tokens_config
            .expected_withdraw_relative_block_locktime =
            self.expected_withdraw_relative_block_locktime;
        if let Some(max_tx_inputs) = self.max_token_transaction_inputs {
            config.tokens_config.max_tx_inputs = max_tx_inputs as usize;
        }
        Ok(config)
    }
}
