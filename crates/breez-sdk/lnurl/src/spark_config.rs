//! The Spark deployment this server issues invoices through, read from a file.
//! Either the JSON a deployment publishes, which is the SDK's `SparkConfig`, so
//! one file configures the wallets and the server that serves their addresses,
//! or a wallet config a caller serialized itself.

use std::{fs, path::Path};

use anyhow::{Context, anyhow};
use serde::Deserialize;
use spark_wallet::{Network, OperatorPoolConfig, SparkWalletConfig};

#[derive(Deserialize)]
#[serde(untagged)]
enum SparkConfigFile {
    Deployment(Deployment),
    Wallet(Box<SparkWalletConfig>),
}

#[derive(Deserialize)]
struct Deployment {
    coordinator_identifier: String,
    threshold: u32,
    signing_operators: Vec<Operator>,
    ssp_config: Ssp,
    expected_withdraw_bond_sats: u64,
    expected_withdraw_relative_block_locktime: u64,
    max_token_transaction_inputs: Option<u32>,
}

#[derive(Deserialize)]
struct Operator {
    id: u32,
    identifier: String,
    address: String,
    identity_public_key: String,
    ca_cert_pem: Option<String>,
}

#[derive(Deserialize)]
struct Ssp {
    base_url: String,
    identity_public_key: String,
    schema_endpoint: Option<String>,
}

pub fn load(network: Network, path: &Path) -> Result<SparkWalletConfig, anyhow::Error> {
    let json = fs::read_to_string(path)
        .with_context(|| format!("failed to read spark config {}", path.display()))?;
    let config: SparkConfigFile = serde_json::from_str(&json)
        .with_context(|| format!("failed to parse spark config {}", path.display()))?;
    match config {
        SparkConfigFile::Deployment(deployment) => build(network, &deployment),
        SparkConfigFile::Wallet(config) => Ok(*config),
    }
}

fn build(network: Network, deployment: &Deployment) -> Result<SparkWalletConfig, anyhow::Error> {
    let coordinator_index = deployment
        .signing_operators
        .iter()
        .position(|operator| operator.identifier == deployment.coordinator_identifier)
        .ok_or_else(|| anyhow!("coordinator_identifier does not match any signing operator"))?;

    let operators = deployment
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
            .map_err(|e| anyhow!("invalid signing operator {}: {e}", operator.id))
        })
        .collect::<Result<_, _>>()?;

    let mut config = SparkWalletConfig::default_config(network);
    config.operator_pool = OperatorPoolConfig::new(coordinator_index, operators)
        .map_err(|e| anyhow!("invalid operator pool: {e}"))?;
    config.split_secret_threshold = deployment.threshold;
    config.service_provider_config = SparkWalletConfig::create_service_provider_config(
        &deployment.ssp_config.base_url,
        &deployment.ssp_config.identity_public_key,
        deployment.ssp_config.schema_endpoint.clone(),
    )
    .map_err(|e| anyhow!("invalid ssp config: {e}"))?;
    config.tokens_config.expected_withdraw_bond_sats = deployment.expected_withdraw_bond_sats;
    config
        .tokens_config
        .expected_withdraw_relative_block_locktime =
        deployment.expected_withdraw_relative_block_locktime;
    if let Some(max_tx_inputs) = deployment.max_token_transaction_inputs {
        config.tokens_config.max_tx_inputs = max_tx_inputs as usize;
    }
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"{
      "coordinator_identifier": "0000000000000000000000000000000000000000000000000000000000000001",
      "threshold": 2,
      "signing_operators": [
        {
          "id": 0,
          "identifier": "0000000000000000000000000000000000000000000000000000000000000001",
          "address": "https://127.0.0.1:8535",
          "identity_public_key": "031b84c5567b126440995d3ed5aaba0565d71e1834604819ff9c17f5e9d5dd078f"
        },
        {
          "id": 1,
          "identifier": "0000000000000000000000000000000000000000000000000000000000000002",
          "address": "https://127.0.0.1:8536",
          "identity_public_key": "024d4b6cd1361032ca9bd2aeb9d900aa4d45d9ead80ac9423374c451a7254d0766"
        }
      ],
      "ssp_config": {
        "base_url": "http://127.0.0.1:59049",
        "identity_public_key": "03e7343c4fdcffdce7b0041c4f482948a1058f8fe8d5a48e0a0c4e884d0e7cc124",
        "schema_endpoint": "graphql/spark/rc"
      },
      "expected_withdraw_bond_sats": 10000,
      "expected_withdraw_relative_block_locktime": 1000
    }"#;

    #[test]
    fn builds_a_wallet_config_for_the_deployment() {
        let SparkConfigFile::Deployment(deployment) = serde_json::from_str(CONFIG).unwrap() else {
            panic!("expected a deployment");
        };
        let config = build(Network::Regtest, &deployment).unwrap();

        assert_eq!(config.operator_pool.get_all_operators().count(), 2);
        assert_eq!(
            config.operator_pool.get_coordinator().address,
            "https://127.0.0.1:8535"
        );
        assert_eq!(
            config.service_provider_config.base_url,
            "http://127.0.0.1:59049"
        );
        assert_eq!(config.split_secret_threshold, 2);
    }

    #[test]
    fn rejects_a_coordinator_that_is_not_an_operator() {
        let json = CONFIG.replacen(
            "\"coordinator_identifier\": \"0000000000000000000000000000000000000000000000000000000000000001\"",
            "\"coordinator_identifier\": \"00000000000000000000000000000000000000000000000000000000000000ff\"",
            1,
        );
        let SparkConfigFile::Deployment(deployment) = serde_json::from_str(&json).unwrap() else {
            panic!("expected a deployment");
        };
        assert!(build(Network::Regtest, &deployment).is_err());
    }

    #[test]
    fn reads_a_serialized_wallet_config() {
        let wallet = SparkWalletConfig::default_config(Network::Regtest);
        let json = serde_json::to_string(&wallet).unwrap();
        let SparkConfigFile::Wallet(read) = serde_json::from_str(&json).unwrap() else {
            panic!("expected a wallet config");
        };
        assert_eq!(
            read.service_provider_config.base_url,
            wallet.service_provider_config.base_url
        );
    }
}
