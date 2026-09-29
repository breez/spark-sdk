use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use spark::signer::{Signer, SparkSigner, SparkSignerAdapter};
use spark::ssp::ServiceProvider;
use spark_wallet::{
    RetryConfig, ServiceProviderConfig, SparkWallet, SparkWalletConfig, WalletEvent,
};
use sspd_lib::lightning::ldk::LdkServerNode;
use sspd_lib::wakeup::Wakeup;

use crate::fixtures::ldk_server::{GRPC_PORT, LdkServerFixture};
use crate::fixtures::setup::{SSPD_WALLET_SEED_HEX, TestFixtures, create_test_signer_alice};
use crate::fixtures::sspd::{LdkSettings, SspdFixture, internal_api};

const POOL_TIMEOUT: Duration = Duration::from_secs(600);

const ONCHAIN_UTXO_SATS: u64 = 1_000_000;
const ONCHAIN_UTXO_COUNT: usize = 4;

pub struct LightningStack {
    pub fixtures: TestFixtures,
    pub sspd: SspdFixture,
    pub ssp_node: Arc<LdkServerNode>,
    pub counterparty: Arc<LdkServerNode>,
    pub alice: SparkWallet,
    pub alice_signer: Arc<dyn Signer>,
    pub alice_config: SparkWalletConfig,
    pub ssp_config: ServiceProviderConfig,
    /// Held so the containers stay up for as long as the stack.
    pub ldk: (LdkServerFixture, LdkServerFixture),
}

impl LightningStack {
    pub async fn start() -> Result<Self> {
        let fixtures = TestFixtures::new().await?;
        let (ssp_ldk, cp_ldk) = tokio::try_join!(
            LdkServerFixture::start(&fixtures.fixture_id, &fixtures.bitcoind, "ssp"),
            LdkServerFixture::start(&fixtures.fixture_id, &fixtures.bitcoind, "cp"),
        )?;

        let ssp_node = Arc::new(LdkServerNode::new(
            ssp_ldk.base_url.clone(),
            ssp_ldk.api_key.clone(),
            &ssp_ldk.cert_pem,
            Wakeup::new(),
            Wakeup::new(),
        )?);
        let counterparty = Arc::new(LdkServerNode::new(
            cp_ldk.base_url.clone(),
            cp_ldk.api_key.clone(),
            &cp_ldk.cert_pem,
            Wakeup::new(),
            Wakeup::new(),
        )?);

        let sspd = SspdFixture::start(
            &fixtures.fixture_id,
            &fixtures.bitcoind,
            &fixtures.spark_so.operators,
            SSPD_WALLET_SEED_HEX,
            Some(&LdkSettings {
                internal_url: format!("{}:{}", ssp_ldk.container_name, GRPC_PORT),
                api_key: ssp_ldk.api_key.clone(),
                cert_pem: ssp_ldk.cert_pem.clone(),
                invoice_signing_key_hex: ssp_ldk.node_secret_key_hex(),
            }),
        )
        .await?;
        sspd.fund_onchain(&fixtures.bitcoind, ONCHAIN_UTXO_SATS, ONCHAIN_UTXO_COUNT)
            .await?;
        sspd.wait_for_pool(&fixtures.bitcoind, 1, POOL_TIMEOUT)
            .await
            .context("waiting for the daemon to stock its pool")?;

        let ssp_config = ServiceProviderConfig {
            base_url: sspd.base_url.clone(),
            schema_endpoint: Some("graphql/spark/rc".to_string()),
            identity_public_key: sspd.identity_public_key,
            user_agent: Some("spark-itest/0.1.0".to_string()),
            retry_config: RetryConfig::default(),
        };
        let alice_config = fixtures
            .create_wallet_config_with_ssp(Some(ssp_config.clone()))
            .await?;
        let alice_signer: Arc<dyn Signer> = Arc::new(create_test_signer_alice());
        let alice_spark_signer: Arc<dyn SparkSigner> =
            Arc::new(SparkSignerAdapter::new(alice_signer.clone()));
        let alice = SparkWallet::connect(alice_config.clone(), alice_spark_signer).await?;
        let mut events = alice.subscribe_events();
        alice.start_background_processing().await;
        loop {
            if matches!(events.recv().await?, WalletEvent::Synced) {
                break;
            }
        }

        Ok(Self {
            fixtures,
            sspd,
            ssp_node,
            counterparty,
            alice,
            alice_signer,
            alice_config,
            ssp_config,
            ldk: (ssp_ldk, cp_ldk),
        })
    }

    pub fn ssp_client(&self) -> Result<ServiceProvider> {
        let spark_signer: Arc<dyn SparkSigner> =
            Arc::new(SparkSignerAdapter::new(Arc::clone(&self.alice_signer)));
        Ok(ServiceProvider::new(
            self.ssp_config.clone(),
            spark_signer,
            Arc::new(spark::session_store::InMemorySessionStore::default()),
            None,
        )?)
    }

    pub async fn lightning_request(
        &self,
        id: &str,
    ) -> Result<internal_api::GetLightningRequestResponse> {
        self.sspd.lightning_request(id).await
    }

    #[must_use]
    pub fn counterparty_peer_address(&self) -> String {
        self.ldk.1.peer_address()
    }
}
