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
use serde::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, serde_as};
use sqlx::postgres::PgPoolOptions;
#[cfg(not(unix))]
use tokio::signal;
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tracing::{info, warn};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use sspd_lib::{bitcoind, chain, fees, pool, postgresql, shutdown, wakeup, wallet};

const DEPENDENCY_RETRY_INTERVAL: Duration = Duration::from_secs(5);

#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize, Parser)]
#[command(version, about = "Spark Service Provider Daemon", long_about = None)]
struct Args {
    #[arg(long, default_value = "sspd.conf")]
    pub config: PathBuf,

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

    /// Nodes the operators allow one tree-creation round to carry. A larger tree
    /// is built in layers, so this decides how many calls creating one takes.
    #[arg(long, default_value = "1000")]
    pub max_nodes_per_request: usize,

    /// The signing operators. Unset uses the network's default operators. Not a
    /// command-line flag: it is a list.
    #[arg(skip)]
    pub operators: Option<Vec<wallet::OperatorSetting>>,
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
        vec![replenish_wakeup.clone(), pooling_wakeup.clone()],
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
    let fee_rates = Arc::new(chain::CachedFeeRates::new(
        Arc::clone(&chain_client) as Arc<_>
    )) as Arc<dyn fees::FeeRateSource>;

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
