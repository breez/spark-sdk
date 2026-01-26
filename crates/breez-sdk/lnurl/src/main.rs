use crate::{repository::LnurlRepository, routes::LnurlServer, state::State};
use anyhow::anyhow;
use axum::{
    Extension, Router,
    extract::DefaultBodyLimit,
    http::Method,
    middleware,
    routing::{delete, get, post},
};
use base64::{Engine, prelude::BASE64_STANDARD};
use clap::Parser;
use figment::{
    Figment,
    providers::{Env, Format, Serialized, Toml},
};
use serde::{Deserialize, Serialize};
use spark::operator::rpc::DefaultConnectionManager;
use spark::session_manager::InMemorySessionManager;
use spark::ssp::ServiceProvider;
use spark::token::InMemoryTokenOutputStore;
use spark::tree::InMemoryTreeStore;
use spark_wallet::{DefaultSigner, Network, SparkWalletConfig};
use sqlx::{PgPool, SqlitePool};
use std::collections::HashSet;
use std::str::FromStr;
use std::{path::PathBuf, sync::Arc};
use tokio::sync::{Mutex, watch};
use tower_http::cors::{Any, CorsLayer};
use tracing::{debug, error, info};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};
use x509_parser::prelude::{FromDer, X509Certificate};

mod auth;
mod background;
mod error;
mod invoice_paid;
mod postgresql;
mod repository;
mod routes;
mod sqlite;
mod state;
mod time;
mod user;
mod zap;

#[derive(Clone, Parser, Debug, Serialize, Deserialize)]
#[command(version, about, long_about = None)]
struct Args {
    /// Address the lnurl server will listen on.
    #[arg(long, default_value = "0.0.0.0:8080")]
    pub address: core::net::SocketAddr,

    #[arg(long, default_value = "lnurl.conf")]
    pub config: PathBuf,

    /// Automatically apply migrations to the database.
    #[arg(long)]
    pub auto_migrate: bool,

    /// Connection string to the postgres database.
    #[arg(long, default_value = "")]
    pub db_url: String,

    /// Loglevel to use. Can be used to filter logs through the env filter
    /// format.
    #[arg(long, default_value = "info")]
    pub log_level: String,

    /// Spark network.
    #[arg(long, default_value = "mainnet")]
    pub network: Network,

    /// Scheme prefix for lnurl urls.
    #[arg(long, default_value = "https")]
    pub scheme: String,

    /// Minimum amount (in millisatoshi) that can be sent in a lnurl payment.
    #[arg(long, default_value = "1000")]
    pub min_sendable: u64,

    /// Maximum amount (in millisatoshi) that can be sent in a lnurl payment.
    #[arg(long, default_value = "4000000000")]
    pub max_sendable: u64,

    /// Whether to include the spark address in the invoices generated.
    /// If included this can reduce fees for wallets that support it at the
    /// cost of privacy.
    #[arg(long, default_value = "false")]
    pub include_spark_address: bool,

    /// List of domains that are allowed to use the lnurl server. Comma separated.
    /// These are in addition to any domains stored in the database. The configured
    /// domains here will be added to the database on startup.
    #[arg(long, default_value = "localhost:8080")]
    pub domains: String,

    /// Nostr private key for zaps. If not set, zap requests will be ignored.
    #[arg(long)]
    pub nsec: Option<String>,

    /// Base64 encoded DER format CA certificate without begin/end certificate markers.
    /// If set, the server will use this certificate to validate api keys.
    #[arg(long)]
    pub ca_cert: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let args = Args::parse();
    let config_file = std::fs::canonicalize(&args.config).ok();
    let mut figment = Figment::new().merge(Serialized::defaults(args));
    if let Some(config_file) = &config_file {
        figment = figment.merge(Toml::file(config_file));
    }

    let args: Args = figment.merge(Env::prefixed("BREEZ_LNURL_")).extract()?;

    tracing_subscriber::registry()
        .with(EnvFilter::new(&args.log_level))
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stdout))
        .init();

    if let Some(config_file) = &config_file {
        info!(
            "starting lnurl server with config file: {}",
            config_file.display()
        );
    } else {
        info!("starting lnurl server without config file");
    }

    if args.db_url.trim().to_lowercase().starts_with("postgres") {
        let pool = PgPool::connect(&args.db_url)
            .await
            .map_err(|e| anyhow!("failed to create connection pool: {:?}", e))?;

        if args.auto_migrate {
            debug!("running postgres database migrations");
            postgresql::run_migrations(&pool).await?;
            debug!("finished running postgres database migrations");
        } else {
            debug!("skipping postgres database migrations");
        }
        let repository = postgresql::LnurlRepository::new(pool);
        run_server(args, repository).await?;
    } else {
        let pool = SqlitePool::connect(&args.db_url)
            .await
            .map_err(|e| anyhow!("failed to create connection pool: {:?}", e))?;

        if args.auto_migrate {
            debug!("running sqlite database migrations");
            sqlite::run_migrations(&pool).await?;
            debug!("finished running sqlite database migrations");
        } else {
            debug!("skipping sqlite database migrations");
        }
        let repository = sqlite::LnurlRepository::new(pool);
        run_server(args, repository).await?;
    }

    Ok(())
}

#[allow(clippy::too_many_lines)]
async fn run_server<DB>(args: Args, repository: DB) -> Result<(), anyhow::Error>
where
    DB: LnurlRepository + Clone + Send + Sync + 'static,
{
    let auth_seed: [u8; 32] = rand::random();

    let spark_config = SparkWalletConfig::default_config(args.network);

    // Create shared infrastructure components
    let signer = Arc::new(DefaultSigner::new(&auth_seed, args.network)?);
    let session_manager = Arc::new(InMemorySessionManager::default());
    let connection_manager: Arc<dyn spark::operator::rpc::ConnectionManager> =
        Arc::new(DefaultConnectionManager::new());
    let coordinator = spark_config.operator_pool.get_coordinator().clone();
    let service_provider = Arc::new(ServiceProvider::new(
        spark_config.service_provider_config.clone(),
        signer.clone(),
        session_manager.clone(),
    ));

    // Create wallet using shared signer
    let wallet = Arc::new(
        spark_wallet::SparkWallet::new(
            spark_config.clone(),
            signer.clone(),
            session_manager.clone(),
            Arc::new(InMemoryTreeStore::default()),
            Arc::new(InMemoryTokenOutputStore::default()),
            Arc::clone(&connection_manager),
            None,
            true,
            None,
        )
        .await?,
    );

    let config_domains: Vec<String> = args
        .domains
        .split(',')
        .map(|d| d.trim().to_lowercase())
        .filter(|d| !d.is_empty())
        .collect();

    for domain in &config_domains {
        repository.add_domain(domain).await?;
        debug!("ensured domain '{}' exists in database", domain);
    }

    let domains: HashSet<String> = repository.list_domains().await?.into_iter().collect();
    info!("loaded {} allowed domains from database", domains.len());

    let ca_cert = args
        .ca_cert
        .map(|ca_cert_str| {
            let raw_ca = BASE64_STANDARD
                .decode(ca_cert_str.trim())
                .map_err(|e| anyhow!("failed to decode base64 ca_cert: {:?}", e))?;
            let (_, ca_cert) = X509Certificate::from_der(&raw_ca)
                .map_err(|e| anyhow!("failed to parse ca certificate: {e:?}"))?;
            Ok::<_, anyhow::Error>(ca_cert.as_raw().to_vec())
        })
        .transpose()?;

    let nostr_keys = args
        .nsec
        .map(|nsec| {
            let keys = nostr::Keys::from_str(&nsec)
                .map_err(|e| anyhow!("failed to parse nsec key: {:?}", e))?;
            Ok::<_, anyhow::Error>(keys)
        })
        .transpose()?;

    let subscribed_keys = Arc::new(Mutex::new(HashSet::new()));

    // Create watch channel for triggering background processing
    let (invoice_paid_trigger, invoice_paid_rx) = watch::channel(());

    // Initialize invoice subscriptions and background processor if nostr keys are provided
    if let Some(nostr_keys) = &nostr_keys {
        // Start background processor for zap receipt publishing
        background::start_background_processor(
            repository.clone(),
            nostr_keys.clone(),
            invoice_paid_rx,
        );

        // Subscribe to users with unexpired invoices for payment monitoring
        for user in repository.get_invoice_monitored_users().await? {
            let user_pubkey = bitcoin::secp256k1::PublicKey::from_str(&user)
                .map_err(|e| anyhow!("failed to parse user pubkey: {e:?}"))?;

            background::create_rpc_client_and_subscribe(
                repository.clone(),
                user_pubkey,
                &connection_manager,
                &coordinator,
                signer.clone(),
                session_manager.clone(),
                Arc::clone(&service_provider),
                nostr_keys.clone(),
                Arc::clone(&subscribed_keys),
                invoice_paid_trigger.clone(),
            )
            .await?;
        }

        // Also subscribe for legacy zap monitoring (users with unexpired zaps)
        for user in repository.get_zap_monitored_users().await? {
            let user_pubkey = bitcoin::secp256k1::PublicKey::from_str(&user)
                .map_err(|e| anyhow!("failed to parse user pubkey: {e:?}"))?;

            zap::create_rpc_client_and_subscribe(
                repository.clone(),
                user_pubkey,
                &connection_manager,
                &coordinator,
                signer.clone(),
                session_manager.clone(),
                Arc::clone(&service_provider),
                nostr_keys.clone(),
                Arc::clone(&subscribed_keys),
            )
            .await?;
        }
    }

    let state = State {
        db: repository,
        wallet,
        scheme: args.scheme,
        min_sendable: args.min_sendable,
        max_sendable: args.max_sendable,
        include_spark_address: args.include_spark_address,
        domains,
        nostr_keys,
        ca_cert,
        connection_manager,
        coordinator,
        signer,
        session_manager,
        service_provider,
        subscribed_keys,
        invoice_paid_trigger,
    };

    let server_router = Router::new()
        .route(
            "/lnurlpay/available/{identifier}",
            get(LnurlServer::<DB>::available),
        )
        .route("/lnurlpay/{pubkey}", post(LnurlServer::<DB>::register))
        .route("/lnurlpay/{pubkey}", delete(LnurlServer::<DB>::unregister))
        .route(
            "/lnurlpay/{pubkey}/recover",
            post(LnurlServer::<DB>::recover),
        )
        .route(
            "/lnurlpay/{pubkey}/metadata",
            get(LnurlServer::<DB>::list_metadata),
        )
        .route(
            "/lnurlpay/{pubkey}/metadata/{payment_hash}/zap",
            post(LnurlServer::<DB>::publish_zap_receipt),
        )
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::auth::<DB>,
        ))
        .route(
            "/.well-known/lnurlp/{identifier}",
            get(LnurlServer::<DB>::handle_lnurl_pay),
        )
        .route(
            "/lnurlp/{identifier}",
            get(LnurlServer::<DB>::handle_lnurl_pay),
        )
        .route(
            "/lnurlp/{identifier}/invoice",
            get(LnurlServer::<DB>::handle_invoice),
        )
        .route("/verify/{payment_hash}", get(LnurlServer::<DB>::verify))
        .layer(Extension(state))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_headers(Any)
                .allow_methods([Method::GET, Method::POST, Method::DELETE, Method::OPTIONS]),
        )
        .layer(DefaultBodyLimit::max(1_000_000));

    let listener = tokio::net::TcpListener::bind(args.address).await?;
    let server = axum::serve(listener, server_router.into_make_service());

    let graceful = server.with_graceful_shutdown(async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to create Ctrl+C shutdown signal");
    });

    // Await the server to receive the shutdown signal
    if let Err(e) = graceful.await {
        error!("shutdown error: {e}");
    }

    info!("lnurl server stopped");
    Ok(())
}
