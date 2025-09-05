use std::{path::PathBuf, sync::Arc};

use anyhow::anyhow;
use axum::{
    Extension, Router,
    extract::DefaultBodyLimit,
    http::{self, Method},
    routing::{delete, get, post},
};
use breez_sdk_spark::Network;
use clap::Parser;
use diesel::{
    SqliteConnection,
    r2d2::{ConnectionManager, Pool},
};
use figment::{
    Figment,
    providers::{Env, Format, Serialized, Toml},
};
use serde::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, serde_as};
use tower_http::cors::{Any, CorsLayer};
use tracing::{error, info};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use crate::state::State;

mod models;
mod routes;
mod sdk;
mod sqlite;
mod state;

#[serde_as]
#[derive(Clone, Parser, Debug, Serialize, Deserialize)]
#[command(version, about, long_about = None)]
struct Args {
    /// Address the lnurl server will listen on.
    #[arg(long, default_value = "127.0.0.1:8080")]
    pub address: core::net::SocketAddr,

    #[arg(long, default_value = "lnurl.conf")]
    pub config: PathBuf,

    /// Automatically apply migrations to the database.
    #[arg(long)]
    pub auto_migrate: bool,

    /// Connectionstring to the postgres database.
    #[arg(long, default_value = "")]
    pub db_url: String,

    /// Loglevel to use. Can be used to filter loges through the env filter
    /// format.
    #[arg(long, default_value = "info")]
    pub log_level: String,

    #[arg(long, default_value = "mainnet")]
    #[serde_as(as = "DisplayFromStr")]
    pub network: Network,

    #[arg(
        long,
        default_value = "all all all all all all all all all all all all"
    )]
    pub mnemonic: String,
    pub domain: String,
    pub min_sendable: u64,
    pub max_sendable: u64,
}

type LnurlServer = routes::LnurlServer<Pool<ConnectionManager<SqliteConnection>>>;

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

    match &config_file {
        Some(config_file) => info!(
            "starting lnurl server with config file: {}",
            config_file.display()
        ),
        None => info!("starting lnurl server without config file"),
    }

    // Create a connection manager for SQLite
    let manager = ConnectionManager::<SqliteConnection>::new(&args.db_url);

    // Create a connection pool
    let pool = Pool::builder()
        .build(manager)
        .map_err(|e| anyhow!("failed to create connection pool: {:?}", e))?;

    // Get a connection to run migrations
    let mut db_connection = pool
        .get()
        .map_err(|e| anyhow!("failed to get connection from pool: {:?}", e))?;

    match args.auto_migrate {
        true => sqlite::run_migrations(&mut db_connection)?,
        false => {
            if sqlite::has_migrations(&mut db_connection)? {
                return Err(anyhow::anyhow!(
                    "database has pending migrations, run with --auto-migrate to apply them, or apply them manually"
                ));
            }
        }
    }

    let state = State {
        db: Arc::new(pool),
        domain: args.domain,
        min_sendable: args.min_sendable,
        max_sendable: args.max_sendable,
    };

    let server_router = Router::new()
        .route("/lnurlpay/:pubkey", post(LnurlServer::register))
        .route("/lnurlpay/:pubkey", delete(LnurlServer::unregister))
        .route("/lnurlpay/:pubkey/recover", delete(LnurlServer::recover))
        .route(
            "/.well-known/lnurlp/:identifier",
            get(LnurlServer::handle_lnurl_pay),
        )
        .route("/lnurlp/:identifier", get(LnurlServer::handle_lnurl_pay))
        .route(
            "/lnurlp/:identifier/invoice",
            get(LnurlServer::handle_invoice),
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
        .layer(DefaultBodyLimit::max(1_000_000)); // max 1mb body size

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

    todo!();
}
