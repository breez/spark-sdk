use std::{path::PathBuf, sync::Arc};

use anyhow::anyhow;
use axum::{
    Extension, Router,
    extract::DefaultBodyLimit,
    http::{self, Method},
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
use spark_wallet::{DefaultSigner, Network, SparkWalletConfig};
use sqlx::{PgPool, SqlitePool};
use tower_http::cors::{Any, CorsLayer};
use tracing::{debug, error, info};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};
use x509_parser::prelude::{FromDer, X509Certificate};

use crate::{repository::LnurlRepository, routes::LnurlServer, state::State};

mod auth;
mod error;
mod postgresql;
mod repository;
mod routes;
mod sqlite;
mod state;
mod time;
mod user;

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

    /// List of domains that are allowed to use the lnurl server. Comma separated.
    #[arg(long, default_value = "localhost:8080")]
    pub domains: String,

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

async fn run_server<DB>(args: Args, repository: DB) -> Result<(), anyhow::Error>
where
    DB: LnurlRepository + Clone + Send + Sync + 'static,
{
    let auth_seed: [u8; 32] = rand::random();
    let wallet = Arc::new(
        spark_wallet::SparkWallet::connect(
            SparkWalletConfig::default_config(args.network),
            DefaultSigner::new(&auth_seed, args.network)?,
        )
        .await?,
    );
    let domains = args
        .domains
        .split(',')
        .map(|d| d.trim().to_lowercase())
        .collect();

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
    let state = State {
        db: repository,
        wallet,
        scheme: args.scheme,
        min_sendable: args.min_sendable,
        max_sendable: args.max_sendable,
        domains,
        ca_cert,
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
            delete(LnurlServer::<DB>::recover),
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
        .layer(Extension(state))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_headers([http::header::CONTENT_TYPE, http::header::AUTHORIZATION])
                .allow_methods([
                    Method::GET,
                    Method::POST,
                    Method::PUT,
                    Method::DELETE,
                    Method::OPTIONS,
                ]),
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
