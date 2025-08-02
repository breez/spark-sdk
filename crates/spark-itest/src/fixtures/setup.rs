use anyhow::Result;
use spark_wallet::{
    DefaultSigner, Network, OperatorConfig, OperatorPoolConfig, PublicKey, ServiceProviderConfig,
    SparkWalletConfig,
};
use tracing::info;

use crate::fixtures::{bitcoind::BitcoindFixture, spark_so::SparkSoFixture};

pub struct TestFixtures {
    pub bitcoind: BitcoindFixture,
    pub spark_so: SparkSoFixture,
}

impl TestFixtures {
    pub async fn new() -> Result<Self> {
        // Initialize bitcoind
        let mut bitcoind = BitcoindFixture::new().await?;
        bitcoind.initialize().await?;

        // Create the SparkSoFixture with the docker_ref and bitcoind connection
        let mut spark_so = SparkSoFixture::new(&bitcoind).await?;
        spark_so.initialize().await?;

        info!("All test fixtures initialized");

        Ok(Self { bitcoind, spark_so })
    }

    pub async fn create_wallet_config(&self) -> Result<SparkWalletConfig> {
        // Create a wallet configuration that points to our service operators
        let mut operator_configs = Vec::new();

        for operator in &self.spark_so.operators {
            operator_configs.push(OperatorConfig {
                address: format!("http://127.0.0.1:{}", operator.host_port).parse()?,
                id: operator.index,
                identifier: operator.identifier,
                identity_public_key: operator.public_key,
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
            },
        })
    }
}

// Helper function to create a test signer
pub fn create_test_signer() -> DefaultSigner {
    // Use deterministic seed for testing
    let seed = [3u8; 32];
    DefaultSigner::new(&seed, spark_wallet::Network::Regtest).unwrap()
}
