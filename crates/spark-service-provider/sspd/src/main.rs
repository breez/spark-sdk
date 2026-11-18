// Not SDK code: the spawn and reqwest rules in the root clippy.toml guard what
// integrators ship.
#![allow(clippy::disallowed_methods)]
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

use std::collections::{HashMap, HashSet};
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
#[cfg(not(unix))]
use tokio::signal;
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tonic::transport::Server;
use tracing::{field, info, warn};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use sspd_lib::incoming_leaves::{CoordinatorAncestors, IncomingLeafCheck};
use sspd_lib::{
    auth, bitcoind, chain, coop_exit, fees, graphql, in_flight, internal_server, leaves, lightning,
    pool, postgresql, shutdown, static_deposit, swap, wakeup, wallet,
};

/// The renewal loop also runs a pass when sspd claims a transfer.
const LEAF_RENEWAL_INTERVAL: Duration = Duration::from_secs(30);

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

    /// The most connections the daemon has open to the postgres database.
    #[arg(long, default_value = "10")]
    pub db_max_connections: usize,

    /// Address to the bitcoind rpc. bitcoind has to run with `-txospenderindex`.
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

    /// Confirmations the funding output of one of the SSP's own trees needs
    /// before the SSP adds the tree's leaves to the pool. Set it to the
    /// operators' `non_static_confirmation_threshold`: they make a tree
    /// available at that count.
    ///
    /// The SSP also refuses a user's transfer while the tree of one of the
    /// transfer's leaves has a funding output with fewer confirmations.
    #[arg(long, default_value = "3")]
    pub deposit_confirmations: u64,

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
        postgresql::connect(&args.db_url, args.db_max_connections)
            .await
            .map_err(|e| e.to_string())?,
    );
    if args.auto_migrate {
        postgresql::migrate(&pgpool)
            .await
            .map_err(|e| e.to_string())?;
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
    let unilateral_exit_blocks = wakeup::Wakeup::new();
    let spend_check_blocks = wakeup::Wakeup::new();
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
    if !chain_info.spender_index {
        return Err(
            "bitcoind runs without -txospenderindex, which finds the spend of an output".into(),
        );
    }

    let chain_tip = chain::ChainTip::load(chain_repository.as_ref()).await?;
    let call_queue = Arc::new(chain::CallQueue::new(
        Arc::clone(&chain_client) as Arc<dyn chain::RequestChainClient>
    ));
    let synced = chain::Synced::new();
    spawn_chain_monitor(
        &tracker,
        args.network,
        Duration::from_secs(args.chain_poll_interval_seconds),
        &chain_client,
        &chain_repository,
        chain_tip.clone(),
        synced.clone(),
        vec![
            replenish_wakeup.clone(),
            pooling_wakeup.clone(),
            coop_exit_wakeup.clone(),
            static_deposit_blocks.clone(),
            htlc_sweep_blocks.clone(),
            unilateral_exit_blocks.clone(),
            spend_check_blocks.clone(),
        ],
        &token,
    );

    let ssp_wallet = Arc::new(
        wallet::SspWallet::new(
            &seed,
            args.network,
            Arc::clone(&chain_repository),
            Arc::clone(&chain_repository),
            Arc::clone(&pgpool),
            args.operators.clone(),
            args.max_nodes_per_request,
        )
        .await
        .map_err(|e| format!("failed to initialize wallet: {e}"))?,
    );

    let pool_repo = Arc::new(postgresql::PoolRepository::new(Arc::clone(&pgpool)));
    let renewal = wakeup::Wakeup::new();
    let incoming_leaves = Arc::new(IncomingLeafCheck {
        leaf_store: Arc::clone(&ssp_wallet.spark.leaf_store),
        chain_repository: Arc::clone(&chain_repository),
        chain_tip: chain_tip.clone(),
        ancestors: Arc::new(CoordinatorAncestors {
            operator_pool: Arc::clone(&ssp_wallet.spark.operator_pool),
            network: ssp_wallet.spark.network,
        }),
        call_queue: Arc::clone(&call_queue),
        synced: synced.clone(),
        confirmations: args.deposit_confirmations,
        timeout: chain::call_queue::TIMEOUT,
    });
    let claim = Arc::new(leaves::ClaimDeps {
        transfer_service: Arc::clone(&ssp_wallet.spark.transfer_service),
        operator_pool: Arc::clone(&ssp_wallet.spark.operator_pool),
        network: ssp_wallet.spark.network,
        signing_keys: Arc::clone(&pool_repo) as Arc<dyn leaves::LeafSigningKeys>,
        leaf_store: Arc::clone(&ssp_wallet.spark.leaf_store),
        renewal: renewal.clone(),
    });
    let swap_repo = Arc::new(postgresql::SwapRepository::new(Arc::clone(&pgpool)));
    let lightning_store = Arc::new(postgresql::PostgresLightningStore::new(Arc::clone(&pgpool)))
        as Arc<dyn lightning::repository::LightningStore>;
    let coop_exit_store = Arc::new(postgresql::PostgresCoopExitStore::new(Arc::clone(&pgpool)))
        as Arc<dyn coop_exit::repository::CoopExitStore>;
    let cached_fee_rates = Arc::new(chain::CachedFeeRates::new(
        Arc::clone(&chain_client) as Arc<_>
    ));
    if let Err(e) = cached_fee_rates.refresh().await {
        warn!("bitcoind returned no fee rate estimate at startup: {e}");
    }
    let fee_rate_token = token.clone();
    tracker.spawn(Arc::clone(&cached_fee_rates).run(fee_rate_token));
    let fee_rates = cached_fee_rates as Arc<dyn fees::FeeRateSource>;
    let coop_exit_service = Arc::new(coop_exit::CoopExitService::new(
        Arc::clone(&coop_exit_store),
        Arc::clone(&ssp_wallet.spark.operator_pool),
        Arc::clone(&ssp_wallet.spark.signer),
        Arc::clone(&ssp_wallet.spark.transfer_service),
        Arc::clone(&ssp_wallet.onchain) as Arc<dyn coop_exit::CoopExitOnchainWallet>,
        Arc::clone(&fee_rates),
        Arc::clone(&incoming_leaves),
        synced.clone(),
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
    let static_deposit_addresses = Arc::new(postgresql::PostgresStaticDepositAddressStore::new(
        Arc::clone(&pgpool),
    ))
        as Arc<dyn static_deposit::repository::StaticDepositAddressStore>;
    let static_deposit_chain = Arc::new(BitcoindStaticDepositChain {
        chain_client: Arc::clone(&chain_client),
        call_queue: Arc::clone(&call_queue),
        synced: synced.clone(),
        chain_repository: Arc::clone(&chain_repository),
        onchain: Arc::clone(&ssp_wallet.onchain),
        network: args.network,
    }) as Arc<dyn static_deposit::StaticDepositChain>;
    let static_deposit_service = Arc::new(static_deposit::StaticDepositService::new(
        Arc::clone(&static_deposit_store),
        Arc::clone(&instant_quote_store),
        Arc::clone(&static_deposit_addresses),
        Arc::clone(&ssp_wallet.spark.transfer_service),
        Arc::clone(&ssp_wallet.spark.leaf_store),
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
        &chain_tip,
        &pool_repo,
        &fee_rates,
        args.deposit_confirmations,
        replenish_wakeup,
        pooling_wakeup,
        &token,
    );
    let swap_claim_wakeup = wakeup::Wakeup::new();
    let swap_requests = Arc::new(in_flight::InFlight::default());
    spawn_swap_claimer(
        &tracker,
        &claim,
        &swap_repo,
        &swap_requests,
        swap_claim_wakeup.clone(),
        &token,
    );
    spawn_coop_exit_worker(
        &tracker,
        &claim,
        &chain_client,
        &chain_repository,
        &chain_tip,
        &coop_exit_service,
        &coop_exit_store,
        &coop_exit_wakeup,
        &token,
    );
    spawn_static_deposit_worker(
        &tracker,
        &static_deposit_service,
        &static_deposit_store,
        &instant_quote_store,
        &static_deposit_addresses,
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

    let operator_view = pool::operator_view::OperatorViewDeps {
        operator_pool: Arc::clone(&ssp_wallet.spark.operator_pool),
        network: ssp_wallet.spark.network,
        leaf_store: Arc::clone(&ssp_wallet.spark.leaf_store),
    };
    let operator_view_token = token.clone();
    tracker.spawn(async move {
        pool::operator_view::run_operator_view_loop(operator_view, operator_view_token).await;
    });

    let unilateral_exit = pool::unilateral_exit::UnilateralExitDeps {
        leaf_store: Arc::clone(&ssp_wallet.spark.leaf_store),
        unilateral_exit_store: Arc::new(postgresql::PostgresUnilateralExitStore::new(Arc::clone(
            &pgpool,
        ))),
        chain_repository: Arc::clone(&chain_repository),
        chain_tip: chain_tip.clone(),
        chain_client: Arc::clone(&chain_client) as Arc<dyn chain::ChainClient + Send + Sync>,
        onchain: Arc::clone(&ssp_wallet.onchain),
        blocks: unilateral_exit_blocks,
    };
    let unilateral_exit_token = token.clone();
    tracker.spawn(async move {
        pool::unilateral_exit::run_unilateral_exit_loop(unilateral_exit, unilateral_exit_token)
            .await;
    });

    let spend_check = chain::spend_check::SpendCheckDeps {
        chain_repository: Arc::clone(&chain_repository),
        chain_tip: chain_tip.clone(),
        chain_client: Arc::clone(&chain_client) as Arc<dyn chain::ChainClient + Send + Sync>,
        blocks: spend_check_blocks,
    };
    let spend_check_token = token.clone();
    tracker.spawn(async move {
        chain::spend_check::run_spend_check_loop(spend_check, spend_check_token).await;
    });

    let lightning = build_lightning(
        &claim,
        &incoming_leaves,
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
        &call_queue,
        &chain_repository,
        &chain_tip,
        &synced,
        &pool_repo,
        &swap_repo,
        &incoming_leaves,
        &swap_requests,
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

    spawn_leaf_renewal(&tracker, &ssp_wallet, renewal.clone(), &token);

    info!("sspd started");

    tracker.close();

    tracker.wait().await;
    if let Some(failure) = shutdown.failure() {
        return Err(format!("sspd shut down: {failure}").into());
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
    chain_tip: chain::ChainTip,
    synced: chain::Synced,
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
        chain_tip,
        synced,
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
    chain_tip: &chain::ChainTip,
    pool_repo: &Arc<postgresql::PoolRepository>,
    fee_rates: &Arc<dyn fees::FeeRateSource>,
    deposit_confirmations: u64,
    replenish_wakeup: wakeup::Wakeup,
    pooling_wakeup: wakeup::Wakeup,
    token: &CancellationToken,
) {
    let replenish_wallet = Arc::clone(ssp_wallet);
    let replenish_client = Arc::clone(chain_client);
    let replenish_chain_repo = Arc::clone(chain_repository);
    let replenish_chain_tip = chain_tip.clone();
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
            replenish_chain_tip,
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
    let pooling_chain_tip = chain_tip.clone();
    let pooling_repo = Arc::clone(pool_repo);
    let pooling_restock = Arc::clone(restock);
    let pooling_token = token.clone();
    tracker.spawn(async move {
        let deps = pool::tree_pooling::TreePoolingDeps {
            wallet: pooling_wallet,
            chain_repository: pooling_chain_repo,
            chain_tip: pooling_chain_tip,
            pool_repo: pooling_repo,
            restock: pooling_restock,
            confirmations: deposit_confirmations,
        };
        pool::tree_pooling::run_tree_pooling_loop(deps, pooling_wakeup, pooling_token).await;
    });
}

fn spawn_swap_claimer(
    tracker: &TaskTracker,
    claim: &Arc<leaves::ClaimDeps>,
    swap_repo: &Arc<postgresql::SwapRepository>,
    requests: &Arc<in_flight::InFlight>,
    wakeup: wakeup::Wakeup,
    token: &CancellationToken,
) {
    let deps = swap::claimer::SwapClaimDeps {
        swap_repo: Arc::clone(swap_repo) as Arc<dyn swap::SwapStore>,
        claim: Arc::clone(claim),
        requests: Arc::clone(requests),
        wakeup,
    };
    let claim_token = token.clone();
    tracker.spawn(async move {
        swap::claimer::run_swap_claim_loop(deps, claim_token).await;
    });
}

struct SparkCoopExitExecutor {
    service: Arc<coop_exit::CoopExitService>,
    chain_client: Arc<BitcoindClient>,
    chain_repository: Arc<postgresql::ChainRepository>,
    chain_tip: chain::ChainTip,
    claim: Arc<leaves::ClaimDeps>,
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
        coop_exit::claim_user_transfer(&self.claim, transfer_id).await
    }

    async fn committed_transfer_is_valid(
        &self,
        record: &coop_exit::repository::CoopExitRecord,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.service.committed_transfer_is_valid(record).await?)
    }

    async fn exit_confirmations(
        &self,
        records: &[coop_exit::repository::CoopExitRecord],
    ) -> Result<HashMap<String, u64>, Box<dyn std::error::Error + Send + Sync>> {
        use chain::ChainRepository as _;
        let mut connectors = Vec::with_capacity(records.len());
        for record in records {
            match connector_funding_output(record, self.service.network()) {
                Ok((outpoint, address)) => connectors.push((&record.id, outpoint, address)),
                Err(e) => warn!(coop_exit_id = %record.id, "invalid exit transaction: {e}"),
            }
        }
        if connectors.is_empty() {
            return Ok(HashMap::new());
        }
        let addresses: Vec<bitcoin::Address> = connectors
            .iter()
            .map(|(_, _, address)| address.clone())
            .collect();
        let Some(tip) = self.chain_tip.record() else {
            return Ok(HashMap::new());
        };
        let txos = self
            .chain_repository
            .get_txos_for_addresses(&addresses)
            .await?;
        Ok(connectors
            .iter()
            .filter_map(|(id, outpoint, _)| {
                let txo = txos.iter().find(|txo| txo.outpoint == *outpoint)?;
                Some(((*id).clone(), txo.confirmations(tip.height)))
            })
            .collect())
    }
}

/// The exit transaction's output that is the input of the connector tx. The
/// output's address is an on-chain wallet address, so the chain monitor
/// records the output.
fn connector_funding_output(
    record: &coop_exit::repository::CoopExitRecord,
    network: Network,
) -> Result<(bitcoin::OutPoint, bitcoin::Address), Box<dyn std::error::Error + Send + Sync>> {
    let tx: bitcoin::Transaction = bitcoin::consensus::deserialize(&record.raw_coop_exit_tx)?;
    let output = tx
        .output
        .get(1)
        .ok_or("the exit tx has no connector funding output")?;
    let address = bitcoin::Address::from_script(&output.script_pubkey, network)?;
    let outpoint = bitcoin::OutPoint {
        txid: tx.compute_txid(),
        vout: 1,
    };
    Ok((outpoint, address))
}

#[allow(clippy::too_many_arguments)]
fn spawn_coop_exit_worker(
    tracker: &TaskTracker,
    claim: &Arc<leaves::ClaimDeps>,
    chain_client: &Arc<BitcoindClient>,
    chain_repository: &Arc<postgresql::ChainRepository>,
    chain_tip: &chain::ChainTip,
    coop_exit_service: &Arc<coop_exit::CoopExitService>,
    coop_exit_store: &Arc<dyn coop_exit::repository::CoopExitStore>,
    wakeup: &wakeup::Wakeup,
    token: &CancellationToken,
) {
    let executor = Arc::new(SparkCoopExitExecutor {
        service: Arc::clone(coop_exit_service),
        chain_client: Arc::clone(chain_client),
        chain_repository: Arc::clone(chain_repository),
        chain_tip: chain_tip.clone(),
        claim: Arc::clone(claim),
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

/// bitcoind pays the faucet's coins from its own wallet. A user requests the
/// coins, so sspd sends the payment call to bitcoind through the call queue.
struct Faucet {
    call_queue: Arc<chain::CallQueue>,
}

#[async_trait::async_trait]
impl graphql::RegtestFunder for Faucet {
    async fn send_to_address(&self, address: &str, amount_sats: u64) -> Result<String, String> {
        let Ok(sent) = tokio::time::timeout(
            chain::call_queue::TIMEOUT,
            self.call_queue.send_to_address(address, amount_sats),
        )
        .await
        else {
            warn!("bitcoind did not make a faucet payment in time");
            return Err("the SSP could not serve the request".to_string());
        };
        sent.map_err(|e| e.to_string())
    }
}

struct BitcoindStaticDepositChain {
    chain_client: Arc<BitcoindClient>,
    call_queue: Arc<chain::CallQueue>,
    synced: chain::Synced,
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

    async fn requested_deposit_output(
        &self,
        txid: &bitcoin::Txid,
        vout: u32,
    ) -> Result<Option<static_deposit::DepositOutput>, Box<dyn std::error::Error + Send + Sync>>
    {
        if !self.synced.is_synced() {
            return Err("the chain record is not at bitcoind's tip".into());
        }
        let outpoint = bitcoin::OutPoint { txid: *txid, vout };
        let output = tokio::time::timeout(
            chain::call_queue::TIMEOUT,
            self.call_queue.unspent_output(&outpoint),
        )
        .await
        .map_err(|_| "bitcoind returned no result in time")??;
        Ok(output.map(|output| static_deposit::DepositOutput {
            tx_out: output.tx_out,
            confirmations: output.confirmations,
        }))
    }

    async fn new_address(
        &self,
    ) -> Result<bitcoin::Address, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.onchain.next_address().await?.0)
    }

    async fn confirmed_among(
        &self,
        txs: &[bitcoin::Transaction],
    ) -> Result<HashSet<bitcoin::Txid>, Box<dyn std::error::Error + Send + Sync>> {
        use chain::ChainRepository as _;
        // The chain monitor records the outputs paying the wallet's addresses as
        // their blocks are applied.
        // The function reports a spend as unconfirmed when its first output has
        // no address, since the chain monitor records no such output.
        let addresses: Vec<bitcoin::Address> = txs
            .iter()
            .filter_map(|tx| tx.output.first())
            .filter_map(|output| {
                bitcoin::Address::from_script(&output.script_pubkey, self.network).ok()
            })
            .collect();
        let txos = self
            .chain_repository
            .get_txos_for_addresses(&addresses)
            .await?;
        Ok(txs
            .iter()
            .map(bitcoin::Transaction::compute_txid)
            .filter(|txid| {
                let outpoint = bitcoin::OutPoint {
                    txid: *txid,
                    vout: 0,
                };
                txos.iter().any(|txo| txo.outpoint == outpoint)
            })
            .collect())
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
    static_deposit_addresses: &Arc<dyn static_deposit::repository::StaticDepositAddressStore>,
    static_deposit_chain: &Arc<dyn static_deposit::StaticDepositChain>,
    ssp_wallet: &Arc<wallet::SspWallet<postgresql::ChainRepository>>,
    blocks: wakeup::Wakeup,
    claims: wakeup::Wakeup,
    token: &CancellationToken,
) {
    let spend_cosigner = Arc::new(static_deposit::SparkStaticDepositSpendCosigner::new(
        Arc::clone(static_deposit_store),
        Arc::clone(static_deposit_addresses),
        Arc::clone(&ssp_wallet.spark.operator_pool),
        Arc::clone(&ssp_wallet.spark.signer),
        Arc::clone(static_deposit_chain),
    )) as Arc<dyn static_deposit::StaticDepositSpendCosigner>;
    let deps = static_deposit::StaticDepositWorkerDeps {
        store: Arc::clone(static_deposit_store),
        quotes: Arc::clone(instant_quote_store),
        chain: Arc::clone(static_deposit_chain),
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
    claim: &Arc<leaves::ClaimDeps>,
    incoming_leaves: &Arc<IncomingLeafCheck>,
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
        Arc::clone(incoming_leaves),
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
        htlc_service: Arc::clone(&htlc_service),
        claim: Arc::clone(claim),
        network: ssp_wallet.spark.network,
        wakeup: send_wakeup,
    };

    let receive_deps = lightning::receive::ReceiveWorkerDeps {
        store: Arc::clone(lightning_store),
        node: Arc::clone(&node),
        operator_pool: Arc::clone(&ssp_wallet.spark.operator_pool),
        signer: Arc::clone(&ssp_wallet.spark.signer),
        htlc_service: Arc::clone(&htlc_service),
        leaf_store: Arc::clone(&ssp_wallet.spark.leaf_store),
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
    call_queue: &Arc<chain::CallQueue>,
    chain_repository: &Arc<postgresql::ChainRepository>,
    chain_tip: &chain::ChainTip,
    synced: &chain::Synced,
    pool_repo: &Arc<postgresql::PoolRepository>,
    swap_repo: &Arc<postgresql::SwapRepository>,
    incoming_leaves: &Arc<IncomingLeafCheck>,
    swap_requests: &Arc<in_flight::InFlight>,
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
    let regtest_funder = matches!(network, Network::Regtest).then(|| {
        Arc::new(Faucet {
            call_queue: Arc::clone(call_queue),
        }) as Arc<dyn graphql::RegtestFunder>
    });
    spawn_graphql_server(
        tracker,
        graphql_address,
        ssp_wallet,
        pool_repo,
        swap_repo,
        incoming_leaves,
        swap_requests,
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
        chain_repository,
        chain_tip,
        synced,
        lightning_store,
        coop_exit_store,
        static_deposit_store,
        &(Arc::clone(swap_repo) as Arc<dyn swap::SwapStore>),
        pool_repo,
        fee_rates,
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
    incoming_leaves: &Arc<IncomingLeafCheck>,
    swap_requests: &Arc<in_flight::InFlight>,
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
        Arc::clone(&ssp_wallet.spark.leaf_store),
        Arc::clone(&ssp_wallet.spark.transfer_service),
        Arc::clone(&ssp_wallet.spark.operator_pool),
        Arc::clone(&ssp_wallet.spark.signer),
        ssp_wallet.spark.network,
        Arc::clone(pool_repo) as Arc<dyn leaves::LeafSigningKeys>,
        Arc::clone(swap_repo) as Arc<dyn swap::SwapStore>,
        Arc::clone(incoming_leaves),
        Arc::clone(swap_requests),
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
    chain_repository: &Arc<postgresql::ChainRepository>,
    chain_tip: &chain::ChainTip,
    synced: &chain::Synced,
    lightning_store: &Arc<dyn lightning::repository::LightningStore>,
    coop_exit_store: &Arc<dyn coop_exit::repository::CoopExitStore>,
    static_deposit_store: &Arc<dyn static_deposit::repository::StaticDepositClaimStore>,
    swap_store: &Arc<dyn swap::SwapStore>,
    pool_repo: &Arc<postgresql::PoolRepository>,
    fee_rates: &Arc<dyn fees::FeeRateSource>,
    restock: &Arc<pool::restock::RestockService>,
    pool_config: &pool::config::PoolConfig,
    token: &CancellationToken,
    shutdown: &Arc<shutdown::Shutdown>,
) {
    let shutdown = Arc::clone(shutdown);
    let internal_server_token = token.child_token();
    let params = internal_server::ServerParams {
        chain_tip: chain_tip.clone(),
        synced: synced.clone(),
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
    let refund_outputs = pool::refund_outputs::RefundOutputs {
        leaf_store: Arc::clone(&ssp_wallet.spark.leaf_store),
        chain_repository: Arc::clone(chain_repository),
        chain_client: Arc::clone(chain_client) as Arc<dyn chain::ChainClient + Send + Sync>,
        fee_rates: Arc::clone(fee_rates),
        onchain: Arc::clone(&ssp_wallet.onchain),
        signer: Arc::clone(&ssp_wallet.spark.spark_signer),
        signing_keys: Arc::clone(pool_repo) as Arc<dyn leaves::LeafSigningKeys>,
    };
    let pool_server = PoolServer::new(internal_server::PoolServer::new(
        Arc::clone(restock),
        Arc::clone(&ssp_wallet.spark.leaf_store),
        refund_outputs,
        internal_server::ReservingStores {
            swaps: Arc::clone(swap_store),
            lightning: Arc::clone(lightning_store),
            static_deposits: Arc::clone(static_deposit_store),
        },
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

fn spawn_leaf_renewal(
    tracker: &TaskTracker,
    ssp_wallet: &Arc<wallet::SspWallet<postgresql::ChainRepository>>,
    renewal: wakeup::Wakeup,
    token: &CancellationToken,
) {
    let deps = pool::renewal::RenewalDeps {
        leaf_store: Arc::clone(&ssp_wallet.spark.leaf_store),
        timelocks: Arc::clone(&ssp_wallet.spark.timelocks),
        operator_pool: Arc::clone(&ssp_wallet.spark.operator_pool),
        network: ssp_wallet.spark.network,
    };
    let token = token.clone();
    tracker.spawn(async move {
        let mut attempts = pool::renewal::Attempts::default();
        let mut interval = tokio::time::interval(LEAF_RENEWAL_INTERVAL);
        loop {
            if let Err(e) = pool::renewal::renew_pass(&deps, &mut attempts).await {
                tracing::warn!("could not renew pool leaves: {e}");
            }
            tokio::select! {
                () = token.cancelled() => {
                    info!("Leaf renewal cancelled");
                    return;
                }
                () = renewal.waited() => {}
                _ = interval.tick() => {}
            }
        }
    });
}
