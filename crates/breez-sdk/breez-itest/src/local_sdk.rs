//! Helpers for spinning up a `BreezSdk` against the local spark-itest
//! operator pool + bitcoind regtest container, with a pluggable signer backend.
//!
//! Unlike the faucet-based `signer_backends` suite, this harness runs against
//! local operators and a controllable bitcoind, which the unilateral-exit flow
//! needs (mining, CSV maturity, package broadcast).

use std::sync::Arc;

use anyhow::Result;
use breez_sdk_spark::{
    BreezSdk, Network, SdkBuilder, Seed, SparkConfig, SparkSigningOperator, SparkSspConfig,
    default_config, default_server_config,
};
use spark_itest::fixtures::setup::TestFixtures;
use spark_wallet::{DefaultSigner, SparkSignerAdapter, SparkWallet, WalletEvent};
use tempfile::TempDir;
use tokio::sync::mpsc;
use tracing::debug;

use crate::chain_service::LocalBitcoindChainService;
use crate::helpers::regtest::SignerBackend;

/// A `BreezSdk` connected to local fixtures, plus a side-channel `SparkWallet`
/// seeded with the same identity for reaching spark-wallet APIs (e.g. deposit
/// claim) not exposed or usable through the public BreezSdk surface locally.
pub struct LocalSdk {
    pub sdk: BreezSdk,
    /// Same identity as the wallet wrapped by `sdk`; both see the same leaves.
    pub spark_wallet: SparkWallet,
    pub events: mpsc::Receiver<breez_sdk_spark::SdkEvent>,
    pub fixtures: Arc<TestFixtures>,
    /// The entropy this wallet's identity derives from, kept so the same wallet
    /// can be rebuilt on an empty store. `None` under Turnkey, whose identity
    /// lives remotely and cannot be re-derived here.
    seed: Option<Vec<u8>>,
    #[allow(dead_code)]
    storage_dir: TempDir,
    /// Deletes the throwaway Turnkey wallet on drop (Turnkey backend only).
    #[cfg(feature = "turnkey")]
    #[allow(dead_code)]
    turnkey_guard: Option<crate::turnkey::TurnkeyWalletGuard>,
}

/// Build a `BreezSdk` pointing at the spark-itest operator pool and a
/// [`LocalBitcoindChainService`], signing with `backend`. A fresh identity is
/// used per call so instances don't collide.
pub async fn build_local_sdk(
    fixtures: Arc<TestFixtures>,
    backend: SignerBackend,
) -> Result<LocalSdk> {
    build_local_sdk_inner(fixtures, backend, None, false).await
}

/// The same wallet as `source`, on an empty store: same identity, so the operators
/// hold the same leaves for it, but nothing persisted locally. Stands in for
/// restoring onto a second device. Seed backend only, since a Turnkey identity is
/// provisioned remotely and cannot be re-derived here.
///
/// Built in server mode, so it runs no background sync and its store stays empty
/// until something writes to it explicitly. That is what lets a test attribute
/// what it holds to an import rather than to the operators.
pub async fn rebuild_on_empty_storage(source: &LocalSdk) -> Result<LocalSdk> {
    let seed = source
        .seed
        .clone()
        .ok_or_else(|| anyhow::anyhow!("this backend's identity cannot be re-derived"))?;
    build_local_sdk_inner(
        Arc::clone(&source.fixtures),
        SignerBackend::Seed,
        Some(seed),
        true,
    )
    .await
}

async fn build_local_sdk_inner(
    fixtures: Arc<TestFixtures>,
    backend: SignerBackend,
    entropy: Option<Vec<u8>>,
    server_mode: bool,
) -> Result<LocalSdk> {
    let wallet_config = fixtures.create_wallet_config().await?;

    let signing_operators: Vec<SparkSigningOperator> = wallet_config
        .operator_pool
        .get_all_operators()
        .map(|op| SparkSigningOperator {
            id: op.id as u32,
            identifier: hex::encode(op.identifier.serialize()),
            address: op.address.clone(),
            identity_public_key: hex::encode(op.identity_public_key.serialize()),
            ca_cert_pem: op
                .ca_cert
                .as_ref()
                .and_then(|b| String::from_utf8(b.clone()).ok()),
        })
        .collect();
    let coordinator = wallet_config.operator_pool.get_coordinator();
    let coordinator_identifier = hex::encode(coordinator.identifier.serialize());

    // Server mode runs no background sync, so the store only ever holds what
    // something wrote to it explicitly.
    let mut config = if server_mode {
        default_server_config(Network::Regtest)
    } else {
        default_config(Network::Regtest)
    };
    config.api_key = None;
    config.lnurl_domain = None;
    config.real_time_sync_server_url = None;
    config.sync_interval_secs = 5;
    // Disable auto-optimization so deposited leaves aren't split/consolidated
    // behind the test's back.
    config.leaf_optimization_config.auto_enabled = false;
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
        max_token_transaction_inputs: None,
    });

    let storage_dir = tempfile::tempdir()?;
    let storage_path = storage_dir.path().to_string_lossy().into_owned();

    let chain_service: Arc<dyn breez_sdk_spark::BitcoinChainService> =
        Arc::new(LocalBitcoindChainService::new(&fixtures.bitcoind));

    #[cfg(feature = "turnkey")]
    let mut turnkey_guard: Option<crate::turnkey::TurnkeyWalletGuard> = None;

    let (sdk, spark_wallet, carried_seed) = match backend {
        SignerBackend::Seed => {
            let mut seed = entropy.unwrap_or_else(|| {
                let mut fresh = [0u8; 32].to_vec();
                rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut fresh);
                fresh
            });
            seed.resize(32, 0);
            let seed: [u8; 32] = seed.try_into().expect("resized to 32 bytes");
            let sdk = SdkBuilder::new(config, Seed::Entropy(seed.to_vec()))
                .with_chain_service(chain_service)
                .with_default_storage(storage_path)
                .build()
                .await?;
            // `DefaultSigner::new` and `SdkBuilder::new(Seed::Entropy)` derive
            // the same identity key, so both wallets see the same leaves.
            let signer = Arc::new(DefaultSigner::new(&seed, spark_wallet::Network::Regtest)?);
            let spark_signer = Arc::new(SparkSignerAdapter::new(signer));
            let spark_wallet = SparkWallet::connect(wallet_config, spark_signer).await?;
            (sdk, spark_wallet, Some(seed.to_vec()))
        }
        #[cfg(feature = "turnkey")]
        SignerBackend::Turnkey => {
            use breez_sdk_spark::signer::ExternalSparkSignerAdapter;
            use breez_sdk_spark::turnkey::create_turnkey_signer;

            let (turnkey_config, guard) = crate::turnkey::provision_turnkey_wallet().await?;
            let signers = create_turnkey_signer(turnkey_config)
                .await
                .map_err(|e| anyhow::anyhow!("create_turnkey_signer failed: {e}"))?;

            let sdk = SdkBuilder::new_with_signer(
                config,
                signers.breez_signer,
                Arc::clone(&signers.spark_signer),
            )
            .with_chain_service(chain_service)
            .with_default_storage(storage_path)
            .build()
            .await?;

            let spark_signer = Arc::new(ExternalSparkSignerAdapter::new(signers.spark_signer));
            let spark_wallet = SparkWallet::connect(wallet_config, spark_signer).await?;
            turnkey_guard = Some(guard);
            (sdk, spark_wallet, None)
        }
    };

    let (tx, events) = mpsc::channel(100);
    sdk.add_event_listener(Box::new(ChannelEventListener { tx }))
        .await;

    // Drive the side-channel wallet's own sync. Subscribe first, then start
    // background processing: the first `Synced` is dropped if no receiver is
    // attached yet (see `SparkWallet::start_background_processing`). Bounded so
    // a future regression fails fast instead of hanging.
    // The side-channel wallet is there to deposit and claim. A server-mode wallet
    // does neither, and syncing it would fetch the very leaves a test may be
    // trying to attribute to something else.
    if !server_mode {
        let mut wallet_events = spark_wallet.subscribe_events();
        spark_wallet.start_background_processing().await;
        tokio::time::timeout(std::time::Duration::from_secs(90), async {
            loop {
                if matches!(wallet_events.recv().await?, WalletEvent::Synced) {
                    break;
                }
            }
            Ok::<_, anyhow::Error>(())
        })
        .await
        .map_err(|_| anyhow::anyhow!("side-channel SparkWallet did not sync within 90s"))??;
    }

    debug!("local BreezSdk + side-channel SparkWallet built ({backend:?})");

    Ok(LocalSdk {
        sdk,
        spark_wallet,
        events,
        fixtures,
        seed: carried_seed,
        storage_dir,
        #[cfg(feature = "turnkey")]
        turnkey_guard,
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
