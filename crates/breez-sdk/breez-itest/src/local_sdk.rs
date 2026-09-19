//! Helpers for spinning up a `BreezSdk` against the local spark-itest
//! operator pool + bitcoind regtest container, with a pluggable signer backend.
//!
//! Unlike the faucet-based `signer_backends` suite, this harness runs against
//! local operators and a controllable bitcoind, which the unilateral-exit flow
//! needs (mining, CSV maturity, package broadcast).

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use breez_sdk_spark::{
    BitcoinChainService, BreezSdk, Config, GetInfoRequest, MaxFee, Network, SdkBuilder, Seed,
    SparkConfig, SparkSigningOperator, SparkSspConfig, SyncWalletRequest, default_config,
    default_external_signers, default_server_config,
};
use spark_itest::fixtures::setup::TestFixtures;
use spark_itest::fixtures::sspd::internal_api;
use spark_wallet::{
    DefaultSigner, RetryConfig, ServiceProviderConfig, SparkSignerAdapter, SparkWallet,
    SparkWalletConfig, WalletEvent,
};
use tempfile::TempDir;
use tokio::sync::mpsc;
use tracing::debug;

use crate::chain_service::LocalBitcoindChainService;
use crate::fixtures::lnurl::LnurlFixture;
use crate::helpers::regtest::SignerBackend;
use crate::{FaucetConfig, SdkInstance};

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

impl LocalSdk {
    /// The txid under which the SSP broadcast its spend of the deposit at
    /// `txid:vout`, if it has.
    pub async fn static_deposit_spend_txid(&self, txid: &str, vout: u32) -> Result<Option<String>> {
        let claim = self
            .fixtures
            .sspd()
            .await?
            .manager_client()
            .await?
            .get_static_deposit_claim(internal_api::GetStaticDepositClaimRequest {
                txid: txid.to_string(),
                vout,
            })
            .await?
            .into_inner()
            .claim;
        Ok(claim.and_then(|claim| claim.spend_broadcast_txid))
    }

    /// The SSP's most recent coop exits, newest first, up to its default listing
    /// limit.
    pub async fn coop_exit_records(&self) -> Result<Vec<internal_api::CoopExit>> {
        Ok(self
            .fixtures
            .sspd()
            .await?
            .manager_client()
            .await?
            .list_coop_exits(internal_api::ListCoopExitsRequest { limit: 0 })
            .await?
            .into_inner()
            .coop_exits)
    }
}

/// Build a `BreezSdk` pointing at the spark-itest operator pool and a
/// [`LocalBitcoindChainService`], signing with `backend`. Pass `seed` to pin the
/// wallet's identity (seed backend only); `None` derives a fresh identity per
/// call so instances don't collide.
pub async fn build_local_sdk(
    fixtures: Arc<TestFixtures>,
    backend: SignerBackend,
    seed: Option<[u8; 32]>,
) -> Result<LocalSdk> {
    build_local_sdk_inner(fixtures, backend, seed.map(|s| s.to_vec()), false, None).await
}

/// Like [`build_local_sdk`], with `configure` applied to the SDK `Config` last.
pub async fn build_local_sdk_with_config(
    fixtures: Arc<TestFixtures>,
    backend: SignerBackend,
    seed: Option<[u8; 32]>,
    configure: impl FnOnce(&mut Config) + Send + 'static,
) -> Result<LocalSdk> {
    build_local_sdk_inner(
        fixtures,
        backend,
        seed.map(|s| s.to_vec()),
        false,
        Some(Box::new(configure)),
    )
    .await
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
        None,
    )
    .await
}

#[allow(clippy::type_complexity)]
async fn build_local_sdk_inner(
    fixtures: Arc<TestFixtures>,
    backend: SignerBackend,
    entropy: Option<Vec<u8>>,
    server_mode: bool,
    configure: Option<Box<dyn FnOnce(&mut Config) + Send>>,
) -> Result<LocalSdk> {
    let wallet_config = fixtures.create_wallet_config().await?;

    let sspd = fixtures.sspd().await?;

    // Server mode runs no background sync, so the store only ever holds what
    // something wrote to it explicitly.
    let mut config = base_local_config(
        &wallet_config,
        &sspd.base_url,
        sspd.identity_public_key,
        server_mode,
    );
    // Disable auto-optimization so deposited leaves aren't split/consolidated
    // behind the test's back.
    config.leaf_optimization_config.auto_enabled = false;

    if let Some(configure) = configure {
        configure(&mut config);
    }

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

fn base_local_config(
    wallet_config: &SparkWalletConfig,
    ssp_base_url: &str,
    ssp_identity_public_key: bitcoin::secp256k1::PublicKey,
    server_mode: bool,
) -> Config {
    let ssp_config = SparkSspConfig {
        base_url: ssp_base_url.to_string(),
        identity_public_key: hex::encode(ssp_identity_public_key.serialize()),
        schema_endpoint: Some("graphql/spark/rc".to_string()),
    };

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

    let mut config = if server_mode {
        default_server_config(Network::Regtest)
    } else {
        default_config(Network::Regtest)
    };
    config.api_key = None;
    config.lnurl_domain = None;
    config.real_time_sync_server_url = None;
    // As the live builders do: a test body written for live expects an invoice
    // that carries a spark address, and a sender that takes it.
    config.prefer_spark_over_lightning = true;
    config.sync_interval_secs = 5;
    // The SSP quotes its spend cost plus a margin, which the SDK's default ceiling
    // (1 sat/vByte over a 99-vByte claim tx) refuses.
    config.max_deposit_claim_fee = Some(MaxFee::Rate { sat_per_vbyte: 4 });
    config.spark_config = Some(SparkConfig {
        coordinator_identifier,
        threshold: wallet_config.split_secret_threshold,
        signing_operators,
        ssp_config,
        expected_withdraw_bond_sats: wallet_config.tokens_config.expected_withdraw_bond_sats,
        expected_withdraw_relative_block_locktime: wallet_config
            .tokens_config
            .expected_withdraw_relative_block_locktime,
        max_token_transaction_inputs: None,
    });
    config
}

/// Operators, an sspd and a bitcoind of a test's own: one per test, since a stack
/// carried between tests makes a failure depend on what ran before it.
pub struct LocalStack {
    fixtures: Arc<TestFixtures>,
    /// Cached from [`TestFixtures::sspd`], which this stack has already started.
    ssp_base_url: String,
    /// Lightning payments between this stack's wallets settle as self-payments on
    /// this node, so it needs no channels.
    _ldk: spark_itest::fixtures::ldk_server::LdkServerFixture,
    /// Mines until the stack is dropped.
    driver: tokio::task::JoinHandle<()>,
}

impl Drop for LocalStack {
    fn drop(&mut self) {
        self.driver.abort();
    }
}

impl LocalStack {
    /// `http://host:port`; the SSP client and the `request_regtest_funds` faucet
    /// use `<base_url>/graphql/spark/rc`.
    pub fn ssp_base_url(&self) -> String {
        self.ssp_base_url.clone()
    }

    /// A wallet on this stack with a side-channel `SparkWallet` of the same
    /// identity, for a test that reaches spark-wallet APIs the SDK does not expose.
    pub async fn create_wallet_with_side_channel(&self) -> Result<LocalSdk> {
        build_local_sdk(Arc::clone(&self.fixtures), SignerBackend::Seed, None).await
    }

    /// The containers this stack runs, for a test that drives them directly.
    pub fn fixtures(&self) -> &Arc<TestFixtures> {
        &self.fixtures
    }

    /// Starts operators, an sspd and a bitcoind, and stocks the SSP's pool.
    pub async fn start() -> Result<Self> {
        let fixtures = Arc::new(TestFixtures::new().await?);

        let fixtures_for_ldk = Arc::clone(&fixtures);
        let ssp_ldk = tokio::task::spawn_blocking(move || -> Result<_> {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            rt.block_on(spark_itest::fixtures::ldk_server::LdkServerFixture::start(
                &fixtures_for_ldk.fixture_id,
                &fixtures_for_ldk.bitcoind,
                "ssp",
            ))
        })
        .await??;

        let sspd = fixtures
            .sspd_with_ldk(Some(&spark_itest::fixtures::sspd::LdkSettings {
                internal_url: format!(
                    "{}:{}",
                    ssp_ldk.container_name,
                    spark_itest::fixtures::ldk_server::GRPC_PORT
                ),
                api_key: ssp_ldk.api_key.clone(),
                cert_pem: ssp_ldk.cert_pem.clone(),
                invoice_signing_key_hex: ssp_ldk.node_secret_key_hex(),
            }))
            .await?;
        let ssp_base_url = sspd.base_url.clone();

        let driver = spawn_local_driver(Arc::clone(&fixtures));

        sspd.wait_for_pool(
            &fixtures.bitcoind,
            spark_itest::fixtures::sspd::LEAVES_PER_DENOMINATION,
            POOL_TIMEOUT,
        )
        .await?;

        Ok(Self {
            fixtures,
            ssp_base_url,
            _ldk: ssp_ldk,
            driver,
        })
    }

    /// An [`SdkInstance`] on this stack, with `configure` applied to the SDK
    /// `Config` last.
    pub async fn create_wallet(
        &self,
        identity: LocalIdentity,
        server_mode: bool,
        configure: impl FnOnce(&mut Config) + Send,
    ) -> Result<SdkInstance> {
        let stack = self;
        let wallet_config = stack.fixtures.create_wallet_config().await?;
        let sspd = stack.fixtures.sspd().await?;

        let mut config = base_local_config(
            &wallet_config,
            &sspd.base_url,
            sspd.identity_public_key,
            server_mode,
        );
        // Otherwise background optimization swaps leaves behind the test's back.
        config.leaf_optimization_config.auto_enabled = false;
        configure(&mut config);
        let background_tasks_enabled = config.background_tasks_enabled;

        let storage_dir = tempfile::tempdir()?;
        let storage_path = storage_dir.path().to_string_lossy().into_owned();
        let chain_service: Arc<dyn BitcoinChainService> =
            Arc::new(LocalBitcoindChainService::new(&stack.fixtures.bitcoind));

        let sdk = match identity {
            LocalIdentity::Seed(seed) => {
                SdkBuilder::new(config, Seed::Entropy(seed.to_vec()))
                    .with_chain_service(chain_service)
                    .with_default_storage(storage_path)
                    .build()
                    .await?
            }
            LocalIdentity::ExternalMnemonic(mnemonic) => {
                let signers = default_external_signers(mnemonic, None, Network::Regtest, None)?;
                SdkBuilder::new_with_signer(config, signers.breez_signer, signers.spark_signer)
                    .with_chain_service(chain_service)
                    .with_default_storage(storage_path)
                    .build()
                    .await?
            }
        };

        let (tx, events) = mpsc::channel(100);
        sdk.add_event_listener(Box::new(crate::helpers::ChannelEventListener { tx }))
            .await;

        // Without background tasks, `ensure_synced` is rejected: there is no background
        // sync to wait for.
        if background_tasks_enabled {
            let _ = sdk
                .get_info(GetInfoRequest {
                    ensure_synced: Some(true),
                })
                .await?;
        } else {
            sdk.sync_wallet(SyncWalletRequest {}).await?;
        }

        Ok(SdkInstance {
            sdk,
            events,
            span: tracing::Span::current(),
            temp_dir: Some(storage_dir),
            data_sync_fixture: None,
            lnurl_fixture: None,
            faucet: FaucetConfig::for_ssp(&stack.ssp_base_url),
            turnkey_guard: None,
        })
    }

    /// An LNURL server that issues invoices through this stack's SSP.
    pub async fn lnurl_server(&self) -> Result<LnurlFixture> {
        let sspd = self.fixtures.sspd().await?;
        let spark_config = self.fixtures.network_wallet_config(ServiceProviderConfig {
            base_url: sspd.network_base_url.clone(),
            schema_endpoint: Some("graphql/spark/rc".to_string()),
            identity_public_key: sspd.identity_public_key,
            user_agent: None,
            retry_config: RetryConfig::default(),
        })?;
        LnurlFixture::on_local_cluster(&self.fixtures.fixture_id.to_network(), &spark_config).await
    }
}

const POOL_TIMEOUT: Duration = Duration::from_secs(300);

/// Mines in the background: test bodies written for the live regtest never mine,
/// because it mines on its own.
fn spawn_local_driver(fixtures: Arc<TestFixtures>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            if let Err(e) = fixtures.bitcoind.generate_blocks(1).await {
                debug!("local driver: mining a block failed: {e}");
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    })
}

pub enum LocalIdentity {
    Seed([u8; 32]),
    ExternalMnemonic(String),
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
