use crate::{
    partner_jwt::{JwtCache, JwtStore, RepoJwtStore},
    repository::LnurlRepository,
    routes::LnurlServer,
    state::State,
};
use anyhow::anyhow;
use axum::{
    Extension, Router,
    extract::DefaultBodyLimit,
    http::{Method, StatusCode},
    middleware,
    routing::{delete, get, post},
};
use base64::{Engine, prelude::BASE64_STANDARD};
use clap::{CommandFactory, FromArgMatches, Parser};
use figment::{
    Figment,
    providers::{Env, Format, Serialized, Toml},
};
use serde::{Deserialize, Serialize};
use spark::operator::rpc::DefaultConnectionManager;
use spark::session_store::InMemorySessionStore;
use spark::ssp::{ServiceProvider, SparkWalletWebhookEventType};
use spark::token::InMemoryTokenOutputStore;
use spark::tree::InMemoryTreeStore;
use spark_wallet::{DefaultSigner, Network, SparkSignerAdapter, SparkWalletConfig};
use sqlx::{PgPool, SqlitePool, sqlite::SqlitePoolOptions};
use std::collections::HashSet;
use std::str::FromStr;
use std::{path::PathBuf, sync::Arc};
use tokio::sync::{Mutex, watch};
use tower_http::cors::{Any, CorsLayer};
use tracing::{debug, error, info};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};
use x509_parser::prelude::{FromDer, X509Certificate};

mod auth;
mod domains;
mod error;
mod invoice_paid;
mod partner_jwt;
mod postgresql;
mod repository;
mod routes;
mod sqlite;
mod state;
mod time;
mod user;
mod webhook_notify;
mod webhooks;
mod zap;

fn default_user_agent() -> String {
    concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")).to_string()
}

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

    /// Scheme prefix for generated lnurl URLs only. The server binds plain HTTP
    /// and does not terminate TLS itself: run it behind a TLS-terminating proxy.
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
    #[cfg(feature = "dev")]
    #[arg(long, default_value = "false")]
    pub dev_dont_use_lnurl_include_spark_address: bool,

    /// List of domains that are allowed to use the lnurl server. Comma separated.
    /// These are in addition to any domains stored in the database. The configured
    /// domains here will be added to the database on startup.
    #[arg(long, default_value = "localhost:8080")]
    pub domains: String,

    /// Fallback Breez API key used to attribute lightning-address receives for
    /// any allowed domain that has no `api_key` of its own. Required on mainnet.
    #[arg(long)]
    pub default_api_key: Option<String>,

    /// Nostr private key for zaps. If not set, zap requests will be ignored.
    #[arg(long)]
    pub nsec: Option<String>,

    /// Base64 encoded DER format CA certificate without begin/end certificate markers.
    /// If set, the server will use this certificate to validate api keys.
    #[arg(long)]
    pub ca_cert: Option<String>,

    /// URL to fetch a comma-separated certificate revocation list from.
    #[arg(long)]
    pub crl_url: Option<String>,

    /// Domain for the webhook URL registered with the SSP.
    #[arg(long)]
    pub webhook_domain: Option<String>,

    /// Hex-encoded 32-byte seed used for SSP authentication.
    /// If not set, a random seed will be generated.
    #[arg(long)]
    pub ssp_auth_seed: Option<String>,

    /// Number of days to keep webhook deliveries (both succeeded and failed)
    /// for audit/debugging before they are cleaned up periodically.
    #[arg(long, default_value = "90")]
    pub webhook_delivery_ttl_days: u32,
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let matches = Args::command().get_matches();
    let args = Args::from_arg_matches(&matches)?;
    let config_file = std::fs::canonicalize(&args.config).ok();

    // Precedence, highest first: explicit CLI args > env > TOML > clap defaults.
    // The fully-parsed args (which carry clap's default for every unset flag) are
    // the lowest layer, so real env/TOML values override those defaults. The flags
    // the user actually typed are re-applied as the top layer via
    // `explicit_cli_overrides`, so they win over env and TOML.
    let mut figment = Figment::new().merge(Serialized::defaults(&args));
    if let Some(config_file) = &config_file {
        figment = figment.merge(Toml::file(config_file));
    }
    figment = figment.merge(Env::prefixed("BREEZ_LNURL_"));
    figment = figment.merge(Serialized::defaults(explicit_cli_overrides(
        &args, &matches,
    )));

    let args: Args = figment.extract()?;

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
        // For in-memory databases, limit to 1 connection so all queries share
        // the same database. Each separate connection to `:memory:` creates its
        // own independent database.
        let pool = if args.db_url.contains(":memory:") {
            SqlitePoolOptions::new()
                .max_connections(1)
                .connect(&args.db_url)
                .await
        } else {
            SqlitePool::connect(&args.db_url).await
        }
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

/// The flags the user set explicitly on the command line, as a serde map keyed
/// by field name. Flags left at their clap default are excluded so they do not
/// override values coming from the environment or the config file, letting
/// command-line arguments sit at the top of the precedence order.
fn explicit_cli_overrides(args: &Args, matches: &clap::ArgMatches) -> serde_json::Value {
    let explicit: HashSet<&str> = matches
        .ids()
        .filter(|id| {
            matches.value_source(id.as_str()) == Some(clap::parser::ValueSource::CommandLine)
        })
        .map(clap::Id::as_str)
        .collect();

    let serde_json::Value::Object(map) = serde_json::to_value(args).unwrap_or_default() else {
        return serde_json::Value::Object(serde_json::Map::new());
    };
    serde_json::Value::Object(
        map.into_iter()
            .filter(|(k, _)| explicit.contains(k.as_str()))
            .collect(),
    )
}

fn parse_auth_seed(hex_str: Option<&str>) -> Result<[u8; 32], anyhow::Error> {
    // Unset is a deliberate "generate an ephemeral identity" case. A malformed
    // seed is not: silently substituting a random identity would swap the
    // server's SSP-auth key without notice, so treat it as fatal.
    let Some(hex_str) = hex_str else {
        return Ok(rand::random());
    };
    let bytes = hex::decode(hex_str).map_err(|e| anyhow!("invalid ssp_auth_seed hex: {e}"))?;
    bytes
        .try_into()
        .map_err(|_| anyhow!("ssp_auth_seed must be 32 bytes"))
}

fn resolve_default_api_key(
    arg: Option<&str>,
    is_mainnet: bool,
) -> Result<Option<String>, anyhow::Error> {
    let key = arg
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(str::to_string);
    if is_mainnet && key.is_none() {
        return Err(anyhow!(
            "a default API key is required on mainnet: set --default-api-key (or BREEZ_LNURL_DEFAULT_API_KEY)"
        ));
    }
    Ok(key.filter(|_| is_mainnet))
}

#[allow(clippy::too_many_lines)]
async fn run_server<DB>(args: Args, repository: DB) -> Result<(), anyhow::Error>
where
    DB: LnurlRepository + webhooks::WebhookRepository + Clone + Send + Sync + 'static,
{
    let auth_seed = parse_auth_seed(args.ssp_auth_seed.as_deref())?;

    let mut spark_config = SparkWalletConfig::default_config(args.network);
    spark_config.service_provider_config.schema_endpoint = Some("graphql/spark/rc".to_string());
    // One HTTP client (one connection pool) shared by all SSP traffic.
    let ssp_http_client = platform_utils::create_http_client(Some(&default_user_agent()));

    // Create shared infrastructure components
    let signer = Arc::new(DefaultSigner::new(&auth_seed, args.network)?);
    // High-level Spark signer wrapping the in-process low-level signer, used by
    // the Spark wallet and service provider.
    let spark_signer: Arc<dyn spark_wallet::SparkSigner> =
        Arc::new(SparkSignerAdapter::new(signer.clone()));
    let session_store = Arc::new(InMemorySessionStore::default());
    let connection_manager: Arc<dyn spark::operator::rpc::ConnectionManager> =
        Arc::new(DefaultConnectionManager::new());
    let coordinator = spark_config.operator_pool.get_coordinator().clone();
    let service_provider = Arc::new(ServiceProvider::new_with_client(
        spark_config.service_provider_config.clone(),
        spark_signer.clone(),
        session_store.clone(),
        None,
        Arc::clone(&ssp_http_client),
    ));

    // Ensure config-provided domains exist, then start the domain refresher
    // (domain -> Breez API key). The partner JWT provider shares this map.
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

    // Partner attribution only works on mainnet: the Breez JWT endpoint is
    // mainnet-only, so regtest/testnet have no API keys or partner JWTs.
    let is_mainnet = matches!(args.network, Network::Mainnet);
    // Fallback key for domains without their own, so none are unattributed.
    // Mandatory on mainnet (fails startup if missing), ignored otherwise.
    let default_api_key = resolve_default_api_key(args.default_api_key.as_deref(), is_mainnet)?;

    let domains = domains::start(repository.clone(), is_mainnet, default_api_key.clone()).await?;

    // Shared partner-JWT cache (mainnet only). Its background task keeps a token
    // warm for every domain with its own api key (persisted to the DB) and one
    // for the default key.
    let jwt_cache = if is_mainnet {
        let store: Arc<dyn JwtStore> = Arc::new(RepoJwtStore(repository.clone()));
        Some(JwtCache::start(Arc::clone(&domains), default_api_key.clone(), store).await)
    } else {
        None
    };

    let default_jwt_provider = jwt_cache
        .as_ref()
        .filter(|_| default_api_key.is_some())
        .map(|c| c.default_provider() as Arc<dyn spark::header_provider::HeaderProvider>);
    let wallet = Arc::new(
        spark_wallet::SparkWallet::new(
            spark_config.clone(),
            spark_signer.clone(),
            session_store.clone(),
            Arc::new(InMemoryTreeStore::default()),
            Arc::new(InMemoryTokenOutputStore::default()),
            Arc::clone(&connection_manager),
            Some(Arc::clone(&ssp_http_client)),
            None,
            default_jwt_provider.clone(),
            default_jwt_provider,
            None,
        )
        .await?,
    );
    wallet.start_background_processing().await;

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

    let crl: HashSet<String> = if let Some(url) = &args.crl_url {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| anyhow!("failed to build crl http client: {e:?}"))?;
        let response = client
            .get(url)
            .send()
            .await
            .map_err(|e| anyhow!("failed to fetch crl from {url}: {e:?}"))?;
        // Guard against parsing an error page (404/500 body) as revocation entries.
        let response = response
            .error_for_status()
            .map_err(|e| anyhow!("crl fetch from {url} returned an error status: {e:?}"))?;
        let body = response
            .text()
            .await
            .map_err(|e| anyhow!("failed to read crl response body: {e:?}"))?;
        body.split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect()
    } else {
        HashSet::new()
    };

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

    // Create a shared HTTP client for webhook delivery. reqwest's default pool
    // settings keep connections warm and HTTP/2 multiplexes requests per host.
    let http_client = reqwest::Client::new();

    let webhook_service = webhooks::WebhookService::new(repository.clone());

    // Load webhook endpoint configs (domain → {url, secret}) and start
    // a background refresher that keeps them in sync with the database.
    let webhook_config_cache = webhooks::config::start(repository.clone()).await?;

    // Start background processors.
    zap::start_background_processor(
        repository.clone(),
        nostr_keys.as_ref(),
        invoice_paid_rx.clone(),
    );
    webhooks::start_background_processor(
        repository.clone(),
        http_client,
        invoice_paid_rx,
        args.webhook_delivery_ttl_days,
        webhook_config_cache,
    );

    // Get or create a shared webhook secret persisted in the database.
    // All instances share the same secret so webhooks verify correctly
    // regardless of which instance receives them.
    let default_secret = hex::encode(rand::random::<[u8; 32]>());
    let webhook_secret = repository
        .get_or_create_setting("webhook_secret", &default_secret)
        .await?;

    if let Some(webhook_domain) = &args.webhook_domain {
        let webhook_url = format!("{}://{}/webhook", args.scheme, webhook_domain);
        register_webhook(
            Arc::clone(&service_provider),
            webhook_url,
            webhook_secret.clone(),
        );
    }

    let state = State {
        db: repository,
        webhook_service,
        wallet,
        is_mainnet,
        scheme: args.scheme,
        min_sendable: args.min_sendable,
        max_sendable: args.max_sendable,
        include_spark_address: {
            #[cfg(feature = "dev")]
            {
                args.dev_dont_use_lnurl_include_spark_address
            }
            #[cfg(not(feature = "dev"))]
            {
                false
            }
        },
        domains,
        nostr_keys,
        ca_cert,
        crl_url: args.crl_url,
        crl,
        connection_manager,
        coordinator,
        signer,
        session_store,
        service_provider,
        spark_config,
        ssp_http_client,
        jwt_cache,
        subscribed_keys,
        invoice_paid_trigger,
        webhook_secret,
    };

    let server_router = Router::new()
        .route(
            "/lnurlpay/available/{identifier}",
            get(LnurlServer::<DB>::available),
        )
        .route("/lnurlpay/{pubkey}", post(LnurlServer::<DB>::register))
        .route("/lnurlpay/{pubkey}", delete(LnurlServer::<DB>::unregister))
        .route(
            "/lnurlpay/{pubkey}/transfer",
            post(LnurlServer::<DB>::transfer),
        )
        .route(
            "/lnurlpay/{pubkey}/recover",
            post(LnurlServer::<DB>::recover),
        )
        .route(
            "/lnurlpay/{pubkey}/metadata",
            get(LnurlServer::<DB>::list_metadata),
        )
        .route(
            "/lnurlpay/{pubkey}/invoice-paid",
            post(LnurlServer::<DB>::invoice_paid),
        )
        .route(
            "/lnurlpay/{pubkey}/invoices-paid",
            post(LnurlServer::<DB>::invoices_paid),
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
        .route("/webhook", post(LnurlServer::<DB>::webhook))
        .route("/health", get(|| async { StatusCode::OK }))
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

fn register_webhook(service_provider: Arc<ServiceProvider>, webhook_url: String, secret: String) {
    tokio::spawn(async move {
        let mut delay = std::time::Duration::from_secs(1);
        let max_delay = std::time::Duration::from_mins(1);
        loop {
            info!("registering webhook with SSP at {}", webhook_url);
            match service_provider
                .register_wallet_webhook(
                    &webhook_url,
                    &secret,
                    vec![SparkWalletWebhookEventType::SparkLightningReceiveFinished],
                )
                .await
            {
                Ok(_) => {
                    info!("webhook registered successfully");
                    break;
                }
                Err(e) => {
                    error!(
                        "failed to register webhook with SSP: {:?}, retrying in {:?}",
                        e, delay
                    );
                    tokio::time::sleep(delay).await;
                    delay = delay.saturating_mul(2).min(max_delay);
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{Args, explicit_cli_overrides, parse_auth_seed, resolve_default_api_key};
    use clap::{CommandFactory, FromArgMatches};
    use figment::{Figment, providers::Serialized};

    #[test]
    fn explicit_cli_overrides_excludes_defaults() {
        let matches = Args::command()
            .try_get_matches_from(["lnurl", "--scheme", "http"])
            .expect("args parse");
        let args = Args::from_arg_matches(&matches).expect("from matches");
        let overrides = explicit_cli_overrides(&args, &matches);
        let obj = overrides.as_object().expect("object");

        // The flag the user typed is present.
        assert_eq!(obj.get("scheme").and_then(|v| v.as_str()), Some("http"));
        // Flags left at their clap default must not appear (else they would
        // clobber env/TOML values).
        assert!(!obj.contains_key("min_sendable"));
        assert!(!obj.contains_key("network"));
    }

    #[test]
    fn cli_wins_over_lower_layer_but_default_does_not() {
        // Mirrors main()'s layering. A second `Serialized` layer stands in for
        // the TOML/env sources: precedence is decided purely by merge order, so
        // this faithfully exercises "explicit CLI beats a lower layer" and
        // "a non-passed flag lets the lower layer beat the clap default".
        let matches = Args::command()
            .try_get_matches_from(["lnurl", "--scheme", "http"])
            .expect("args parse");
        let args = Args::from_arg_matches(&matches).expect("from matches");
        let lower_layer = serde_json::json!({ "scheme": "https", "min_sendable": 5000u64 });

        let resolved: Args = Figment::new()
            .merge(Serialized::defaults(&args))
            .merge(Serialized::defaults(lower_layer))
            .merge(Serialized::defaults(explicit_cli_overrides(
                &args, &matches,
            )))
            .extract()
            .expect("extract");

        assert_eq!(
            resolved.scheme, "http",
            "CLI-passed flag must win over lower layer"
        );
        assert_eq!(
            resolved.min_sendable, 5000,
            "un-passed flag must take the lower-layer value, not the clap default"
        );
    }

    #[test]
    fn auth_seed_valid_hex_is_parsed() {
        let hex = "11".repeat(32);
        let seed = parse_auth_seed(Some(&hex)).expect("valid 32-byte hex must parse");
        assert_eq!(seed, [0x11u8; 32]);
    }

    #[test]
    fn auth_seed_unset_generates_random() {
        // Unset is a deliberate ephemeral-identity case, not an error.
        assert!(parse_auth_seed(None).is_ok());
    }

    #[test]
    fn auth_seed_invalid_hex_is_fatal() {
        assert!(parse_auth_seed(Some("nothex")).is_err());
    }

    #[test]
    fn auth_seed_wrong_length_is_fatal() {
        // 31 bytes, valid hex but wrong length.
        assert!(parse_auth_seed(Some(&"22".repeat(31))).is_err());
    }

    #[test]
    fn default_api_key_required_on_mainnet() {
        // Present and trimmed on mainnet.
        assert_eq!(
            resolve_default_api_key(Some("  key  "), true).unwrap(),
            Some("key".to_string())
        );
        // Missing or blank is a startup error on mainnet.
        assert!(resolve_default_api_key(None, true).is_err());
        assert!(resolve_default_api_key(Some(""), true).is_err());
        assert!(resolve_default_api_key(Some("   "), true).is_err());
    }

    #[test]
    fn default_api_key_ignored_off_mainnet() {
        // No JWT endpoint off mainnet, so no key is required and any key is dropped.
        assert_eq!(resolve_default_api_key(None, false).unwrap(), None);
        assert_eq!(resolve_default_api_key(Some("key"), false).unwrap(), None);
    }
}
