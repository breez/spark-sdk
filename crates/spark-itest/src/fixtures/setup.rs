use anyhow::Result;
use rand::Rng;
use spark_wallet::{
    DefaultSigner, LeafOptimizationOptions, Network, OperatorConfig, OperatorPoolConfig, PublicKey,
    ServiceProviderConfig, SparkWalletConfig, TokenOutputsOptimizationOptions,
};
use tracing::info;

use crate::fixtures::{bitcoind::BitcoindFixture, spark_so::SparkSoFixture};

pub struct TestFixtures {
    pub bitcoind: BitcoindFixture,
    pub spark_so: SparkSoFixture,
}

#[derive(Clone, Debug)]
pub struct FixtureId(String);

impl Default for FixtureId {
    fn default() -> Self {
        Self::new()
    }
}

impl FixtureId {
    pub fn new() -> Self {
        let id: u32 = rand::thread_rng().gen_range(0..0xFFFFFFFF);
        Self(hex::encode(id.to_le_bytes()))
    }

    pub fn to_network(&self) -> String {
        format!("network-{}", self.0)
    }
}

impl std::fmt::Display for FixtureId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TestFixtures {
    pub async fn new() -> Result<Self> {
        let fixture_id = FixtureId::new();

        // Initialize bitcoind
        let mut bitcoind = BitcoindFixture::new(&fixture_id).await?;
        bitcoind.initialize().await?;

        // Create the SparkSoFixture with the docker_ref and bitcoind connection
        let mut spark_so = SparkSoFixture::new(&fixture_id, &bitcoind).await?;
        spark_so.initialize().await?;

        info!("All test fixtures initialized");

        Ok(Self { bitcoind, spark_so })
    }

    pub async fn create_wallet_config(&self) -> Result<SparkWalletConfig> {
        // Create a wallet configuration that points to our service operators
        let mut operator_configs = Vec::new();

        for operator in &self.spark_so.operators {
            operator_configs.push(OperatorConfig {
                address: format!("https://127.0.0.1:{}", operator.host_port).parse()?,
                ca_cert: Some(operator.ca_cert.as_bytes().to_vec()),
                id: operator.index,
                identifier: operator.identifier,
                identity_public_key: operator.public_key,
                user_agent: None,
            });
        }

        Ok(SparkWalletConfig {
            network: Network::Regtest,
            operator_pool: OperatorPoolConfig::new(0, operator_configs)?,
            split_secret_threshold: crate::fixtures::spark_so::MIN_SIGNERS as u32,
            reconnect_interval_seconds: 1,
            service_provider_config: ServiceProviderConfig {
                base_url: "".to_string(),
                schema_endpoint: None,
                identity_public_key: PublicKey::from_slice(&[2; 33])?,
                user_agent: Some("spark-wallet-itest/0.1.0".to_string()),
            },
            tokens_config: SparkWalletConfig::default_tokens_config(),
            leaf_optimization_options: LeafOptimizationOptions::default(),
            leaf_auto_optimize_enabled: false,
            token_outputs_optimization_options: TokenOutputsOptimizationOptions {
                min_outputs_threshold: 50,
                auto_optimize_interval: None,
            },
            self_payment_allowed: false,
        })
    }
}

// Helper function to create a test signer
pub fn create_test_signer_alice() -> DefaultSigner {
    // Use deterministic seed for testing
    let seed = [3u8; 32];
    DefaultSigner::new(&seed, spark_wallet::Network::Regtest).unwrap()
}

pub fn create_test_signer_bob() -> DefaultSigner {
    // Use deterministic seed for testing
    let seed = [4u8; 32];
    DefaultSigner::new(&seed, spark_wallet::Network::Regtest).unwrap()
}
