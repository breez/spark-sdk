//! Helpers for spinning up a `BreezSdk` against the local spark-itest
//! operator pool + bitcoind regtest container.

use std::sync::Arc;

use anyhow::Result;
use breez_sdk_spark::{
    BreezSdk, Network, SdkBuilder, Seed, SparkConfig, SparkSigningOperator, SparkSspConfig,
    default_config,
};
use spark_itest::fixtures::setup::TestFixtures;
use spark_wallet::{DefaultSigner, SparkWallet, WalletEvent};
use tempdir::TempDir;
use tokio::sync::mpsc;
use tracing::debug;

use crate::chain_service::LocalBitcoindChainService;

/// A `BreezSdk` connected to local fixtures, plus a side-channel `SparkWallet`
/// seeded with the **same** identity so tests can reach spark-wallet APIs
/// (e.g. deposit claim) that the public BreezSdk surface either doesn't expose
/// or routes through services unavailable in the local fixture (e.g. SSP fee
/// quotes).
pub struct LocalSdk {
    pub sdk: BreezSdk,
    /// Separate `SparkWallet` with the same identity as the one wrapped by
    /// `sdk`; both see the same leaves through the Spark operators.
    pub spark_wallet: SparkWallet,
    pub events: mpsc::Receiver<breez_sdk_spark::SdkEvent>,
    pub fixtures: Arc<TestFixtures>,
    #[allow(dead_code)]
    pub storage_dir: TempDir,
}

/// Build a BreezSdk pointing at the spark-itest operator pool (through the
/// `with_spark_wallet_config` test-utils knob) and a `LocalBitcoindChainService`.
///
/// The seed parameter selects the wallet identity; tests usually want a random
/// seed per instance so multiple wallets don't collide in storage.
pub async fn build_local_sdk(fixtures: Arc<TestFixtures>, seed: [u8; 32]) -> Result<LocalSdk> {
    let wallet_config = fixtures.create_wallet_config().await?;

    let signing_operators: Vec<SparkSigningOperator> = wallet_config
        .operator_pool
        .get_all_operators()
        .map(|op| SparkSigningOperator {
            id: op.id as u32,
            identifier: hex::encode(op.identifier.serialize()),
            address: op.address.clone(),
            identity_public_key: hex::encode(op.identity_public_key.serialize()),
            ca_cert: op.ca_cert.clone(),
        })
        .collect();
    let coordinator = wallet_config.operator_pool.get_coordinator();
    let coordinator_identifier = hex::encode(coordinator.identifier.serialize());

    let mut config = default_config(Network::Regtest);
    config.api_key = None;
    config.lnurl_domain = None;
    config.real_time_sync_server_url = None;
    config.sync_interval_secs = 5;
    // Disable auto-optimization so deposited leaves aren't split/consolidated
    // behind the test's back.
    config.optimization_config.auto_enabled = false;
    config.spark_config = Some(SparkConfig {
        coordinator_identifier,
        threshold: wallet_config.split_secret_threshold,
        signing_operators,
        ssp_config: SparkSspConfig {
            base_url: wallet_config.service_provider_config.base_url.clone(),
            identity_public_key: hex::encode(
                wallet_config
                    .service_provider_config
                    .identity_public_key
                    .serialize(),
            ),
            schema_endpoint: wallet_config
                .service_provider_config
                .schema_endpoint
                .clone(),
        },
        expected_withdraw_bond_sats: wallet_config.tokens_config.expected_withdraw_bond_sats,
        expected_withdraw_relative_block_locktime: wallet_config
            .tokens_config
            .expected_withdraw_relative_block_locktime,
    });

    let storage_dir = TempDir::new("breez_sdk_local_test")?;
    let storage_path = storage_dir.path().to_string_lossy().into_owned();

    let chain_service: Arc<dyn breez_sdk_spark::BitcoinChainService> =
        Arc::new(LocalBitcoindChainService::new(Arc::clone(&fixtures)));

    let sdk = SdkBuilder::new(config, Seed::Entropy(seed.to_vec()))
        .with_chain_service(chain_service)
        .with_default_storage(storage_path)
        .build()
        .await?;

    let (tx, events) = mpsc::channel(100);
    sdk.add_event_listener(Box::new(ChannelEventListener { tx }))
        .await;

    // Side-channel SparkWallet with the same identity. `DefaultSigner::new`
    // uses `KeySetType::Default, use_address_index=false, account_number=None`,
    // which matches how `SdkBuilder::new` hands `Seed::Entropy` to the internal
    // signer, so both wallets derive the same identity key.
    let spark_signer = Arc::new(DefaultSigner::new(&seed, spark_wallet::Network::Regtest)?);
    let spark_wallet = SparkWallet::connect(wallet_config, spark_signer).await?;
    let mut wallet_events = spark_wallet.subscribe_events();
    loop {
        let event = wallet_events.recv().await?;
        if event == WalletEvent::Synced {
            break;
        }
    }

    debug!("local BreezSdk + side-channel SparkWallet built");

    Ok(LocalSdk {
        sdk,
        spark_wallet,
        events,
        fixtures,
        storage_dir,
    })
}

struct ChannelEventListener {
    tx: mpsc::Sender<breez_sdk_spark::SdkEvent>,
}

#[macros::async_trait]
impl breez_sdk_spark::EventListener for ChannelEventListener {
    async fn on_event(&self, event: breez_sdk_spark::SdkEvent) {
        let _ = self.tx.send(event).await;
    }
}
