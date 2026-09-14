#![cfg_attr(
    not(test),
    warn(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::panic,
        clippy::string_slice,
        clippy::todo,
        clippy::unimplemented,
        clippy::unreachable,
        clippy::unwrap_used
    )
)]

use std::{path::PathBuf, sync::Arc, time::Duration};

use bitcoin::Network;
use bitcoind::BitcoindClient;
use chain::ChainMonitor;
use clap::{CommandFactory, FromArgMatches, Parser, parser::ValueSource};
use figment::{
    Figment,
    providers::{Env, Format, Serialized, Toml},
};
use internal_server::ssp_internal_api::onchain_wallet_server::OnchainWalletServer;
use internal_server::ssp_internal_api::pool_server::PoolServer;
use internal_server::ssp_internal_api::ssp_manager_server::SspManagerServer;
use serde::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, serde_as};
use sqlx::postgres::PgPoolOptions;
#[cfg(not(unix))]
use tokio::signal;
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tonic::transport::Server;
use tracing::{field, info, warn};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use sspd_lib::{
    auth, bitcoind, chain, coop_exit, fees, graphql, internal_server, leaves, lightning, pool,
    postgresql, shutdown, static_deposit, swap, wakeup, wallet,
};

/// How often held leaves are tried again. Arriving leaves do not wait for it.
const LEAF_ADMISSION_INTERVAL: Duration = Duration::from_secs(30);

const EXIT_CHAIN_RESOLVE_INTERVAL: Duration = Duration::from_secs(60);

const DEPENDENCY_RETRY_INTERVAL: Duration = Duration::from_secs(5);

#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize, Parser)]
#[command(version, about = "Spark Service Provider Daemon", long_about = None)]
struct Args {
    #[arg(long, default_value = "sspd.conf")]
    pub config: PathBuf,

    /// Address the public GraphQL API server will listen on.
    #[arg(long, default_value = "127.0.0.1:59049")]
    pub address: core::net::SocketAddr,

    /// Address the internal grpc server will listen on. That API has no
    /// authentication: whoever reaches it can stop the daemon and spend its
    /// on-chain funds restocking the pool, so keep it on a loopback address.
    #[arg(long, default_value = "127.0.0.1:59050")]
    pub internal_address: core::net::SocketAddr,

    /// Bitcoin network. Valid values are bitcoin, testnet, signet, regtest.
    #[arg(long, default_value = "bitcoin")]
    #[serde_as(as = "DisplayFromStr")]
    pub network: Network,

    /// Loglevel to use. Can be used to filter logs through the env filter
    /// format.
    #[arg(long, default_value = "info")]
    pub log_level: String,

    /// Connection string to the postgres database. Config file or `SSPD_DB_URL`
    /// only, since it can carry a password.
    #[arg(skip)]
    pub db_url: String,

    /// The most connections the daemon holds open to the postgres database, not
    /// counting the leaf store's own pool.
    #[arg(long, default_value = "10")]
    pub db_max_connections: u32,

    /// Address to the bitcoind rpc.
    #[arg(long, default_value = "http://localhost:8332")]
    pub bitcoind_rpc_address: String,

    /// Bitcoind rpc username.
    #[arg(long, default_value = "")]
    pub bitcoind_rpc_user: String,

    /// Bitcoind rpc password. Config file or `SSPD_BITCOIND_RPC_PASSWORD` only.
    #[arg(skip)]
    pub bitcoind_rpc_password: String,

    /// The longest the chain monitor waits for a new block before syncing again.
    #[arg(long, default_value = "60")]
    pub chain_poll_interval_seconds: u64,

    /// Apply the database migrations, the leaf store's included, at startup.
    #[arg(long)]
    pub auto_migrate: bool,

    /// Hex-encoded seed for the SSP wallet. Config file or `SSPD_WALLET_SEED` only:
    /// a command-line flag would show in the process list.
    #[arg(skip)]
    pub wallet_seed: String,

    /// Target number of Spark leaves to maintain per power-of-two denomination.
    #[arg(long, default_value = "50")]
    pub leaves_per_denomination: u32,

    /// Highest power of two for leaf denominations (e.g. 20 means up to 2^20 = 1,048,576 sats).
    #[arg(long, default_value = "20")]
    pub max_denomination_power: u32,

    /// Interval in seconds between pool replenishment checks. Blocks, changes to the
    /// pool and restock requests start one sooner.
    #[arg(long, default_value = "600")]
    pub replenish_interval_seconds: u64,

    /// How long, in seconds, the SSP's leaves stay fronted to a user who has not
    /// revealed a HODL preimage before the operators return them. Hold invoices
    /// demand enough CLTV to outlast it.
    #[arg(long, default_value = "1800")]
    pub receive_leaf_transfer_expiry_seconds: u64,

    /// Nodes the operators allow one tree-creation round to carry. A larger tree
    /// is built in layers, so this decides how many calls creating one takes.
    #[arg(long, default_value = "1000")]
    pub max_nodes_per_request: usize,

    /// ldk-server gRPC endpoint (host:port, no scheme). When empty, Lightning is
    /// disabled and the lightning operations report "not configured".
    #[arg(long, default_value = "")]
    pub ldk_server_url: String,

    /// API key for ldk-server HMAC authentication. Config file or
    /// `SSPD_LDK_SERVER_API_KEY` only.
    #[arg(skip)]
    pub ldk_server_api_key: String,

    /// Path to ldk-server's self-signed TLS certificate (PEM).
    #[arg(long, default_value = "")]
    pub ldk_server_cert_path: String,

    /// The ldk-server node's secret key (hex), to re-sign invoices that advertise a
    /// Spark address: ldk-server's hold invoice API takes no route hints. Config
    /// file or `SSPD_LDK_SERVER_INVOICE_SIGNING_KEY` only.
    #[arg(skip)]
    pub ldk_server_invoice_signing_key: String,

    /// The signing operators. Unset uses the network's default operators. Not a
    /// command-line flag: it is a list.
    #[arg(skip)]
    pub operators: Option<Vec<wallet::OperatorSetting>>,

    /// SSP flat base fee (sats) for fronting a lightning send. With the proportional
    /// fee, it is the least a user must commit beyond the amount. A route may charge
    /// up to what the user committed beyond the amount.
    #[arg(long, default_value = "2")]
    pub lightning_send_base_fee_sats: u64,

    /// Credit a static deposit through an instant claim before the deposit
    /// confirms. A deposit double-spent before it confirms leaves the SSP with
    /// nothing to collect for the leaves it credited.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub accept_unconfirmed_deposits: bool,

    /// SSP proportional fee (parts per million) for fronting a lightning send.
    /// Sized above a typical route so a retry down a dearer path still fits inside
    /// what the user committed.
    #[arg(long, default_value = "4000")]
    pub lightning_send_fee_ppm: u64,
}

#[tokio::main]
#[allow(clippy::too_many_lines)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (args, config_file) = load_args()?;
    if args.wallet_seed.is_empty() {
        return Err("wallet_seed is required, in the config file or SSPD_WALLET_SEED".into());
    }
    let seed =
        hex::decode(&args.wallet_seed).map_err(|e| format!("invalid wallet seed hex: {e}"))?;
    let pool_config = pool::config::PoolConfig {
        leaves_per_denomination: args.leaves_per_denomination,
        max_denomination_power: args.max_denomination_power,
        replenish_interval: Duration::from_secs(args.replenish_interval_seconds),
    };
    let lightning_config = LightningConfig::from_args(&args, pool_config.largest_denomination());

    tracing_subscriber::registry()
        .with(EnvFilter::new(&args.log_level))
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stdout))
        .init();

    if let Some(config_file) = &config_file {
        info!("starting sspd with config file: {}", config_file.display());
    } else {
        info!("starting sspd without config file");
    }

    let pgpool = Arc::new(
        PgPoolOptions::new()
            .max_connections(args.db_max_connections)
            .connect(&args.db_url)
            .await
            .map_err(|e| format!("failed to connect to postgres: {e:?}"))?,
    );
    if args.auto_migrate {
        postgresql::migrate(&pgpool).await?;
    }

    let chain_client = Arc::new(BitcoindClient::new(
        args.bitcoind_rpc_address,
        args.bitcoind_rpc_user,
        args.bitcoind_rpc_password,
    )?);

    let chain_repository = Arc::new(postgresql::ChainRepository::new(
        Arc::clone(&pgpool),
        args.network,
    ));

    let shutdown = shutdown::Shutdown::new();
    let token = shutdown.child();
    let tracker = TaskTracker::new();
    spawn_shutdown_signal_handler(Arc::clone(&shutdown));

    let replenish_wakeup = wakeup::Wakeup::new();
    let pooling_wakeup = wakeup::Wakeup::new();
    let coop_exit_wakeup = wakeup::Wakeup::new();
    let static_deposit_blocks = wakeup::Wakeup::new();
    let htlc_sweep_blocks = wakeup::Wakeup::new();
    let static_deposit_claims = wakeup::Wakeup::new();

    let chain_info = loop {
        match chain_client.chain_info().await {
            Ok(info) => break info,
            Err(e) => warn!("bitcoind is unavailable, retrying: {e}"),
        }
        tokio::select! {
            () = token.cancelled() => return Ok(()),
            () = tokio::time::sleep(DEPENDENCY_RETRY_INTERVAL) => {}
        }
    };
    if chain_info.network != args.network {
        return Err(format!(
            "bitcoind is on {}, but the daemon is configured for {}",
            chain_info.network, args.network
        )
        .into());
    }
    if chain_info.pruned {
        return Err("bitcoind is pruned, but the daemon reads every block since it started".into());
    }

    spawn_chain_monitor(
        &tracker,
        args.network,
        Duration::from_secs(args.chain_poll_interval_seconds),
        &chain_client,
        &chain_repository,
        vec![
            replenish_wakeup.clone(),
            pooling_wakeup.clone(),
            coop_exit_wakeup.clone(),
            static_deposit_blocks.clone(),
            htlc_sweep_blocks.clone(),
        ],
        &token,
    );

    let ssp_wallet = Arc::new(
        wallet::SspWallet::new(
            &seed,
            args.network,
            spark_postgres::PostgresStorageConfig {
                run_migration: args.auto_migrate,
                ..spark_postgres::PostgresStorageConfig::with_defaults(&args.db_url)
            },
            Arc::clone(&chain_repository),
            Arc::clone(&pgpool),
            args.operators.clone(),
            args.max_nodes_per_request,
        )
        .await
        .map_err(|e| format!("failed to initialize wallet: {e}"))?,
    );

    let pool_repo = Arc::new(postgresql::PoolRepository::new(Arc::clone(&pgpool)));
    let incoming: Arc<dyn leaves::IncomingLeafStore> = Arc::new(
        postgresql::PostgresIncomingLeafStore::new(Arc::clone(&pgpool)),
    );
    let admission = wakeup::Wakeup::new();
    let swap_repo = Arc::new(postgresql::SwapRepository::new(Arc::clone(&pgpool)));
    let lightning_store = Arc::new(postgresql::PostgresLightningStore::new(Arc::clone(&pgpool)))
        as Arc<dyn lightning::repository::LightningStore>;
    let coop_exit_store = Arc::new(postgresql::PostgresCoopExitStore::new(Arc::clone(&pgpool)))
        as Arc<dyn coop_exit::repository::CoopExitStore>;
    let fee_rates = Arc::new(chain::CachedFeeRates::new(
        Arc::clone(&chain_client) as Arc<_>
    )) as Arc<dyn fees::FeeRateSource>;
    let coop_exit_service = Arc::new(coop_exit::CoopExitService::new(
        Arc::clone(&coop_exit_store),
        Arc::clone(&ssp_wallet.spark.operator_pool),
        Arc::clone(&ssp_wallet.spark.signer),
        Arc::clone(&ssp_wallet.spark.transfer_service),
        Arc::clone(&ssp_wallet.onchain) as Arc<dyn coop_exit::CoopExitOnchainWallet>,
        Arc::clone(&fee_rates),
        ssp_wallet.spark.network,
        coop_exit_wakeup.clone(),
    ));
    let static_deposit_store = Arc::new(postgresql::PostgresStaticDepositClaimStore::new(
        Arc::clone(&pgpool),
    ))
        as Arc<dyn static_deposit::repository::StaticDepositClaimStore>;
    let instant_quote_store = Arc::new(postgresql::PostgresInstantStaticDepositQuoteStore::new(
        Arc::clone(&pgpool),
    ))
        as Arc<dyn static_deposit::repository::InstantStaticDepositQuoteStore>;
    let static_deposit_chain = Arc::new(BitcoindStaticDepositChain {
        chain_client: Arc::clone(&chain_client),
        chain_repository: Arc::clone(&chain_repository),
        onchain: Arc::clone(&ssp_wallet.onchain),
        network: args.network,
    }) as Arc<dyn static_deposit::StaticDepositChain>;
    let static_deposit_service = Arc::new(static_deposit::StaticDepositService::new(
        Arc::clone(&static_deposit_store),
        Arc::clone(&instant_quote_store),
        Arc::clone(&ssp_wallet.spark.transfer_service),
        Arc::clone(&ssp_wallet.spark.tree_store),
        Arc::clone(&ssp_wallet.spark.operator_pool),
        Arc::clone(&ssp_wallet.spark.signer),
        Arc::clone(&static_deposit_chain),
        Arc::clone(&fee_rates),
        Arc::clone(&pool_repo) as Arc<dyn leaves::LeafSigningKeys>,
        args.accept_unconfirmed_deposits,
        pool_config.largest_denomination(),
        static_deposit_claims.clone(),
    ));

    let restock = Arc::new(pool::restock::RestockService::new(replenish_wakeup.clone()));
    spawn_pool_loops(
        &tracker,
        &pool_config,
        &restock,
        &ssp_wallet,
        &chain_client,
        &chain_repository,
        &pool_repo,
        &fee_rates,
        replenish_wakeup,
        pooling_wakeup,
        &token,
    );
    let swap_claim_wakeup = wakeup::Wakeup::new();
    spawn_swap_claimer(
        &tracker,
        &incoming,
        &admission,
        &ssp_wallet,
        &swap_repo,
        &pool_repo,
        swap_claim_wakeup.clone(),
        &token,
    );
    spawn_coop_exit_worker(
        &tracker,
        &incoming,
        &admission,
        &ssp_wallet,
        &chain_client,
        &chain_repository,
        &coop_exit_service,
        &coop_exit_store,
        &pool_repo,
        &coop_exit_wakeup,
        &token,
    );
    spawn_static_deposit_worker(
        &tracker,
        &static_deposit_service,
        &static_deposit_store,
        &instant_quote_store,
        &static_deposit_chain,
        &ssp_wallet,
        static_deposit_blocks,
        static_deposit_claims,
        &token,
    );

    let htlc_sweep = lightning::htlc_sweep::HtlcSweepDeps {
        store: Arc::clone(&lightning_store),
        chain_client: Arc::clone(&chain_client) as Arc<dyn chain::ChainClient + Send + Sync>,
        fee_rates: Arc::clone(&fee_rates),
        signer: Arc::clone(&ssp_wallet.spark.signer),
        onchain: Arc::clone(&ssp_wallet.onchain),
        network: ssp_wallet.spark.network,
        blocks: htlc_sweep_blocks,
    };
    let htlc_sweep_token = token.clone();
    tracker.spawn(async move {
        lightning::htlc_sweep::run_htlc_sweep_loop(htlc_sweep, htlc_sweep_token).await;
    });

    let lightning = build_lightning(
        &incoming,
        &admission,
        &tracker,
        &lightning_config,
        &ssp_wallet,
        &pool_repo,
        &lightning_store,
        &token,
        &shutdown,
    )?;

    let auth = Arc::new(auth::AuthService::from_seed(&seed));
    spawn_servers(
        &tracker,
        args.address,
        args.internal_address,
        args.network,
        &ssp_wallet,
        &chain_client,
        &pool_repo,
        &swap_repo,
        swap_claim_wakeup,
        &lightning_store,
        &coop_exit_store,
        &static_deposit_store,
        &coop_exit_service,
        &static_deposit_service,
        lightning,
        auth,
        &fee_rates,
        &restock,
        &pool_config,
        &token,
        &shutdown,
    );

    spawn_exit_chain_resolver(&tracker, &ssp_wallet, &token);
    spawn_leaf_admission(&tracker, &ssp_wallet, &incoming, admission.clone(), &token);

    info!("sspd started");

    tracker.close();

    tracker.wait().await;
    if shutdown.is_failure() {
        return Err("sspd shut down after a subsystem failed".into());
    }
    info!("shutdown complete");
    Ok(())
}

/// Each source overrides the one before: defaults, the config file, `SSPD_*`
/// environment variables, flags given on the command line.
fn load_args() -> Result<(Args, Option<PathBuf>), Box<dyn std::error::Error>> {
    let command = Args::command();
    let flags: std::collections::HashSet<String> = command
        .get_arguments()
        .map(|arg| arg.get_id().to_string())
        .collect();
    let matches = command.get_matches();
    let cli = Args::from_arg_matches(&matches)?;
    let given =
        |id: &str| flags.contains(id) && matches.value_source(id) == Some(ValueSource::CommandLine);

    let config_file = std::fs::canonicalize(&cli.config).ok();
    if config_file.is_none() && given("config") {
        return Err(format!("config file {} cannot be read", cli.config.display()).into());
    }

    let mut explicit = serde_json::to_value(&cli)?;
    if let Some(fields) = explicit.as_object_mut() {
        fields.retain(|id, _| given(id));
    }

    let mut figment = Figment::new().merge(Serialized::defaults(&cli));
    if let Some(config_file) = &config_file {
        figment = figment.merge(Toml::file(config_file));
    }
    let args = figment
        .merge(Env::prefixed("SSPD_"))
        .merge(Serialized::defaults(explicit))
        .extract()?;
    Ok((args, config_file))
}

fn spawn_shutdown_signal_handler(shutdown: Arc<shutdown::Shutdown>) {
    tokio::spawn(async move {
        match wait_for_shutdown_signal().await {
            Ok(signal) => shutdown.stop(signal),
            Err(err) => shutdown.fail("shutdown signal handler", &err),
        }
    });
}

/// Docker, systemd and Kubernetes stop a process with SIGTERM by default, and
/// SIGTERM's default action ends the process without a graceful shutdown.
#[cfg(unix)]
async fn wait_for_shutdown_signal() -> std::io::Result<&'static str> {
    use tokio::signal::unix::{SignalKind, signal};

    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    Ok(tokio::select! {
        _ = terminate.recv() => "SIGTERM",
        _ = interrupt.recv() => "SIGINT",
    })
}

#[cfg(not(unix))]
async fn wait_for_shutdown_signal() -> std::io::Result<&'static str> {
    signal::ctrl_c().await?;
    Ok("ctrl-c")
}

#[allow(clippy::too_many_arguments)]
fn spawn_chain_monitor(
    tracker: &TaskTracker,
    network: Network,
    poll_interval: Duration,
    chain_client: &Arc<BitcoindClient>,
    chain_repository: &Arc<postgresql::ChainRepository>,
    chain_advanced: Vec<wakeup::Wakeup>,
    token: &CancellationToken,
) {
    let chain_monitor_token = token.child_token();
    let chain_monitor = Arc::new(ChainMonitor::new(
        network,
        Arc::clone(chain_client),
        Arc::clone(chain_repository),
        poll_interval,
        chain_advanced,
    ));
    tracker.spawn(async move {
        info!("Starting chain monitor");
        chain_monitor.start(chain_monitor_token).await;
    });
}

#[allow(clippy::too_many_arguments)]
fn spawn_pool_loops(
    tracker: &TaskTracker,
    pool_config: &pool::config::PoolConfig,
    restock: &Arc<pool::restock::RestockService>,
    ssp_wallet: &Arc<wallet::SspWallet<postgresql::ChainRepository>>,
    chain_client: &Arc<BitcoindClient>,
    chain_repository: &Arc<postgresql::ChainRepository>,
    pool_repo: &Arc<postgresql::PoolRepository>,
    fee_rates: &Arc<dyn fees::FeeRateSource>,
    replenish_wakeup: wakeup::Wakeup,
    pooling_wakeup: wakeup::Wakeup,
    token: &CancellationToken,
) {
    let replenish_wallet = Arc::clone(ssp_wallet);
    let replenish_client = Arc::clone(chain_client);
    let replenish_chain_repo = Arc::clone(chain_repository);
    let replenish_repo = Arc::clone(pool_repo);
    let replenish_fee_rates = Arc::clone(fee_rates);
    let replenish_config = pool_config.clone();
    let replenish_restock = Arc::clone(restock);
    let replenish_token = token.clone();
    tracker.spawn(async move {
        pool::replenish::run_replenish_loop(
            replenish_config,
            replenish_wallet,
            replenish_client,
            replenish_chain_repo,
            replenish_repo,
            replenish_fee_rates,
            replenish_restock,
            replenish_wakeup,
            replenish_token,
        )
        .await;
    });

    let pooling_wallet = Arc::clone(ssp_wallet);
    let pooling_chain_repo = Arc::clone(chain_repository);
    let pooling_repo = Arc::clone(pool_repo);
    let pooling_restock = Arc::clone(restock);
    let pooling_token = token.clone();
    tracker.spawn(async move {
        pool::tree_pooling::run_tree_pooling_loop(
            pooling_wallet,
            pooling_chain_repo,
            pooling_repo,
            pooling_restock,
            pooling_wakeup,
            pooling_token,
        )
        .await;
    });
}

#[allow(clippy::too_many_arguments)]
fn spawn_swap_claimer(
    tracker: &TaskTracker,
    incoming: &Arc<dyn leaves::IncomingLeafStore>,
    admission: &wakeup::Wakeup,
    ssp_wallet: &Arc<wallet::SspWallet<postgresql::ChainRepository>>,
    swap_repo: &Arc<postgresql::SwapRepository>,
    pool_repo: &Arc<postgresql::PoolRepository>,
    wakeup: wakeup::Wakeup,
    token: &CancellationToken,
) {
    let claim_repo = Arc::clone(swap_repo) as Arc<dyn swap::SwapStore>;
    let transfer_service = Arc::clone(&ssp_wallet.spark.transfer_service);
    let incoming = Arc::clone(incoming);
    let admission = admission.clone();
    let tree_store = Arc::clone(&ssp_wallet.spark.tree_store);
    let leaf_signing_keys = Arc::clone(pool_repo) as Arc<dyn leaves::LeafSigningKeys>;
    let claim_token = token.clone();
    tracker.spawn(async move {
        swap::claimer::run_swap_claim_loop(
            swap::claimer::SwapClaimDeps {
                swap_repo: claim_repo,
                tree_store,
                transfer_service,
                incoming,
                admission,
                leaf_signing_keys,
                wakeup,
            },
            claim_token,
        )
        .await;
    });
}

struct SparkCoopExitExecutor {
    service: Arc<coop_exit::CoopExitService>,
    chain_client: Arc<BitcoindClient>,
    chain_repository: Arc<postgresql::ChainRepository>,
    transfer_service: Arc<spark::services::TransferService>,
    incoming: Arc<dyn leaves::IncomingLeafStore>,
    admission: wakeup::Wakeup,
    leaf_signing_keys: Arc<dyn leaves::LeafSigningKeys>,
}

#[async_trait::async_trait]
impl coop_exit::CoopExitExecutor for SparkCoopExitExecutor {
    async fn sign(
        &self,
        record: &coop_exit::repository::CoopExitRecord,
    ) -> Result<(Vec<u8>, String), Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.service.sign_coop_exit_tx(record).await?)
    }

    async fn broadcast(
        &self,
        signed_tx: &[u8],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use chain::ChainClient as _;
        let tx: bitcoin::Transaction = bitcoin::consensus::deserialize(signed_tx)
            .map_err(|e| format!("invalid signed coop-exit tx: {e}"))?;
        match self.chain_client.broadcast_tx(tx).await {
            Ok(()) | Err(chain::BroadcastError::AlreadyKnown) => Ok(()),
            Err(e) => Err(Box::new(e)),
        }
    }

    async fn claim(
        &self,
        transfer_id: &spark::services::TransferId,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        coop_exit::claim_user_transfer(
            &self.transfer_service,
            &self.incoming,
            &self.admission,
            &self.leaf_signing_keys,
            transfer_id,
        )
        .await
    }

    async fn committed_transfer_is_valid(
        &self,
        record: &coop_exit::repository::CoopExitRecord,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.service.committed_transfer_is_valid(record).await?)
    }

    async fn exit_confirmations(
        &self,
        record: &coop_exit::repository::CoopExitRecord,
    ) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        use chain::ChainRepository as _;
        let tx: bitcoin::Transaction = bitcoin::consensus::deserialize(&record.raw_coop_exit_tx)?;
        let txid = tx.compute_txid();
        // The exit's second output funds the connector and pays a watched address.
        let output = tx
            .output
            .get(1)
            .ok_or("the exit tx has no connector funding output")?;
        let address = bitcoin::Address::from_script(&output.script_pubkey, self.service.network())?;
        let Some(tip) = self.chain_repository.get_tip().await? else {
            return Ok(0);
        };
        Ok(self
            .chain_repository
            .get_txos_for_address(&address)
            .await?
            .iter()
            .find(|txo| txo.outpoint == bitcoin::OutPoint { txid, vout: 1 })
            .map_or(0, |txo| txo.confirmations(tip.height)))
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_coop_exit_worker(
    tracker: &TaskTracker,
    incoming: &Arc<dyn leaves::IncomingLeafStore>,
    admission: &wakeup::Wakeup,
    ssp_wallet: &Arc<wallet::SspWallet<postgresql::ChainRepository>>,
    chain_client: &Arc<BitcoindClient>,
    chain_repository: &Arc<postgresql::ChainRepository>,
    coop_exit_service: &Arc<coop_exit::CoopExitService>,
    coop_exit_store: &Arc<dyn coop_exit::repository::CoopExitStore>,
    pool_repo: &Arc<postgresql::PoolRepository>,
    wakeup: &wakeup::Wakeup,
    token: &CancellationToken,
) {
    let executor = Arc::new(SparkCoopExitExecutor {
        service: Arc::clone(coop_exit_service),
        chain_client: Arc::clone(chain_client),
        chain_repository: Arc::clone(chain_repository),
        transfer_service: Arc::clone(&ssp_wallet.spark.transfer_service),
        incoming: Arc::clone(incoming),
        admission: admission.clone(),
        leaf_signing_keys: Arc::clone(pool_repo) as Arc<dyn leaves::LeafSigningKeys>,
    }) as Arc<dyn coop_exit::CoopExitExecutor>;
    let deps = coop_exit::CoopExitWorkerDeps {
        store: Arc::clone(coop_exit_store),
        executor,
    };
    let worker_token = token.clone();
    let wakeup = wakeup.clone();
    tracker.spawn(async move {
        coop_exit::run_coop_exit_loop(deps, wakeup, worker_token).await;
    });
}

struct BitcoindStaticDepositChain {
    chain_client: Arc<BitcoindClient>,
    chain_repository: Arc<postgresql::ChainRepository>,
    onchain: Arc<wallet::OnchainWallet<postgresql::ChainRepository>>,
    network: Network,
}

#[async_trait::async_trait]
impl static_deposit::StaticDepositChain for BitcoindStaticDepositChain {
    async fn deposit_output(
        &self,
        txid: &bitcoin::Txid,
        vout: u32,
    ) -> Result<Option<static_deposit::DepositOutput>, Box<dyn std::error::Error + Send + Sync>>
    {
        let outpoint = bitcoin::OutPoint { txid: *txid, vout };
        Ok(self
            .chain_client
            .unspent_output(&outpoint)
            .await?
            .map(|output| static_deposit::DepositOutput {
                tx_out: output.tx_out,
                confirmations: output.confirmations,
            }))
    }

    async fn new_address(
        &self,
    ) -> Result<bitcoin::Address, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.onchain.next_address().await?.0)
    }

    async fn is_confirmed(
        &self,
        tx: &bitcoin::Transaction,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        use chain::ChainRepository as _;
        // The chain monitor records the outputs paying the wallet's addresses as
        // their blocks are applied.
        let output = tx.output.first().ok_or("transaction has no output")?;
        let address = bitcoin::Address::from_script(&output.script_pubkey, self.network)?;
        let outpoint = bitcoin::OutPoint {
            txid: tx.compute_txid(),
            vout: 0,
        };
        Ok(self
            .chain_repository
            .get_txos_for_address(&address)
            .await?
            .iter()
            .any(|txo| txo.outpoint == outpoint))
    }

    async fn broadcast(
        &self,
        tx: &bitcoin::Transaction,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use chain::ChainClient as _;
        match self.chain_client.broadcast_tx(tx.clone()).await {
            Ok(()) | Err(chain::BroadcastError::AlreadyKnown) => Ok(()),
            Err(e) => Err(Box::new(e)),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_static_deposit_worker(
    tracker: &TaskTracker,
    static_deposit_service: &Arc<static_deposit::StaticDepositService>,
    static_deposit_store: &Arc<dyn static_deposit::repository::StaticDepositClaimStore>,
    instant_quote_store: &Arc<dyn static_deposit::repository::InstantStaticDepositQuoteStore>,
    static_deposit_chain: &Arc<dyn static_deposit::StaticDepositChain>,
    ssp_wallet: &Arc<wallet::SspWallet<postgresql::ChainRepository>>,
    blocks: wakeup::Wakeup,
    claims: wakeup::Wakeup,
    token: &CancellationToken,
) {
    let finalizer = Arc::new(static_deposit::SparkStaticDepositSpendFinalizer::new(
        Arc::clone(&ssp_wallet.spark.signer),
    )) as Arc<dyn static_deposit::StaticDepositSpendFinalizer>;
    let spend_cosigner = Arc::new(static_deposit::SparkStaticDepositSpendCosigner::new(
        Arc::clone(static_deposit_store),
        Arc::clone(&ssp_wallet.spark.operator_pool),
        Arc::clone(&ssp_wallet.spark.signer),
        Arc::clone(static_deposit_chain),
    )) as Arc<dyn static_deposit::StaticDepositSpendCosigner>;
    let deps = static_deposit::StaticDepositWorkerDeps {
        store: Arc::clone(static_deposit_store),
        quotes: Arc::clone(instant_quote_store),
        chain: Arc::clone(static_deposit_chain),
        finalizer,
        spend_cosigner,
        credit_settler: Arc::clone(static_deposit_service) as Arc<_>,
        blocks,
        claims,
    };
    let worker_token = token.clone();
    tracker.spawn(async move {
        static_deposit::run_static_deposit_loop(deps, worker_token).await;
    });
}

struct LightningConfig {
    network: Network,
    ldk_server_url: String,
    ldk_server_api_key: String,
    ldk_server_cert_path: String,
    invoice_signing_key: String,
    send_base_fee_sats: u64,
    send_fee_ppm: u64,
    receive_leaf_transfer_expiry: Duration,
    largest_denomination: u64,
}

impl LightningConfig {
    fn from_args(args: &Args, largest_denomination: u64) -> Self {
        Self {
            network: args.network,
            ldk_server_url: args.ldk_server_url.clone(),
            ldk_server_api_key: args.ldk_server_api_key.clone(),
            ldk_server_cert_path: args.ldk_server_cert_path.clone(),
            invoice_signing_key: args.ldk_server_invoice_signing_key.clone(),
            send_base_fee_sats: args.lightning_send_base_fee_sats,
            send_fee_ppm: args.lightning_send_fee_ppm,
            receive_leaf_transfer_expiry: Duration::from_secs(
                args.receive_leaf_transfer_expiry_seconds,
            ),
            largest_denomination,
        }
    }
}

const LDK_SERVER_CHECK_RETRY_DELAY: Duration = Duration::from_secs(10);

enum LdkServerCheck {
    Unreachable(String),
    Misconfigured(String),
}

async fn check_ldk_server(
    network: Network,
    invoice_signing_key: Option<&bitcoin::secp256k1::SecretKey>,
    ldk: &lightning::ldk::LdkServerNode,
) -> Result<(), LdkServerCheck> {
    let node_network = ldk.network().await.map_err(|e| {
        LdkServerCheck::Unreachable(format!("could not read the ldk-server network: {e}"))
    })?;
    if node_network != network {
        return Err(LdkServerCheck::Misconfigured(format!(
            "ldk-server is on {node_network}, but the daemon is configured for {network}"
        )));
    }
    let Some(key) = invoice_signing_key else {
        return Ok(());
    };
    let derived = key.public_key(&bitcoin::secp256k1::Secp256k1::new());
    let node_id = ldk.node_id().await.map_err(|e| {
        LdkServerCheck::Unreachable(format!("could not read the ldk-server node id: {e}"))
    })?;
    if derived != node_id {
        return Err(LdkServerCheck::Misconfigured(format!(
            "the configured invoice signing key is for {derived}, but the ldk-server node is \
             {node_id}; invoices signed with it would name a payee that cannot be paid"
        )));
    }
    Ok(())
}

/// Returns `false` when cancelled first.
async fn await_ldk_server(
    network: Network,
    invoice_signing_key: Option<&bitcoin::secp256k1::SecretKey>,
    ldk: &lightning::ldk::LdkServerNode,
    token: &CancellationToken,
) -> Result<bool, String> {
    loop {
        match check_ldk_server(network, invoice_signing_key, ldk).await {
            Ok(()) => return Ok(true),
            Err(LdkServerCheck::Misconfigured(e)) => return Err(e),
            Err(LdkServerCheck::Unreachable(e)) => {
                warn!("Lightning waits for ldk-server: {e}");
            }
        }
        tokio::select! {
            () = token.cancelled() => return Ok(false),
            () = tokio::time::sleep(LDK_SERVER_CHECK_RETRY_DELAY) => {}
        }
    }
}

fn invoice_signing_key(
    config: &LightningConfig,
) -> Result<Option<bitcoin::secp256k1::SecretKey>, Box<dyn std::error::Error>> {
    if config.invoice_signing_key.is_empty() {
        return Ok(None);
    }
    let bytes = hex::decode(&config.invoice_signing_key)
        .map_err(|e| format!("ldk-server invoice signing key is not hex: {e}"))?;
    let key = bitcoin::secp256k1::SecretKey::from_slice(&bytes)
        .map_err(|e| format!("invalid ldk-server invoice signing key: {e}"))?;
    Ok(Some(key))
}

/// Lightning is enabled once ldk-server is reachable and matches the configuration,
/// so an outage at startup leaves the rest of the daemon running.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn build_lightning(
    incoming: &Arc<dyn leaves::IncomingLeafStore>,
    admission: &wakeup::Wakeup,
    tracker: &TaskTracker,
    config: &LightningConfig,
    ssp_wallet: &Arc<wallet::SspWallet<postgresql::ChainRepository>>,
    pool_repo: &Arc<postgresql::PoolRepository>,
    lightning_store: &Arc<dyn lightning::repository::LightningStore>,
    token: &CancellationToken,
    shutdown: &Arc<shutdown::Shutdown>,
) -> Result<graphql::Lightning, Box<dyn std::error::Error>> {
    let lightning = graphql::Lightning::default();
    if config.ldk_server_url.is_empty() {
        info!("Lightning disabled: no ldk-server configured");
        return Ok(lightning);
    }
    let cltv_delta =
        lightning::receive::hold_invoice_cltv_delta(config.receive_leaf_transfer_expiry)?;
    let invoice_signing_key = invoice_signing_key(config)?;

    let cert = std::fs::read(&config.ldk_server_cert_path).map_err(|e| {
        format!(
            "failed to read ldk-server cert '{}': {e}",
            config.ldk_server_cert_path
        )
    })?;
    let send_wakeup = wakeup::Wakeup::new();
    let receive_wakeup = wakeup::Wakeup::new();
    let ldk = Arc::new(lightning::ldk::LdkServerNode::new(
        config.ldk_server_url.clone(),
        config.ldk_server_api_key.clone(),
        &cert,
        send_wakeup.clone(),
        receive_wakeup.clone(),
    )?);
    let node = Arc::clone(&ldk) as Arc<dyn lightning::node::LightningNode>;

    let htlc_service = Arc::new(spark::services::HtlcService::new(
        Arc::clone(&ssp_wallet.spark.operator_pool),
        ssp_wallet.spark.network,
        Arc::clone(&ssp_wallet.spark.spark_signer),
        Arc::clone(&ssp_wallet.spark.transfer_service),
        None,
    ));

    let send_service = Arc::new(lightning::send::LightningSendService::new(
        Arc::clone(&node),
        Arc::clone(lightning_store),
        Arc::clone(&ssp_wallet.spark.operator_pool),
        Arc::clone(&ssp_wallet.spark.signer),
        Arc::clone(&ssp_wallet.spark.transfer_service),
        ssp_wallet.spark.network,
        lightning::send::LightningSendFeePolicy {
            base_sats: config.send_base_fee_sats,
            ppm: config.send_fee_ppm,
        },
        send_wakeup.clone(),
    ));
    let receive_service = Arc::new(lightning::receive::LightningReceiveService::new(
        Arc::clone(&node),
        Arc::clone(lightning_store),
        invoice_signing_key,
        cltv_delta,
    ));

    let listener_node = Arc::clone(&ldk);
    let listener_token = token.clone();
    tracker.spawn(async move { listener_node.run_event_listener(listener_token).await });
    let refresh_node = Arc::clone(&ldk);
    let refresh_token = token.clone();
    tracker.spawn(async move {
        refresh_node
            .run_paid_hold_invoices_refresh(refresh_token)
            .await;
    });

    let send_deps = lightning::send::SendWorkerDeps {
        store: Arc::clone(lightning_store),
        node: Arc::clone(&node),
        operator_pool: Arc::clone(&ssp_wallet.spark.operator_pool),
        signer: Arc::clone(&ssp_wallet.spark.signer),
        transfer_service: Arc::clone(&ssp_wallet.spark.transfer_service),
        htlc_service: Arc::clone(&htlc_service),
        incoming: Arc::clone(incoming),
        admission: admission.clone(),
        leaf_signing_keys: Arc::clone(pool_repo) as Arc<dyn leaves::LeafSigningKeys>,
        network: ssp_wallet.spark.network,
        wakeup: send_wakeup,
    };

    let receive_deps = lightning::receive::ReceiveWorkerDeps {
        store: Arc::clone(lightning_store),
        node: Arc::clone(&node),
        operator_pool: Arc::clone(&ssp_wallet.spark.operator_pool),
        signer: Arc::clone(&ssp_wallet.spark.signer),
        htlc_service: Arc::clone(&htlc_service),
        tree_service: Arc::clone(&ssp_wallet.spark.tree_service),
        tree_store: Arc::clone(&ssp_wallet.spark.tree_store),
        key_resolver: Arc::clone(pool_repo) as Arc<dyn leaves::LeafSigningKeys>,
        network: ssp_wallet.spark.network,
        wakeup: receive_wakeup.clone(),
        leaf_transfer_expiry: config.receive_leaf_transfer_expiry,
        largest_denomination: config.largest_denomination,
    };

    let identity_public_key = ssp_wallet.spark.identity_public_key;
    let operator_pool = Arc::clone(&ssp_wallet.spark.operator_pool);
    let network = config.network;
    let url = config.ldk_server_url.clone();
    let enabled = lightning.clone();
    let workers = tracker.clone();
    let token = token.clone();
    let shutdown = Arc::clone(shutdown);
    tracker.spawn(async move {
        match await_ldk_server(network, invoice_signing_key.as_ref(), &ldk, &token).await {
            Ok(true) => {}
            Ok(false) => return,
            Err(e) => return shutdown.fail("Lightning", &e),
        }
        let send_token = token.clone();
        workers.spawn(async move { lightning::send::run_send_loop(send_deps, send_token).await });
        let events_token = token.clone();
        workers.spawn(async move {
            lightning::receive::wake_on_handover_events(
                identity_public_key,
                operator_pool,
                receive_wakeup,
                events_token,
            )
            .await;
        });
        workers.spawn(async move {
            lightning::receive::run_receive_loop(receive_deps, token).await;
        });
        enabled.enable(graphql::LightningServices {
            send: send_service,
            receive: receive_service,
        });
        info!(url = %url, "Lightning enabled via ldk-server");
    });
    Ok(lightning)
}

#[allow(clippy::too_many_arguments)]
fn spawn_servers(
    tracker: &TaskTracker,
    graphql_address: core::net::SocketAddr,
    internal_address: core::net::SocketAddr,
    network: Network,
    ssp_wallet: &Arc<wallet::SspWallet<postgresql::ChainRepository>>,
    chain_client: &Arc<BitcoindClient>,
    pool_repo: &Arc<postgresql::PoolRepository>,
    swap_repo: &Arc<postgresql::SwapRepository>,
    swap_claim_wakeup: wakeup::Wakeup,
    lightning_store: &Arc<dyn lightning::repository::LightningStore>,
    coop_exit_store: &Arc<dyn coop_exit::repository::CoopExitStore>,
    static_deposit_store: &Arc<dyn static_deposit::repository::StaticDepositClaimStore>,
    coop_exit_service: &Arc<coop_exit::CoopExitService>,
    static_deposit_service: &Arc<static_deposit::StaticDepositService>,
    lightning: graphql::Lightning,
    auth: Arc<auth::AuthService>,
    fee_rates: &Arc<dyn fees::FeeRateSource>,
    restock: &Arc<pool::restock::RestockService>,
    pool_config: &pool::config::PoolConfig,
    token: &CancellationToken,
    shutdown: &Arc<shutdown::Shutdown>,
) {
    let regtest_funder = matches!(network, Network::Regtest)
        .then(|| Arc::clone(chain_client) as Arc<dyn graphql::RegtestFunder>);
    spawn_graphql_server(
        tracker,
        graphql_address,
        ssp_wallet,
        pool_repo,
        swap_repo,
        swap_claim_wakeup,
        lightning_store,
        coop_exit_service,
        static_deposit_service,
        lightning,
        auth,
        fee_rates,
        regtest_funder,
        pool_config.largest_denomination(),
        token,
        shutdown,
    );
    spawn_internal_server(
        tracker,
        internal_address,
        network,
        ssp_wallet,
        chain_client,
        lightning_store,
        coop_exit_store,
        static_deposit_store,
        &(Arc::clone(swap_repo) as Arc<dyn swap::SwapStore>),
        restock,
        pool_config,
        token,
        shutdown,
    );
}

fn graphql_network(network: spark::Network) -> graphql::BitcoinNetwork {
    match network {
        spark::Network::Mainnet => graphql::BitcoinNetwork::Mainnet,
        spark::Network::Regtest => graphql::BitcoinNetwork::Regtest,
        spark::Network::Testnet => graphql::BitcoinNetwork::Testnet,
        spark::Network::Signet => graphql::BitcoinNetwork::Signet,
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_graphql_server(
    tracker: &TaskTracker,
    graphql_address: core::net::SocketAddr,
    ssp_wallet: &Arc<wallet::SspWallet<postgresql::ChainRepository>>,
    pool_repo: &Arc<postgresql::PoolRepository>,
    swap_repo: &Arc<postgresql::SwapRepository>,
    swap_claim_wakeup: wakeup::Wakeup,
    lightning_store: &Arc<dyn lightning::repository::LightningStore>,
    coop_exit_service: &Arc<coop_exit::CoopExitService>,
    static_deposit_service: &Arc<static_deposit::StaticDepositService>,
    lightning: graphql::Lightning,
    auth: Arc<auth::AuthService>,
    fee_rates: &Arc<dyn fees::FeeRateSource>,
    regtest_funder: Option<Arc<dyn graphql::RegtestFunder>>,
    largest_denomination: u64,
    token: &CancellationToken,
    shutdown: &Arc<shutdown::Shutdown>,
) {
    let shutdown = Arc::clone(shutdown);
    let graphql_token = token.clone();
    let swap_service = Arc::new(swap::SwapService::new(
        Arc::clone(&ssp_wallet.spark.tree_store),
        Arc::clone(&ssp_wallet.spark.transfer_service),
        Arc::clone(&ssp_wallet.spark.operator_pool),
        Arc::clone(&ssp_wallet.spark.signer),
        ssp_wallet.spark.network,
        Arc::clone(pool_repo) as Arc<dyn leaves::LeafSigningKeys>,
        Arc::clone(swap_repo) as Arc<dyn swap::SwapStore>,
        swap_claim_wakeup,
        largest_denomination,
    ));
    let schema = graphql::build_schema(graphql::SchemaContext {
        swap_service,
        coop_exit_service: Arc::clone(coop_exit_service),
        static_deposit_service: Arc::clone(static_deposit_service),
        lightning,
        ln_store: Arc::clone(lightning_store),
        network: graphql_network(ssp_wallet.spark.network),
        auth: Arc::clone(&auth),
        fee_rates: Arc::clone(fee_rates),
        regtest_funder,
    });
    let app = graphql::router(schema, auth);
    tracker.spawn(async move {
        info!(
            address = field::display(&graphql_address),
            "Starting GraphQL API server"
        );
        let listener = match tokio::net::TcpListener::bind(graphql_address).await {
            Ok(l) => l,
            Err(e) => {
                shutdown.fail(
                    "GraphQL API server",
                    &format!("failed to bind {graphql_address}: {e}"),
                );
                return;
            }
        };
        let res = axum::serve(listener, app)
            .with_graceful_shutdown(async move { graphql_token.cancelled().await })
            .await;
        match res {
            Ok(()) => shutdown.stop("the GraphQL API server finished"),
            Err(e) => shutdown.fail("GraphQL API server", &format!("{e:?}")),
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn spawn_internal_server(
    tracker: &TaskTracker,
    internal_address: core::net::SocketAddr,
    network: Network,
    ssp_wallet: &Arc<wallet::SspWallet<postgresql::ChainRepository>>,
    chain_client: &Arc<BitcoindClient>,
    lightning_store: &Arc<dyn lightning::repository::LightningStore>,
    coop_exit_store: &Arc<dyn coop_exit::repository::CoopExitStore>,
    static_deposit_store: &Arc<dyn static_deposit::repository::StaticDepositClaimStore>,
    swap_store: &Arc<dyn swap::SwapStore>,
    restock: &Arc<pool::restock::RestockService>,
    pool_config: &pool::config::PoolConfig,
    token: &CancellationToken,
    shutdown: &Arc<shutdown::Shutdown>,
) {
    let shutdown = Arc::clone(shutdown);
    let internal_server_token = token.child_token();
    let params = internal_server::ServerParams {
        chain_client: Arc::clone(chain_client),
        network,
        token: token.clone(),
        wallet: Arc::clone(ssp_wallet),
        lightning_store: Arc::clone(lightning_store),
        coop_exit_store: Arc::clone(coop_exit_store),
        static_deposit_store: Arc::clone(static_deposit_store),
        swap_store: Arc::clone(swap_store),
    };
    let manager_server = SspManagerServer::new(internal_server::Server::new(&params));
    let wallet_server = OnchainWalletServer::new(internal_server::WalletServer::new(&params));
    let pool_server = PoolServer::new(internal_server::PoolServer::new(
        Arc::clone(restock),
        Arc::clone(&ssp_wallet.spark.tree_store),
        pool_config.denominations(),
        token.clone(),
    ));
    tracker.spawn(async move {
        info!(
            address = field::display(&internal_address),
            "Starting internal server"
        );
        let res = Server::builder()
            .add_service(manager_server)
            .add_service(wallet_server)
            .add_service(pool_server)
            .serve_with_shutdown(internal_address, internal_server_token.cancelled())
            .await;
        match res {
            Ok(()) => shutdown.stop("the internal server finished"),
            Err(e) => shutdown.fail("internal server", &format!("{e:?}")),
        }
    });
}

fn spawn_exit_chain_resolver(
    tracker: &TaskTracker,
    ssp_wallet: &Arc<wallet::SspWallet<postgresql::ChainRepository>>,
    token: &CancellationToken,
) {
    let resolver = Arc::clone(&ssp_wallet.spark.exit_chains);
    let token = token.clone();
    tracker.spawn(async move {
        let mut interval = tokio::time::interval(EXIT_CHAIN_RESOLVE_INTERVAL);
        loop {
            tokio::select! {
                () = token.cancelled() => {
                    info!("Exit chain resolver cancelled");
                    return;
                }
                _ = interval.tick() => {
                    if let Err(e) = resolver.resolve_missing_chains().await {
                        tracing::warn!("could not resolve exit chains: {e:?}");
                    }
                }
            }
        }
    });
}

fn spawn_leaf_admission(
    tracker: &TaskTracker,
    ssp_wallet: &Arc<wallet::SspWallet<postgresql::ChainRepository>>,
    incoming: &Arc<dyn leaves::IncomingLeafStore>,
    admission: wakeup::Wakeup,
    token: &CancellationToken,
) {
    let deps = pool::incoming::AdmissionDeps {
        store: Arc::clone(incoming),
        tree_service: Arc::clone(&ssp_wallet.spark.tree_service),
        tree_store: Arc::clone(&ssp_wallet.spark.tree_store),
        timelocks: Arc::clone(&ssp_wallet.spark.timelocks),
        identity: ssp_wallet.spark.identity_public_key,
    };
    let token = token.clone();
    tracker.spawn(async move {
        let mut interval = tokio::time::interval(LEAF_ADMISSION_INTERVAL);
        loop {
            // Drains the backlog: a large arrival should not need one wake per batch.
            while !token.is_cancelled() {
                match pool::incoming::admit_once(&deps).await {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(e) => {
                        tracing::warn!("could not admit claimed leaves: {e:?}");
                        break;
                    }
                }
            }
            tokio::select! {
                () = token.cancelled() => {
                    info!("Leaf admission cancelled");
                    return;
                }
                () = admission.waited() => {}
                _ = interval.tick() => {}
            }
        }
    });
}
