//! Regtest-only test helpers: faucet-funded wallets, docker-backed PG/MySQL
//! tree stores, and the regtest-flavoured `build_sdk_*` builders.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use breez_sdk_spark::*;
use rand::RngCore;
use tempfile::TempDir;
use testcontainers::{ContainerAsync, runners::AsyncRunner};
use testcontainers_modules::mysql::Mysql;
use testcontainers_modules::postgres::Postgres;
use tokio::sync::{OnceCell, mpsc};
use tracing::{Instrument, info};

use super::{ChannelEventListener, wait_for_balance, wait_for_claimed_event};
use crate::SdkInstance;
use crate::faucet::RegtestFaucet;

/// Shared PostgreSQL container for tree store testing.
/// Started once on first access and kept alive for the process lifetime.
struct SharedPgContainer {
    _container: ContainerAsync<Postgres>,
    base_conn_str: String,
}

static PG_TREE_STORE_CONTAINER: OnceCell<SharedPgContainer> = OnceCell::const_new();
static TREE_STORE_DB_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Shared MySQL container for tree store testing.
/// Started once on first access and kept alive for the process lifetime.
struct SharedMysqlContainer {
    _container: ContainerAsync<Mysql>,
    /// Connection string up to (but not including) the path. Callers append `/<dbname>`.
    base_url: String,
}

static MYSQL_TREE_STORE_CONTAINER: OnceCell<SharedMysqlContainer> = OnceCell::const_new();
static MYSQL_TREE_STORE_DB_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Returns the base connection string for the shared postgres container,
/// starting the container on first call. Returns None if USE_POSTGRES_TREE_STORE is not set.
async fn get_postgres_tree_store_base_url() -> Option<&'static str> {
    if std::env::var("USE_POSTGRES_TREE_STORE").is_err() {
        return None;
    }
    let shared = PG_TREE_STORE_CONTAINER
        .get_or_init(|| async {
            info!("Starting shared PostgreSQL container for tree store testing...");
            let container = Postgres::default()
                .start()
                .await
                .expect("Failed to start PostgreSQL container for tree store");
            let port = container
                .get_host_port_ipv4(5432)
                .await
                .expect("Failed to get PostgreSQL container port");
            info!("Shared PostgreSQL tree store container started on port {port}");
            SharedPgContainer {
                _container: container,
                base_conn_str: format!(
                    "host=127.0.0.1 port={port} user=postgres password=postgres"
                ),
            }
        })
        .await;
    Some(&shared.base_conn_str)
}

/// Returns the base URL for the shared mysql container,
/// starting the container on first call. Returns None if USE_MYSQL_TREE_STORE is not set.
async fn get_mysql_tree_store_base_url() -> Option<&'static str> {
    if std::env::var("USE_MYSQL_TREE_STORE").is_err() {
        return None;
    }
    let shared = MYSQL_TREE_STORE_CONTAINER
        .get_or_init(|| async {
            info!("Starting shared MySQL container for tree store testing...");
            let container = Mysql::default()
                .start()
                .await
                .expect("Failed to start MySQL container for tree store");
            let port = container
                .get_host_port_ipv4(3306)
                .await
                .expect("Failed to get MySQL container port");
            info!("Shared MySQL tree store container started on port {port}");
            SharedMysqlContainer {
                _container: container,
                base_url: format!("mysql://root@127.0.0.1:{port}"),
            }
        })
        .await;
    Some(&shared.base_url)
}

/// A resolved storage backend. Captures the choice (and connection string for
/// PG/MySQL) so the same database can be reused across multiple SDK builds —
/// e.g. for `ReinitializableSdkInstance`, which expects storage to persist
/// across SDK reinitializations.
#[derive(Clone, Debug)]
pub enum BackendChoice {
    Sqlite,
    Postgres(String),
    Mysql(String),
}

/// Resolves which backend to use based on env vars. For PG/MySQL, allocates a
/// fresh database by incrementing the appropriate counter and ensuring the DB
/// exists; for SQLite returns [`BackendChoice::Sqlite`]. Call once per logical
/// storage scope and reuse the result across SDK builds that share that scope.
///
/// If both `USE_POSTGRES_TREE_STORE` and `USE_MYSQL_TREE_STORE` are set,
/// postgres wins (matches prior `apply_storage` behavior).
pub async fn resolve_backend_choice() -> Result<BackendChoice> {
    if let Some(base_url) = get_postgres_tree_store_base_url().await {
        let counter = TREE_STORE_DB_COUNTER.fetch_add(1, Ordering::Relaxed);
        let conn_str = format!("{base_url} dbname=ts_{counter}");
        ensure_postgres_database_exists(&conn_str).await?;
        return Ok(BackendChoice::Postgres(conn_str));
    }
    if let Some(base_url) = get_mysql_tree_store_base_url().await {
        let counter = MYSQL_TREE_STORE_DB_COUNTER.fetch_add(1, Ordering::Relaxed);
        let db_name = format!("ts_{counter}");
        let conn_str = format!("{base_url}/{db_name}");
        ensure_mysql_database_exists(base_url, &db_name).await?;
        return Ok(BackendChoice::Mysql(conn_str));
    }
    Ok(BackendChoice::Sqlite)
}

/// Attaches a previously-resolved [`BackendChoice`] to the builder.
async fn apply_backend_choice(
    builder: SdkBuilder,
    storage_dir: String,
    choice: &BackendChoice,
) -> Result<SdkBuilder> {
    match choice {
        BackendChoice::Sqlite => Ok(builder.with_default_storage(storage_dir)),
        BackendChoice::Postgres(conn_str) => {
            let pg_config = breez_sdk_spark::default_postgres_storage_config(conn_str.clone());
            let ctx = breez_sdk_spark::new_shared_sdk_context(breez_sdk_spark::SdkContextConfig {
                storage: Some(breez_sdk_spark::postgres_storage(pg_config)?),
                ..breez_sdk_spark::SdkContextConfig::new(breez_sdk_spark::Network::Regtest)
            })
            .await?;
            Ok(builder.with_shared_context(ctx))
        }
        BackendChoice::Mysql(conn_str) => {
            let my_config = breez_sdk_spark::default_mysql_storage_config(conn_str.clone());
            let ctx = breez_sdk_spark::new_shared_sdk_context(breez_sdk_spark::SdkContextConfig {
                storage: Some(breez_sdk_spark::mysql_storage(my_config)?),
                ..breez_sdk_spark::SdkContextConfig::new(breez_sdk_spark::Network::Regtest)
            })
            .await?;
            Ok(builder.with_shared_context(ctx))
        }
    }
}

/// If USE_POSTGRES_TREE_STORE or USE_MYSQL_TREE_STORE is set, creates a unique
/// database and attaches the corresponding backend to the builder. Otherwise
/// sets default storage with the given directory. Exactly one storage
/// configuration is applied; if both env vars are set, postgres wins.
pub(crate) async fn apply_storage(builder: SdkBuilder, storage_dir: String) -> Result<SdkBuilder> {
    let choice = resolve_backend_choice().await?;
    apply_backend_choice(builder, storage_dir, &choice).await
}

/// Build and initialize a BreezSDK instance for testing
///
/// # Arguments
/// * `storage_dir` - Directory path for SDK storage
/// * `seed_bytes` - 32-byte seed for deterministic wallet generation
/// * `temp_dir` - Optional TempDir to keep alive (prevents premature deletion)
///
/// # Returns
/// An SdkInstance containing the SDK, event channel, and optional TempDir
pub async fn build_sdk_with_dir(
    storage_dir: String,
    seed_bytes: [u8; 32],
    temp_dir: Option<tempfile::TempDir>,
) -> Result<SdkInstance> {
    let mut config = default_config(Network::Regtest);
    config.api_key = None; // Regtest: no API key needed
    config.lnurl_domain = None; // Avoid lnurl server in tests
    config.prefer_spark_over_lightning = true; // prefer spark transfers when possible
    config.sync_interval_secs = 5; // Faster syncing for tests
    config.real_time_sync_server_url = None; // Disable real-time sync for tests

    let seed = Seed::Entropy(seed_bytes.to_vec());
    let builder = SdkBuilder::new(config, seed);
    let builder = apply_storage(builder, storage_dir).await?;
    let sdk = builder.build().await?;

    // Set up event listener
    let (tx, rx) = mpsc::channel(100);
    let event_listener = Box::new(ChannelEventListener { tx });
    let _listener_id = sdk.add_event_listener(event_listener).await;

    // Ensure initial sync completes
    let _ = sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(true),
        })
        .await?;

    Ok(SdkInstance {
        sdk,
        events: rx,
        span: tracing::Span::current(),
        temp_dir,
        data_sync_fixture: None,
        lnurl_fixture: None,
        turnkey_guard: None,
    })
}

/// Build and initialize a BreezSDK instance for testing (without TempDir management)
///
/// # Arguments
/// * `storage_dir` - Directory path for SDK storage
/// * `seed_bytes` - 32-byte seed for deterministic wallet generation
///
/// # Returns
/// An SdkInstance containing the SDK and event channel
pub async fn build_sdk(storage_dir: String, seed_bytes: [u8; 32]) -> Result<SdkInstance> {
    build_sdk_with_dir(storage_dir, seed_bytes, None).await
}

/// Build and initialize a BreezSDK instance attached to a shared SdkContext.
///
/// The supplied `Arc<SdkContext>` bundles both the shared HTTP client (SSP /
/// chain / LNURL / JWT) and the operator gRPC channels, so a single shared
/// handle covers every cross-instance sharing scenario.
pub async fn build_sdk_with_shared_context(
    storage_dir: String,
    seed_bytes: [u8; 32],
    context: Arc<SdkContext>,
    temp_dir: Option<TempDir>,
) -> Result<SdkInstance> {
    let mut config = default_config(Network::Regtest);
    config.api_key = None;
    config.lnurl_domain = None;
    config.prefer_spark_over_lightning = true;
    config.sync_interval_secs = 5;
    config.real_time_sync_server_url = None;

    let seed = Seed::Entropy(seed_bytes.to_vec());
    let builder = SdkBuilder::new(config, seed)
        .with_shared_context(context)
        .with_default_storage(storage_dir);
    let sdk = builder.build().await?;

    let (tx, rx) = mpsc::channel(100);
    let event_listener = Box::new(ChannelEventListener { tx });
    let _listener_id = sdk.add_event_listener(event_listener).await;

    let _ = sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(true),
        })
        .await?;

    Ok(SdkInstance {
        sdk,
        events: rx,
        span: tracing::Span::current(),
        temp_dir,
        data_sync_fixture: None,
        lnurl_fixture: None,
        turnkey_guard: None,
    })
}

/// Build and initialize a BreezSDK instance with a custom config override
///
/// Allows tests to tweak configuration fields (e.g., `max_deposit_claim_fee`).
/// Common test defaults (no API key, no lnurl, faster sync, prefer spark) are applied
/// on top unless explicitly set in the provided config.
pub async fn build_sdk_with_custom_config(
    storage_dir: String,
    seed_bytes: [u8; 32],
    config: Config,
    temp_dir: Option<tempfile::TempDir>,
    apply_sensible_test_defaults: bool,
) -> Result<SdkInstance> {
    build_sdk_with_custom_config_and_backend(
        storage_dir,
        seed_bytes,
        config,
        temp_dir,
        apply_sensible_test_defaults,
        None,
    )
    .await
}

/// Like [`build_sdk_with_custom_config`], but accepts an optional pre-resolved
/// [`BackendChoice`] that pins which database the SDK opens. When `None`, the
/// backend is selected fresh (via the usual `USE_*_TREE_STORE` env-var logic),
/// which allocates a new PG/MySQL database. Passing `Some(_)` lets callers
/// reuse the same database across multiple builds — required for tests that
/// reinit an SDK and expect persisted state to survive.
pub async fn build_sdk_with_custom_config_and_backend(
    storage_dir: String,
    seed_bytes: [u8; 32],
    mut config: Config,
    temp_dir: Option<tempfile::TempDir>,
    apply_sensible_test_defaults: bool,
    backend: Option<BackendChoice>,
) -> Result<SdkInstance> {
    // Apply sensible test defaults if not already configured
    if config.api_key.is_some() && matches!(config.network, Network::Regtest) {
        // In regtest we don't need an API key; drop it if present to avoid network calls
        config.api_key = None;
    }
    config.prefer_spark_over_lightning = true;
    if apply_sensible_test_defaults {
        config.sync_interval_secs = 5;
        config.real_time_sync_server_url = None;
        config.lnurl_domain = None;
    }

    let background_tasks_enabled = config.background_tasks_enabled;
    let seed = Seed::Entropy(seed_bytes.to_vec());

    let builder = SdkBuilder::new(config, seed);
    let builder = match backend {
        Some(choice) => apply_backend_choice(builder, storage_dir, &choice).await?,
        None => apply_storage(builder, storage_dir).await?,
    };
    let sdk = builder.build().await?;

    // Set up event listener
    let (tx, rx) = mpsc::channel(100);
    let event_listener = Box::new(ChannelEventListener { tx });
    let _listener_id = sdk.add_event_listener(event_listener).await;

    // Ensure initial sync completes. Server mode rejects `ensure_synced=true`
    // (there's no background sync to await), so drive it explicitly instead.
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
        events: rx,
        span: tracing::Span::current(),
        temp_dir,
        data_sync_fixture: None,
        lnurl_fixture: None,
        turnkey_guard: None,
    })
}

/// Extracts the database name from a PostgreSQL connection string.
///
/// Looks for the `dbname=<value>` parameter in a whitespace-separated
/// connection string and returns the value if found.
///
/// # Arguments
/// * `conn_str` - PostgreSQL connection string (key=value pairs)
///
/// # Returns
/// The database name if a `dbname` parameter is present
fn extract_dbname(conn_str: &str) -> Option<String> {
    for part in conn_str.split_whitespace() {
        if let Some((key, value)) = part.split_once('=')
            && key == "dbname"
        {
            return Some(value.to_string());
        }
    }
    None
}

/// Creates a PostgreSQL connection string for the default 'postgres' database.
/// This is used to connect and create other databases.
fn postgres_admin_conn_str(conn_str: &str) -> String {
    let mut parts: Vec<String> = Vec::new();

    for part in conn_str.split_whitespace() {
        if let Some((key, value)) = part.split_once('=') {
            if key == "dbname" {
                parts.push("dbname=postgres".to_string());
            } else {
                parts.push(format!("{key}={value}"));
            }
        }
    }

    // If no dbname was found, add postgres
    if !parts.iter().any(|p| p.starts_with("dbname=")) {
        parts.push("dbname=postgres".to_string());
    }

    parts.join(" ")
}

/// Ensures a PostgreSQL database exists, creating it if necessary.
///
/// Connects to the 'postgres' admin database and creates the target database
/// if it doesn't exist. This is useful for benchmarks that need isolated
/// databases for each SDK instance.
///
/// # Arguments
/// * `conn_str` - PostgreSQL connection string for the target database
///
/// # Returns
/// Ok if database exists or was created successfully
pub async fn ensure_postgres_database_exists(conn_str: &str) -> Result<()> {
    let db_name = extract_dbname(conn_str).unwrap_or_else(|| "postgres".to_string());
    let admin_conn_str = postgres_admin_conn_str(conn_str);

    info!("Ensuring database '{}' exists...", db_name);

    // Connect to postgres admin database
    let (client, connection) = tokio_postgres::connect(&admin_conn_str, tokio_postgres::NoTls)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to postgres admin database: {e}"))?;

    // Spawn the connection handler
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            tracing::error!("Postgres connection error: {}", e);
        }
    });

    // Check if database exists
    let row = client
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = $1)",
            &[&db_name],
        )
        .await
        .map_err(|e| anyhow::anyhow!("Failed to check if database exists: {e}"))?;

    let exists: bool = row.get(0);

    if !exists {
        info!("Creating database '{}'...", db_name);
        // CREATE DATABASE cannot be run in a transaction, so we use simple_query
        client
            .simple_query(&format!("CREATE DATABASE {db_name}"))
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create database '{}': {e}", db_name))?;
        info!("Database '{}' created successfully", db_name);
    } else {
        info!("Database '{}' already exists", db_name);
    }

    Ok(())
}

/// Ensures the named MySQL database exists, creating it if necessary.
///
/// `admin_url` should be the connection string up to (but not including) the
/// `/<dbname>` path, e.g. `mysql://root@127.0.0.1:33060`. The function connects
/// to the server's default `mysql` database to issue `CREATE DATABASE`.
pub async fn ensure_mysql_database_exists(admin_url: &str, db_name: &str) -> Result<()> {
    use mysql_async::prelude::*;

    info!("Ensuring MySQL database '{}' exists...", db_name);

    let admin_conn_str = format!("{admin_url}/mysql");
    let pool = mysql_async::Pool::from_url(admin_conn_str.as_str())
        .map_err(|e| anyhow::anyhow!("Failed to parse MySQL admin URL: {e}"))?;
    let mut conn = pool
        .get_conn()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to MySQL admin database: {e}"))?;

    // CREATE DATABASE IF NOT EXISTS is idempotent and safe to call concurrently.
    // We don't parameterize the database name because identifiers can't be bound;
    // we control db_name (it's of the form ts_<u64>), so injection isn't a risk.
    let stmt = format!("CREATE DATABASE IF NOT EXISTS `{db_name}`");
    conn.query_drop(stmt)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create MySQL database '{}': {e}", db_name))?;

    drop(conn);
    pool.disconnect()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to disconnect MySQL admin pool: {e}"))?;

    info!("MySQL database '{}' is ready", db_name);
    Ok(())
}

/// Drops a PostgreSQL database if it exists.
///
/// Connects to the 'postgres' admin database and drops the target database
/// if it exists. This is useful for cleaning up benchmark databases.
///
/// # Arguments
/// * `conn_str` - PostgreSQL connection string for the target database
///
/// # Returns
/// Ok if database was dropped or didn't exist
pub async fn drop_postgres_database(conn_str: &str) -> Result<()> {
    let db_name = extract_dbname(conn_str).unwrap_or_else(|| "postgres".to_string());

    // Don't drop the postgres admin database
    if db_name == "postgres" {
        return Ok(());
    }

    let admin_conn_str = postgres_admin_conn_str(conn_str);

    info!("Dropping database '{}' if exists...", db_name);

    // Connect to postgres admin database
    let (client, connection) = tokio_postgres::connect(&admin_conn_str, tokio_postgres::NoTls)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to postgres admin database: {e}"))?;

    // Spawn the connection handler
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            tracing::error!("Postgres connection error: {}", e);
        }
    });

    // Check if database exists
    let row = client
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = $1)",
            &[&db_name],
        )
        .await
        .map_err(|e| anyhow::anyhow!("Failed to check if database exists: {e}"))?;

    let exists: bool = row.get(0);

    if exists {
        // Terminate existing connections to the database
        client
            .simple_query(&format!(
                "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = '{db_name}'"
            ))
            .await
            .ok(); // Ignore errors - connections might already be gone

        // DROP DATABASE cannot be run in a transaction, so we use simple_query
        client
            .simple_query(&format!("DROP DATABASE {db_name}"))
            .await
            .map_err(|e| anyhow::anyhow!("Failed to drop database '{}': {e}", db_name))?;
        info!("Database '{}' dropped successfully", db_name);
    } else {
        info!("Database '{}' does not exist, nothing to drop", db_name);
    }

    Ok(())
}

/// Drops a MySQL database if it exists.
///
/// `conn_str` is a MySQL URL of the form `mysql://user:pass@host:port/<dbname>`.
/// We connect to the server's default `mysql` admin database to issue the drop.
pub async fn drop_mysql_database(conn_str: &str) -> Result<()> {
    use mysql_async::prelude::*;

    let (admin_url, db_name) = split_mysql_url(conn_str)?;
    if db_name.is_empty() || db_name == "mysql" {
        return Ok(());
    }

    info!("Dropping MySQL database '{}' if exists...", db_name);

    let admin_conn_str = format!("{admin_url}/mysql");
    let pool = mysql_async::Pool::from_url(admin_conn_str.as_str())
        .map_err(|e| anyhow::anyhow!("Failed to parse MySQL admin URL: {e}"))?;
    let mut conn = pool
        .get_conn()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to MySQL admin database: {e}"))?;

    // Identifiers can't be parameterized; db_name is supplied by the caller, so
    // we trust it (mirrors the postgres drop helper above).
    let stmt = format!("DROP DATABASE IF EXISTS `{db_name}`");
    conn.query_drop(stmt)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to drop MySQL database '{}': {e}", db_name))?;
    drop(conn);
    pool.disconnect()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to disconnect MySQL admin pool: {e}"))?;
    info!("MySQL database '{}' dropped (or did not exist)", db_name);
    Ok(())
}

/// Splits `mysql://user:pass@host:port/<dbname>` into `(admin_url, db_name)`,
/// where `admin_url` is the URL minus the `/<dbname>` path.
pub fn split_mysql_url(conn_str: &str) -> Result<(String, String)> {
    let (scheme, rest) = conn_str
        .split_once("://")
        .ok_or_else(|| anyhow::anyhow!("Invalid MySQL URL (missing scheme): {conn_str}"))?;
    let (authority, path) = rest
        .split_once('/')
        .ok_or_else(|| anyhow::anyhow!("Invalid MySQL URL (missing dbname): {conn_str}"))?;
    let db_name = path.split('?').next().unwrap_or("").to_string();
    Ok((format!("{scheme}://{authority}"), db_name))
}

/// PostgreSQL tree store input for `build_sdk_with_tree_store_config`.
///
/// `ConnectionString` lazily ensures the database exists and builds a fresh
/// `SdkContext` for that one SDK. `SharedContext` reuses a pre-built
/// [`SdkContext`] across SDK instances (multi-tenant scoping isolates rows
/// by seed identity).
pub enum PostgresTreeStore {
    ConnectionString(String),
    SharedContext(std::sync::Arc<SdkContext>),
}

/// MySQL tree store input for `build_sdk_with_tree_store_config`. See
/// [`PostgresTreeStore`] for semantics.
pub enum MysqlTreeStore {
    ConnectionString(String),
    SharedContext(std::sync::Arc<SdkContext>),
}

/// Build and initialize a BreezSDK instance with optional PostgreSQL/MySQL
/// tree store.
///
/// Similar to `build_sdk_with_custom_config` but allows specifying a backend
/// connection or a pre-built shared pool. Useful for benchmarks that want to
/// test SDK performance with different tree store backends, or for multi-SDK
/// setups that share a single connection pool.
///
/// Multi-tenant scoping (rows scoped by seed identity) means multiple SDKs
/// can safely share one database / pool — pass `SharedPool` to do so.
///
/// # Arguments
/// * `storage_dir` - Directory path for SDK storage
/// * `seed_bytes` - 32-byte seed for deterministic wallet generation
/// * `config` - SDK configuration to use
/// * `temp_dir` - Optional TempDir to keep alive (prevents premature deletion)
/// * `apply_sensible_test_defaults` - Whether to apply test defaults to config
/// * `postgres_tree_store` - Optional PostgreSQL backend (connection or shared pool)
/// * `mysql_tree_store` - Optional MySQL backend (connection or shared pool)
///
/// # Returns
/// An SdkInstance containing the SDK, event channel, and optional TempDir
pub async fn build_sdk_with_tree_store_config(
    storage_dir: String,
    seed_bytes: [u8; 32],
    mut config: Config,
    temp_dir: Option<tempfile::TempDir>,
    apply_sensible_test_defaults: bool,
    postgres_tree_store: Option<PostgresTreeStore>,
    mysql_tree_store: Option<MysqlTreeStore>,
) -> Result<SdkInstance> {
    // Apply sensible test defaults if not already configured
    if config.api_key.is_some() && matches!(config.network, Network::Regtest) {
        // In regtest we don't need an API key; drop it if present to avoid network calls
        config.api_key = None;
    }
    // Speed up tests and prefer spark routing
    config.prefer_spark_over_lightning = true;
    config.sync_interval_secs = 5;
    if apply_sensible_test_defaults {
        config.real_time_sync_server_url = None;
        config.lnurl_domain = None;
    }

    let seed = Seed::Entropy(seed_bytes.to_vec());

    let mut builder = SdkBuilder::new(config, seed);

    match (postgres_tree_store, mysql_tree_store) {
        (Some(PostgresTreeStore::ConnectionString(conn_str)), None) => {
            ensure_postgres_database_exists(&conn_str).await?;
            let mut pg_config = breez_sdk_spark::default_postgres_storage_config(conn_str);
            pg_config.max_pool_size = 30;
            let ctx = breez_sdk_spark::new_shared_sdk_context(breez_sdk_spark::SdkContextConfig {
                storage: Some(breez_sdk_spark::postgres_storage(pg_config)?),
                ..breez_sdk_spark::SdkContextConfig::new(breez_sdk_spark::Network::Regtest)
            })
            .await?;
            builder = builder.with_shared_context(ctx);
        }
        (Some(PostgresTreeStore::SharedContext(ctx)), None) => {
            builder = builder.with_shared_context(ctx);
        }
        (None, Some(MysqlTreeStore::ConnectionString(conn_str))) => {
            let (admin_url, db_name) = split_mysql_url(&conn_str)?;
            ensure_mysql_database_exists(&admin_url, &db_name).await?;
            let mut my_config = breez_sdk_spark::default_mysql_storage_config(conn_str);
            my_config.max_pool_size = 30;
            let ctx = breez_sdk_spark::new_shared_sdk_context(breez_sdk_spark::SdkContextConfig {
                storage: Some(breez_sdk_spark::mysql_storage(my_config)?),
                ..breez_sdk_spark::SdkContextConfig::new(breez_sdk_spark::Network::Regtest)
            })
            .await?;
            builder = builder.with_shared_context(ctx);
        }
        (None, Some(MysqlTreeStore::SharedContext(ctx))) => {
            builder = builder.with_shared_context(ctx);
        }
        (None, None) => {
            builder = apply_storage(builder, storage_dir).await?;
        }
        (Some(_), Some(_)) => {
            anyhow::bail!("only one of postgres_tree_store / mysql_tree_store may be set");
        }
    }

    let sdk = builder.build().await?;

    // Set up event listener
    let (tx, rx) = mpsc::channel(100);
    let event_listener = Box::new(ChannelEventListener { tx });
    let _listener_id = sdk.add_event_listener(event_listener).await;

    // Force an initial full sync. Works in both modes (auto sync on or off)
    // because `sync_wallet` drives the coordinator directly rather than
    // waiting on the auto-sync initial-synced watcher.
    sdk.sync_wallet(breez_sdk_spark::SyncWalletRequest {})
        .await?;

    Ok(SdkInstance {
        sdk,
        events: rx,
        span: tracing::Span::current(),
        temp_dir,
        data_sync_fixture: None,
        lnurl_fixture: None,
        turnkey_guard: None,
    })
}

/// Build and initialize a BreezSDK instance from a BIP-39 mnemonic phrase
///
/// This is used for wallet recovery testing, where we need to restore a wallet
/// from its mnemonic and verify that all historical payments are correctly synced.
///
/// # Arguments
/// * `storage_dir` - Directory path for SDK storage
/// * `mnemonic` - BIP-39 mnemonic phrase (12 or 24 words)
/// * `passphrase` - Optional BIP-39 passphrase
/// * `temp_dir` - Optional TempDir to keep alive (prevents premature deletion)
///
/// # Returns
/// An SdkInstance containing the SDK, event channel, and optional TempDir
pub async fn build_sdk_from_mnemonic(
    storage_dir: String,
    mnemonic: String,
    passphrase: Option<String>,
    temp_dir: Option<tempfile::TempDir>,
) -> Result<SdkInstance> {
    let mut config = default_config(Network::Regtest);
    config.api_key = None; // Regtest: no API key needed
    config.lnurl_domain = None; // Avoid lnurl server in tests
    config.prefer_spark_over_lightning = true; // prefer spark transfers when possible
    config.sync_interval_secs = 5; // Faster syncing for tests
    config.real_time_sync_server_url = None; // Disable real-time sync for tests

    let seed = Seed::Mnemonic {
        mnemonic,
        passphrase,
    };
    let builder = SdkBuilder::new(config, seed);
    let builder = apply_storage(builder, storage_dir).await?;
    let sdk = builder.build().await?;

    // Set up event listener
    let (tx, rx) = mpsc::channel(100);
    let event_listener = Box::new(ChannelEventListener { tx });
    let _listener_id = sdk.add_event_listener(event_listener).await;

    // Ensure initial sync completes
    let _ = sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(true),
        })
        .await?;

    Ok(SdkInstance {
        sdk,
        events: rx,
        span: tracing::Span::current(),
        temp_dir,
        data_sync_fixture: None,
        lnurl_fixture: None,
        turnkey_guard: None,
    })
}

/// Build SDK instance from a mnemonic via the external-signer path.
///
/// Uses the default external signers from `default_external_signers` with
/// `SdkBuilder::new_with_signer`, so the external signer surface and its FFI
/// type conversions are exercised end to end. Key derivation matches the seed
/// path: an SDK built either way from the same mnemonic is the same wallet.
///
/// # Arguments
/// * `storage_dir` - Directory path for SDK storage
/// * `mnemonic` - BIP39 mnemonic phrase for the signer
/// * `temp_dir` - Optional TempDir to keep alive
///
/// # Returns
/// An SdkInstance with SDK initialized via SdkBuilder::new_with_signer
pub async fn build_sdk_with_external_signer(
    storage_dir: String,
    mnemonic: String,
    temp_dir: Option<TempDir>,
) -> Result<SdkInstance> {
    let mut config = default_config(Network::Regtest);
    config.api_key = None;
    config.lnurl_domain = None;
    config.prefer_spark_over_lightning = true;
    config.sync_interval_secs = 5;
    config.real_time_sync_server_url = None;

    let signers = default_external_signers(mnemonic, None, Network::Regtest, None)?;
    let builder = SdkBuilder::new_with_signer(config, signers.breez_signer, signers.spark_signer);
    let builder = apply_storage(builder, storage_dir).await?;
    let sdk = builder.build().await?;

    // Set up event listener
    let (tx, rx) = mpsc::channel(100);
    let event_listener = Box::new(ChannelEventListener { tx });
    let _listener_id = sdk.add_event_listener(event_listener).await;

    // Ensure initial sync completes
    let _ = sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(true),
        })
        .await?;

    Ok(SdkInstance {
        sdk,
        events: rx,
        span: tracing::Span::current(),
        temp_dir,
        data_sync_fixture: None,
        lnurl_fixture: None,
        turnkey_guard: None,
    })
}
/// Which signer backend an SDK is built with, for backend-parametrized tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignerBackend {
    Seed,
    #[cfg(feature = "turnkey")]
    Turnkey,
}

/// The standard Regtest test config used by [`build_backend_sdk`]: no API key,
/// no LNURL server, prefer-spark routing, fast sync, and real-time sync off.
pub fn regtest_test_config() -> Config {
    let mut config = default_config(Network::Regtest);
    config.api_key = None;
    config.lnurl_domain = None;
    config.prefer_spark_over_lightning = true;
    config.sync_interval_secs = 5;
    config.real_time_sync_server_url = None;
    config
}

/// Builds a Regtest SDK for the given signer backend with the standard test
/// config, so one test body can run against multiple signers.
pub async fn build_backend_sdk(backend: SignerBackend) -> Result<SdkInstance> {
    build_backend_sdk_with_config(backend, regtest_test_config()).await
}

/// Like [`build_backend_sdk`], but with a caller-supplied config (e.g. to block
/// auto-claim with `max_deposit_claim_fee = None` for refund tests).
pub async fn build_backend_sdk_with_config(
    backend: SignerBackend,
    config: Config,
) -> Result<SdkInstance> {
    let temp = TempDir::new()?;
    let dir = temp.path().to_string_lossy().to_string();
    match backend {
        SignerBackend::Seed => {
            let mut seed = [0u8; 32];
            rand::thread_rng().fill_bytes(&mut seed);
            build_sdk_with_custom_config(dir, seed, config, Some(temp), false).await
        }
        #[cfg(feature = "turnkey")]
        SignerBackend::Turnkey => {
            crate::turnkey::build_sdk_with_turnkey(config, dir, Some(temp)).await
        }
    }
}

/// Waits for an `UnclaimedDeposits` event, returning the unclaimed deposits.
pub async fn wait_for_unclaimed_event(
    event_rx: &mut mpsc::Receiver<SdkEvent>,
    timeout_secs: u64,
) -> Result<Vec<DepositInfo>> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            anyhow::bail!("Timeout waiting for UnclaimedDeposits event");
        }
        match tokio::time::timeout(remaining, event_rx.recv()).await {
            Ok(Some(SdkEvent::UnclaimedDeposits { unclaimed_deposits })) => {
                return Ok(unclaimed_deposits);
            }
            Ok(Some(other)) => {
                info!("Ignored SDK event while waiting for unclaimed: {other:?}");
            }
            Ok(None) => anyhow::bail!("Event channel closed"),
            Err(_) => anyhow::bail!("Timeout waiting for UnclaimedDeposits event"),
        }
    }
}

/// Ensure SDK has at least the specified balance, funding if necessary
pub async fn ensure_funded(sdk_instance: &mut SdkInstance, min_balance: u64) -> Result<()> {
    let span = sdk_instance.span.clone();
    return ensure_funded_inner(sdk_instance, min_balance)
        .instrument(span)
        .await;
}

async fn ensure_funded_inner(sdk_instance: &mut SdkInstance, min_balance: u64) -> Result<()> {
    sdk_instance.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let info = sdk_instance
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?;
    if info.balance_sats < min_balance {
        let needed = min_balance - info.balance_sats;
        info!("Funding wallet via faucet: need {} sats", needed);
        receive_and_fund(sdk_instance, needed.clamp(10000, 50000), true).await?;
    }
    Ok(())
}

/// Server-mode variant of [`ensure_funded`].
///
/// Identical contract, but routes through `receive_and_fund(.., must_be_claimer=false)`
/// so the wait loop never depends on a `ClaimedDeposits` SDK event. Server-mode
/// SDKs don't run the spark-wallet `BackgroundProcessor`, so the event would
/// never fire on its own; `wait_for_balance` (which polls `sync_wallet` +
/// `get_info`) is the only mechanism that works in both modes.
pub async fn ensure_funded_via_polling(
    sdk_instance: &mut SdkInstance,
    min_balance: u64,
) -> Result<()> {
    let span = sdk_instance.span.clone();
    return ensure_funded_via_polling_inner(sdk_instance, min_balance)
        .instrument(span)
        .await;
}

async fn ensure_funded_via_polling_inner(
    sdk_instance: &mut SdkInstance,
    min_balance: u64,
) -> Result<()> {
    sdk_instance.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let info = sdk_instance
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?;
    if info.balance_sats < min_balance {
        let needed = min_balance - info.balance_sats;
        info!("Funding wallet via faucet (polling): need {} sats", needed);
        receive_and_fund(sdk_instance, needed.clamp(10000, 50000), false).await?;
    }
    Ok(())
}

/// Get a deposit address and fund it from the faucet in one operation
///
/// This helper generates a deposit address, funds it, and waits for the claim event.
///
/// # Arguments
/// * `sdk_instance` - The SdkInstance with SDK and event channel
/// * `amount_sats` - Amount to request from faucet
/// * `must_be_claimer` - Whether the SDK instance must be the claimer
///
/// # Returns
/// Tuple of (deposit_address, funding_txid)
pub async fn receive_and_fund(
    sdk_instance: &mut SdkInstance,
    amount_sats: u64,
    must_be_claimer: bool,
) -> Result<(String, String)> {
    let span = sdk_instance.span.clone();
    return receive_and_fund_inner(sdk_instance, amount_sats, must_be_claimer)
        .instrument(span)
        .await;
}

async fn receive_and_fund_inner(
    sdk_instance: &mut SdkInstance,
    amount_sats: u64,
    must_be_claimer: bool,
) -> Result<(String, String)> {
    let initial_balance = sdk_instance
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;
    // Get a static deposit address
    let receive = sdk_instance
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::BitcoinAddress { new_address: None },
        })
        .await?;

    let deposit_address = receive.payment_request;
    info!("Generated deposit address: {}", deposit_address);

    // Fund the address
    let faucet = RegtestFaucet::new()?;
    info!(
        "Funding address {} with {} sats from faucet",
        deposit_address, amount_sats
    );
    let txid = faucet.fund_address(&deposit_address, amount_sats).await?;

    info!(
        "Faucet sent funds in txid: {}, waiting for claim event...",
        txid
    );

    if must_be_claimer {
        wait_for_claimed_event(&mut sdk_instance.events, 180).await?;
        wait_for_balance(&sdk_instance.sdk, Some(initial_balance + 1), None, 20).await?;
    } else {
        wait_for_balance(&sdk_instance.sdk, Some(initial_balance + 1), None, 200).await?;
    }
    sdk_instance.sdk.sync_wallet(SyncWalletRequest {}).await?;

    Ok((deposit_address, txid))
}

/// Build and initialize a BreezSDK instance backed by PostgreSQL storage
///
/// # Arguments
/// * `connection_string` - PostgreSQL connection string
/// * `seed_bytes` - 32-byte seed for deterministic wallet generation
///
/// # Returns
/// An SdkInstance containing the SDK and event channel
pub async fn build_sdk_with_postgres(
    connection_string: &str,
    seed_bytes: [u8; 32],
) -> Result<SdkInstance> {
    let mut config = breez_sdk_spark::default_config(breez_sdk_spark::Network::Regtest);
    config.api_key = None;
    config.lnurl_domain = None;
    config.prefer_spark_over_lightning = true;
    config.sync_interval_secs = 5;
    config.real_time_sync_server_url = None;
    // Disable auto-optimization to avoid balance discrepancies when multiple instances run
    // concurrently. This is unrelated to storage sharing - even with separate storage, when
    // one instance performs a swap during optimization, other instances syncing with operators
    // may see a temporarily lower balance (old leaves spent, new leaves not yet visible).
    // Spark will soon add visibility into pending incoming funds, which should allow
    // removing this limitation.
    config.leaf_optimization_config.auto_enabled = false;

    let seed = breez_sdk_spark::Seed::Entropy(seed_bytes.to_vec());

    let postgres_config =
        breez_sdk_spark::default_postgres_storage_config(connection_string.to_string());
    let ctx = breez_sdk_spark::new_shared_sdk_context(breez_sdk_spark::SdkContextConfig {
        storage: Some(breez_sdk_spark::postgres_storage(postgres_config)?),
        ..breez_sdk_spark::SdkContextConfig::new(breez_sdk_spark::Network::Regtest)
    })
    .await?;

    let sdk = breez_sdk_spark::SdkBuilder::new(config, seed)
        .with_shared_context(ctx)
        .build()
        .await?;

    // Set up event listener
    let (tx, rx) = mpsc::channel(100);
    let event_listener = Box::new(ChannelEventListener { tx });
    let _listener_id = sdk.add_event_listener(event_listener).await;

    // Ensure initial sync completes
    let _ = sdk
        .get_info(breez_sdk_spark::GetInfoRequest {
            ensure_synced: Some(true),
        })
        .await?;

    Ok(SdkInstance {
        sdk,
        events: rx,
        span: tracing::Span::current(),
        temp_dir: None,
        data_sync_fixture: None,
        lnurl_fixture: None,
        turnkey_guard: None,
    })
}

/// Build and initialize a BreezSDK instance backed by `MySQL` storage.
///
/// Mirror of `build_sdk_with_postgres` for the `MySQL` backend.
///
/// # Arguments
/// * `connection_string` - MySQL URL connection string (e.g. `mysql://root@127.0.0.1:3306/dbname`)
/// * `seed_bytes` - 32-byte seed for deterministic wallet generation
pub async fn build_sdk_with_mysql(
    connection_string: &str,
    seed_bytes: [u8; 32],
) -> Result<SdkInstance> {
    let mut config = breez_sdk_spark::default_config(breez_sdk_spark::Network::Regtest);
    config.api_key = None;
    config.lnurl_domain = None;
    config.prefer_spark_over_lightning = true;
    config.sync_interval_secs = 5;
    config.real_time_sync_server_url = None;
    // Disable auto-optimization to avoid balance discrepancies when multiple instances run
    // concurrently. Same rationale as build_sdk_with_postgres.
    config.leaf_optimization_config.auto_enabled = false;

    let seed = breez_sdk_spark::Seed::Entropy(seed_bytes.to_vec());

    let mut mysql_config =
        breez_sdk_spark::default_mysql_storage_config(connection_string.to_string());
    mysql_config.max_pool_size = 30;
    let ctx = breez_sdk_spark::new_shared_sdk_context(breez_sdk_spark::SdkContextConfig {
        storage: Some(breez_sdk_spark::mysql_storage(mysql_config)?),
        ..breez_sdk_spark::SdkContextConfig::new(breez_sdk_spark::Network::Regtest)
    })
    .await?;

    let sdk = breez_sdk_spark::SdkBuilder::new(config, seed)
        .with_shared_context(ctx)
        .build()
        .await?;

    let (tx, rx) = mpsc::channel(100);
    let event_listener = Box::new(ChannelEventListener { tx });
    let _listener_id = sdk.add_event_listener(event_listener).await;

    let _ = sdk
        .get_info(breez_sdk_spark::GetInfoRequest {
            ensure_synced: Some(true),
        })
        .await?;

    Ok(SdkInstance {
        sdk,
        events: rx,
        span: tracing::Span::current(),
        temp_dir: None,
        data_sync_fixture: None,
        lnurl_fixture: None,
        turnkey_guard: None,
    })
}

/// Server-mode variant of [`build_sdk_with_postgres`].
///
/// Same wiring (shared PG pool, shared seed, regtest, no LNURL/RT-sync), but
/// the config comes from `default_server_config` so the SDK runs with
/// `background_tasks_enabled=false`: no periodic-sync loop, no spark-wallet
/// `BackgroundProcessor`, no event-driven payment refresh. Callers must drive
/// every reconciliation via `sync_wallet` (or use polling helpers like
/// [`ensure_funded_via_polling`] / [`wait_for_balance`]).
pub async fn build_sdk_with_postgres_server_mode(
    connection_string: &str,
    seed_bytes: [u8; 32],
) -> Result<SdkInstance> {
    let mut config = breez_sdk_spark::default_server_config(breez_sdk_spark::Network::Regtest);
    config.api_key = None;
    config.lnurl_domain = None;
    config.prefer_spark_over_lightning = true;
    config.real_time_sync_server_url = None;
    config.leaf_optimization_config.auto_enabled = false;
    config.token_optimization_config.auto_enabled = false;

    let seed = breez_sdk_spark::Seed::Entropy(seed_bytes.to_vec());

    let postgres_config =
        breez_sdk_spark::default_postgres_storage_config(connection_string.to_string());
    let ctx = breez_sdk_spark::new_shared_sdk_context(breez_sdk_spark::SdkContextConfig {
        storage: Some(breez_sdk_spark::postgres_storage(postgres_config)?),
        ..breez_sdk_spark::SdkContextConfig::new(breez_sdk_spark::Network::Regtest)
    })
    .await?;

    let sdk = breez_sdk_spark::SdkBuilder::new(config, seed)
        .with_shared_context(ctx)
        .build()
        .await?;

    let (tx, rx) = mpsc::channel(100);
    let event_listener = Box::new(ChannelEventListener { tx });
    let _listener_id = sdk.add_event_listener(event_listener).await;

    // `ensure_synced=true` is rejected when `background_tasks_enabled` is
    // false; issue an explicit sync_wallet so the initial tree-store hydrate
    // completes before returning.
    sdk.sync_wallet(SyncWalletRequest {}).await?;

    Ok(SdkInstance {
        sdk,
        events: rx,
        span: tracing::Span::current(),
        temp_dir: None,
        data_sync_fixture: None,
        lnurl_fixture: None,
        turnkey_guard: None,
    })
}

/// Server-mode variant of [`build_sdk_with_mysql`]. See
/// [`build_sdk_with_postgres_server_mode`] for the rationale.
pub async fn build_sdk_with_mysql_server_mode(
    connection_string: &str,
    seed_bytes: [u8; 32],
) -> Result<SdkInstance> {
    let mut config = breez_sdk_spark::default_server_config(breez_sdk_spark::Network::Regtest);
    config.api_key = None;
    config.lnurl_domain = None;
    config.prefer_spark_over_lightning = true;
    config.real_time_sync_server_url = None;
    config.leaf_optimization_config.auto_enabled = false;
    config.token_optimization_config.auto_enabled = false;

    let seed = breez_sdk_spark::Seed::Entropy(seed_bytes.to_vec());

    let mut mysql_config =
        breez_sdk_spark::default_mysql_storage_config(connection_string.to_string());
    mysql_config.max_pool_size = 30;
    let ctx = breez_sdk_spark::new_shared_sdk_context(breez_sdk_spark::SdkContextConfig {
        storage: Some(breez_sdk_spark::mysql_storage(mysql_config)?),
        ..breez_sdk_spark::SdkContextConfig::new(breez_sdk_spark::Network::Regtest)
    })
    .await?;

    let sdk = breez_sdk_spark::SdkBuilder::new(config, seed)
        .with_shared_context(ctx)
        .build()
        .await?;

    let (tx, rx) = mpsc::channel(100);
    let event_listener = Box::new(ChannelEventListener { tx });
    let _listener_id = sdk.add_event_listener(event_listener).await;

    sdk.sync_wallet(SyncWalletRequest {}).await?;

    Ok(SdkInstance {
        sdk,
        events: rx,
        span: tracing::Span::current(),
        temp_dir: None,
        data_sync_fixture: None,
        lnurl_fixture: None,
        turnkey_guard: None,
    })
}
