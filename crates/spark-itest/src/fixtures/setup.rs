use std::path::PathBuf;

use anyhow::Result;
use rand::Rng;
use spark_wallet::{
    DefaultSigner, LeafOptimizationOptions, Network, OperatorConfig, OperatorPoolConfig, PublicKey,
    RetryConfig, ServiceProviderConfig, SparkWalletConfig, TokenOutputsOptimizationOptions,
};
use tokio::sync::OnceCell;
use tracing::info;

use crate::fixtures::{
    bitcoind::BitcoindFixture,
    spark_so::SparkSoFixture,
    sspd::{LdkSettings, SspdFixture},
    state_snapshot,
};

/// Fixed because the state snapshot's leaf pool is stored against the identity
/// this seed derives.
pub const SSPD_WALLET_SEED_HEX: &str =
    "0505050505050505050505050505050505050505050505050505050505050505";

pub struct TestFixtures {
    pub fixture_id: FixtureId,
    pub bitcoind: BitcoindFixture,
    pub spark_so: SparkSoFixture,
    sspd: OnceCell<SspdFixture>,
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

    /// This cluster's directory for the files its containers bind-mount. Keyed by
    /// the fixture id alone: a test-scoped `testdir!()` is named after the running
    /// thread, which concurrent tests can share.
    pub fn testdir(&self) -> std::io::Result<PathBuf> {
        let dir = testdir::testdir!(ModuleScope).join(&self.0);
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }
}

impl std::fmt::Display for FixtureId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Together two full pools' worth: the daemon spends its on-chain funds on its
/// leaf pool and on fronting coop-exit withdrawals.
const SSPD_ONCHAIN_UTXO_SATS: u64 = 1_000_000;
const SSPD_ONCHAIN_UTXO_COUNT: usize =
    2 * (crate::fixtures::sspd::FULL_POOL_ONCHAIN_SATS / 1_000_000) as usize;

impl TestFixtures {
    pub async fn new() -> Result<Self> {
        let fixture_id = FixtureId::new();

        let snapshot = state_snapshot::is_current();
        let bitcoind = if snapshot {
            let mut bitcoind =
                BitcoindFixture::restored(&fixture_id, &state_snapshot::bitcoind_datadir()).await?;
            bitcoind.adopt_restored_wallet().await?;
            bitcoind
        } else {
            let mut bitcoind = BitcoindFixture::new(&fixture_id).await?;
            bitcoind.initialize().await?;
            bitcoind
        };

        // Create the SparkSoFixture with the docker_ref and bitcoind connection
        let mut spark_so = SparkSoFixture::new(&fixture_id, &bitcoind).await?;
        spark_so.initialize().await?;

        info!("All test fixtures initialized");

        Ok(Self {
            fixture_id,
            bitcoind,
            spark_so,
            sspd: OnceCell::new(),
        })
    }

    /// Started and funded on first use, without a lightning node.
    pub async fn sspd(&self) -> Result<&SspdFixture> {
        self.sspd_with_ldk(None).await
    }

    /// This cluster's daemon, started with `ldk` if this call is what starts it.
    pub async fn sspd_with_ldk(&self, ldk: Option<&LdkSettings>) -> Result<&SspdFixture> {
        self.sspd
            .get_or_try_init(|| async {
                let sspd = SspdFixture::start(
                    &self.fixture_id,
                    &self.bitcoind,
                    &self.spark_so.operators,
                    SSPD_WALLET_SEED_HEX,
                    ldk,
                )
                .await?;
                sspd.fund_onchain(
                    &self.bitcoind,
                    SSPD_ONCHAIN_UTXO_SATS,
                    SSPD_ONCHAIN_UTXO_COUNT,
                )
                .await?;
                Ok(sspd)
            })
            .await
    }

    /// Takes all operators offline by stopping their containers; bitcoind stays up.
    /// See [`SparkSoFixture::stop_operators`].
    pub async fn stop_operators(&self) -> Result<()> {
        self.spark_so.stop_operators().await
    }

    pub async fn create_wallet_config(&self) -> Result<SparkWalletConfig> {
        self.create_wallet_config_with_ssp(None).await
    }

    pub async fn create_wallet_config_with_ssp(
        &self,
        ssp_config: Option<ServiceProviderConfig>,
    ) -> Result<SparkWalletConfig> {
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

        let service_provider_config = match ssp_config {
            Some(config) => config,
            None => ServiceProviderConfig {
                base_url: "".to_string(),
                schema_endpoint: None,
                identity_public_key: PublicKey::from_slice(&[2; 33])?,
                user_agent: Some("spark-wallet-itest/0.1.0".to_string()),
                retry_config: RetryConfig::default(),
            },
        };

        Ok(SparkWalletConfig {
            network: Network::Regtest,
            operator_pool: OperatorPoolConfig::new(0, operator_configs)?,
            split_secret_threshold: crate::fixtures::spark_so::MIN_SIGNERS as u32,
            reconnect_interval_seconds: 1,
            service_provider_config,
            tokens_config: SparkWalletConfig::default_tokens_config(),
            leaf_optimization_options: LeafOptimizationOptions::default(),
            leaf_auto_optimize_enabled: false,
            token_outputs_optimization_options: TokenOutputsOptimizationOptions {
                min_outputs_threshold: 50,
                target_output_count: 5,
                auto_optimize_interval: None,
            },
            self_payment_allowed: false,
            max_concurrent_claims: 1,
        })
    }
}

// Helper function to create a test signer
pub fn create_test_signer_alice() -> DefaultSigner {
    create_random_test_signer()
}

pub fn create_test_signer_bob() -> DefaultSigner {
    create_random_test_signer()
}

fn create_random_test_signer() -> DefaultSigner {
    let mut seed = [0u8; 32];
    rand::thread_rng().fill(&mut seed);
    DefaultSigner::new(&seed, spark_wallet::Network::Regtest).unwrap()
}
