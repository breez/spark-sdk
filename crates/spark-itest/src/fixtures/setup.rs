use std::path::PathBuf;

use anyhow::Result;
use rand::Rng;
use spark_wallet::{
    DefaultSigner, LeafOptimizationOptions, Network, OperatorConfig, OperatorPoolConfig, PublicKey,
    RetryConfig, ServiceProviderConfig, SparkWalletConfig, TokenOutputsOptimizationOptions,
};
use tracing::{Instrument, debug_span, info, instrument};

use crate::fixtures::{
    bitcoind::BitcoindFixture,
    database::DatabaseFixture,
    ldk_server::{GRPC_PORT, LdkServerFixture},
    spark_so::{OperatorFixture, SparkSoFixture},
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
    pub database: DatabaseFixture,
    pub spark_so: SparkSoFixture,
    sspd: Option<SspdFixture>,
    lightning: Vec<(&'static str, LdkServerFixture)>,
}

/// What a cluster runs, beyond the operators, bitcoind and Postgres every one
/// has. A test asks for what it needs before the cluster starts, so the daemon
/// and the lightning nodes come up while the operators finish starting.
#[derive(Default)]
pub struct ClusterBuilder {
    sspd: bool,
    lightning: Vec<&'static str>,
}

impl ClusterBuilder {
    /// Runs the daemon the wallets pay through.
    pub fn with_sspd(mut self) -> Self {
        self.sspd = true;
        self
    }

    /// Runs a lightning node per name, and gives the daemon the first of them.
    pub fn with_lightning(mut self, names: &[&'static str]) -> Self {
        self.lightning = names.to_vec();
        self.sspd = true;
        self
    }

    pub async fn build(self) -> Result<TestFixtures> {
        // Boxed: spans deepen the future type past the layout recursion limit.
        Box::pin(TestFixtures::start(self)).await
    }
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

/// Starts a lightning node per name, together.
async fn start_lightning(
    fixture_id: &FixtureId,
    bitcoind: &BitcoindFixture,
    names: &[&'static str],
) -> Result<Vec<(&'static str, LdkServerFixture)>> {
    let started = futures::future::try_join_all(
        names
            .iter()
            .map(|name| LdkServerFixture::start(fixture_id, bitcoind, name)),
    )
    .await?;
    Ok(names.iter().copied().zip(started).collect())
}

/// Starts the daemon, paying through `lightning` when the cluster runs one.
async fn start_sspd(
    fixture_id: &FixtureId,
    bitcoind: &BitcoindFixture,
    database: &DatabaseFixture,
    spark_so: &SparkSoFixture,
    lightning: Option<&(&'static str, LdkServerFixture)>,
) -> Result<SspdFixture> {
    let ldk = lightning.map(|(_, node)| LdkSettings {
        internal_url: format!("{}:{GRPC_PORT}", node.container_name),
        api_key: node.api_key.clone(),
        cert_pem: node.cert_pem.clone(),
        invoice_signing_key_hex: node.node_secret_key_hex(),
    });
    SspdFixture::start(
        fixture_id,
        bitcoind,
        &spark_so.operators,
        database,
        SSPD_WALLET_SEED_HEX,
        ldk.as_ref(),
    )
    .instrument(debug_span!("setup.sspd"))
    .await
}

impl TestFixtures {
    /// A cluster of operators, bitcoind and Postgres, and nothing else.
    pub async fn new() -> Result<Self> {
        Self::builder().build().await
    }

    pub fn builder() -> ClusterBuilder {
        ClusterBuilder::default()
    }

    #[instrument(level = "debug", name = "setup.fixtures", skip_all)]
    async fn start(builder: ClusterBuilder) -> Result<Self> {
        state_snapshot::check()?;
        let fixture_id = FixtureId::new();

        let (bitcoind, database) = tokio::try_join!(
            async {
                let mut bitcoind =
                    BitcoindFixture::restored(&fixture_id, &state_snapshot::bitcoind_datadir())
                        .await?;
                bitcoind
                    .adopt_restored_wallet()
                    .instrument(debug_span!("bitcoind.adopt_wallet"))
                    .await?;
                Ok::<_, anyhow::Error>(bitcoind)
            },
            DatabaseFixture::start(&fixture_id, true),
        )?;

        let mut spark_so = SparkSoFixture::new(&fixture_id, &bitcoind, &database).await?;
        // The daemon and the lightning nodes need the operators' addresses and
        // keys, not their readiness, so they start while the operators finish.
        let startup = spark_so.startup_wait();
        let ((), (lightning, sspd)) = tokio::try_join!(startup, async {
            let lightning = start_lightning(&fixture_id, &bitcoind, &builder.lightning).await?;
            let sspd = match builder.sspd {
                true => Some(
                    start_sspd(
                        &fixture_id,
                        &bitcoind,
                        &database,
                        &spark_so,
                        lightning.first(),
                    )
                    .await?,
                ),
                false => None,
            };
            Ok((lightning, sspd))
        })?;
        spark_so.finish_startup().await?;

        info!("All test fixtures initialized");

        Ok(Self {
            fixture_id,
            bitcoind,
            database,
            spark_so,
            sspd,
            lightning,
        })
    }

    /// The daemon this cluster runs.
    pub fn sspd(&self) -> &SspdFixture {
        self.sspd
            .as_ref()
            .expect("this cluster was built without a daemon: ask the builder for one")
    }

    /// The daemon, for a test that stops and starts it.
    pub fn sspd_mut(&mut self) -> &mut SspdFixture {
        self.sspd
            .as_mut()
            .expect("this cluster was built without a daemon: ask the builder for one")
    }

    /// The lightning node this cluster runs under `name`.
    pub fn lightning(&self, name: &str) -> &LdkServerFixture {
        self.lightning
            .iter()
            .find(|(node, _)| *node == name)
            .map(|(_, fixture)| fixture)
            .unwrap_or_else(|| panic!("this cluster runs no lightning node named {name}"))
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
        self.wallet_config(ssp_config, |operator| {
            format!("https://127.0.0.1:{}", operator.host_port)
        })
    }

    /// A wallet config for a client on the cluster's docker network, which reaches
    /// the operators by container name.
    pub fn network_wallet_config(
        &self,
        ssp_config: ServiceProviderConfig,
    ) -> Result<SparkWalletConfig> {
        self.wallet_config(Some(ssp_config), |operator| {
            format!("https://{}:{}", operator.host_name, operator.internal_port)
        })
    }

    fn wallet_config(
        &self,
        ssp_config: Option<ServiceProviderConfig>,
        address: impl Fn(&OperatorFixture) -> String,
    ) -> Result<SparkWalletConfig> {
        // Create a wallet configuration that points to our service operators
        let mut operator_configs = Vec::new();

        for operator in &self.spark_so.operators {
            operator_configs.push(OperatorConfig {
                address: address(operator).parse()?,
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
