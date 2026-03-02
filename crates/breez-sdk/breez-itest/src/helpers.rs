use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use bitcoin::hashes::{Hash as _, sha256};
use breez_sdk_spark::*;
use rand::RngCore;
use testcontainers::{ContainerAsync, runners::AsyncRunner};
use testcontainers_modules::postgres::Postgres;
use tokio::sync::{OnceCell, mpsc};
use tracing::{Instrument, debug, info};

use crate::SdkInstance;
use crate::faucet::RegtestFaucet;
use tempdir::TempDir;

/// Shared PostgreSQL container for tree store testing.
/// Started once on first access and kept alive for the process lifetime.
struct SharedPgContainer {
    _container: ContainerAsync<Postgres>,
    base_conn_str: String,
}

static PG_TREE_STORE_CONTAINER: OnceCell<SharedPgContainer> = OnceCell::const_new();
static TREE_STORE_DB_COUNTER: AtomicU64 = AtomicU64::new(0);

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

/// If USE_POSTGRES_TREE_STORE is set, creates a unique database and attaches
/// a PostgreSQL tree store to the builder. Otherwise returns builder unchanged.
async fn apply_postgres_tree_store(builder: SdkBuilder) -> Result<SdkBuilder> {
    let Some(base_url) = get_postgres_tree_store_base_url().await else {
        return Ok(builder);
    };
    let counter = TREE_STORE_DB_COUNTER.fetch_add(1, Ordering::Relaxed);
    let conn_str = format!("{base_url} dbname=ts_{counter}");
    ensure_postgres_database_exists(&conn_str).await?;
    let pg_config = breez_sdk_spark::default_postgres_storage_config(conn_str);
    Ok(builder.with_postgres_tree_store(pg_config))
}

/// Event listener that forwards events to a channel
struct ChannelEventListener {
    tx: mpsc::Sender<SdkEvent>,
}

#[async_trait::async_trait]
impl EventListener for ChannelEventListener {
    async fn on_event(&self, event: SdkEvent) {
        info!("Received SDK event: {event}");
        let _ = self.tx.try_send(event);
    }
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
    temp_dir: Option<tempdir::TempDir>,
) -> Result<SdkInstance> {
    let mut config = default_config(Network::Regtest);
    config.api_key = None; // Regtest: no API key needed
    config.lnurl_domain = None; // Avoid lnurl server in tests
    config.prefer_spark_over_lightning = true; // prefer spark transfers when possible
    config.sync_interval_secs = 5; // Faster syncing for tests
    config.real_time_sync_server_url = None; // Disable real-time sync for tests

    let seed = Seed::Entropy(seed_bytes.to_vec());
    let builder = SdkBuilder::new(config, seed).with_default_storage(storage_dir);
    let builder = apply_postgres_tree_store(builder).await?;
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

/// Build and initialize a BreezSDK instance with a custom config override
///
/// Allows tests to tweak configuration fields (e.g., `max_deposit_claim_fee`).
/// Common test defaults (no API key, no lnurl, faster sync, prefer spark) are applied
/// on top unless explicitly set in the provided config.
pub async fn build_sdk_with_custom_config(
    storage_dir: String,
    seed_bytes: [u8; 32],
    mut config: Config,
    temp_dir: Option<tempdir::TempDir>,
    apply_sensible_test_defaults: bool,
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

    let builder = SdkBuilder::new(config, seed).with_default_storage(storage_dir);
    let builder = apply_postgres_tree_store(builder).await?;
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
    })
}

/// Modifies a PostgreSQL connection string to use a different database name.
///
/// Replaces or appends `dbname=<new_db_name>` to the connection string.
/// This is useful when multiple SDK instances need separate databases.
///
/// # Arguments
/// * `conn_str` - Original PostgreSQL connection string
/// * `suffix` - Suffix to append to the database name
///
/// # Returns
/// Modified connection string with the new database name
/// Extracts the database name from a PostgreSQL connection string.
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

/// Build and initialize a BreezSDK instance with optional PostgreSQL tree store
///
/// Similar to `build_sdk_with_custom_config` but allows specifying a PostgreSQL
/// connection string for the tree store. This is useful for benchmarks that want
/// to test SDK performance with different tree store backends.
///
/// **Important**: Each SDK instance MUST use a separate database. The tree store
/// tables don't have wallet identity separation, so sharing a database between
/// multiple SDKs will cause conflicts.
///
/// # Arguments
/// * `storage_dir` - Directory path for SDK storage
/// * `seed_bytes` - 32-byte seed for deterministic wallet generation
/// * `config` - SDK configuration to use
/// * `temp_dir` - Optional TempDir to keep alive (prevents premature deletion)
/// * `apply_sensible_test_defaults` - Whether to apply test defaults to config
/// * `postgres_tree_store_connection` - Optional PostgreSQL connection string for tree store
///
/// # Returns
/// An SdkInstance containing the SDK, event channel, and optional TempDir
pub async fn build_sdk_with_tree_store_config(
    storage_dir: String,
    seed_bytes: [u8; 32],
    mut config: Config,
    temp_dir: Option<tempdir::TempDir>,
    apply_sensible_test_defaults: bool,
    postgres_tree_store_connection: Option<String>,
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

    let mut builder = SdkBuilder::new(config, seed).with_default_storage(storage_dir);

    // Add postgres tree store if connection string provided, otherwise fall through
    // to the env-var-based shared container
    if let Some(conn_str) = postgres_tree_store_connection {
        // Ensure the database exists (create if necessary)
        ensure_postgres_database_exists(&conn_str).await?;

        // Create config with 30 connections to support high concurrency
        let mut pg_config = breez_sdk_spark::default_postgres_storage_config(conn_str);
        pg_config.max_pool_size = 30;

        builder = builder.with_postgres_tree_store(pg_config);
    } else {
        builder = apply_postgres_tree_store(builder).await?;
    }

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
    temp_dir: Option<tempdir::TempDir>,
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
    let builder = SdkBuilder::new(config, seed).with_default_storage(storage_dir);
    let builder = apply_postgres_tree_store(builder).await?;
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
    })
}

/// Build SDK instance using external signer instead of seed
///
/// # Arguments
/// * `storage_dir` - Directory path for SDK storage
/// * `mnemonic` - BIP39 mnemonic phrase for the external signer
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

    // Create default external signer from mnemonic
    let signer = breez_sdk_spark::default_external_signer(
        mnemonic,
        None, // no passphrase
        Network::Regtest,
        Some(KeySetConfig {
            key_set_type: KeySetType::Default,
            use_address_index: false,
            account_number: None,
        }),
    )?;

    // Use SdkBuilder directly so we can apply postgres tree store
    let builder = SdkBuilder::new_with_signer(config, signer).with_default_storage(storage_dir);
    let builder = apply_postgres_tree_store(builder).await?;
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
    })
}

pub async fn wait_for<F, Fut, T>(mut check_fn: F, timeout_secs: u64) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(timeout_secs);

    loop {
        match check_fn().await {
            Ok(value) => {
                debug!(
                    "Condition met after {:?}, returning result",
                    start.elapsed()
                );
                return Ok(value);
            }
            Err(e) => {
                if start.elapsed() >= timeout {
                    return Err(anyhow::anyhow!(
                        "Timeout after {} seconds waiting for condition: {}",
                        timeout_secs,
                        e
                    ));
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }
    }
}

/// Wait for SDK wallet balance to reach at least the specified amount
///
/// This helper polls the wallet balance periodically until it reaches the minimum
/// required amount or times out.
///
/// # Arguments
/// * `sdk` - The BreezSDK instance to check
/// * `min_balance` - Minimum balance in satoshis to wait for
/// * `max_balance` - Maximum balance in satoshis to wait for
/// * `timeout_secs` - Maximum time to wait in seconds before giving up
///
/// # Returns
/// The current balance once it reaches the minimum, or error if timeout
pub async fn wait_for_balance(
    sdk: &BreezSdk,
    min_balance: Option<u64>,
    max_balance: Option<u64>,
    timeout_secs: u64,
) -> Result<u64> {
    wait_for(
        || async {
            // Sync wallet to ensure we have the latest balance
            sdk.sync_wallet(SyncWalletRequest {}).await?;
            let info = sdk
                .get_info(GetInfoRequest {
                    ensure_synced: Some(false),
                })
                .await?;

            if let Some(min_balance) = min_balance
                && info.balance_sats >= min_balance
            {
                info!(
                    "Balance requirement met: {} sats (required: {} sats)",
                    info.balance_sats, min_balance
                );
                return Ok(info.balance_sats);
            }

            if let Some(max_balance) = max_balance
                && info.balance_sats >= max_balance
            {
                info!(
                    "Balance requirement met: {} sats (required: {} sats)",
                    info.balance_sats, max_balance
                );
                return Ok(info.balance_sats);
            }

            info!(
                "Waiting for balance... current: {} sats, target min: {} sats or max: {} sats",
                info.balance_sats,
                min_balance.unwrap_or_default(),
                max_balance.unwrap_or_default()
            );

            anyhow::bail!(
                "Balance not yet reached. Current: {} sats, target min: {:?} sats, max: {:?} sats",
                info.balance_sats,
                min_balance,
                max_balance
            )
        },
        timeout_secs,
    )
    .await
}

/// Wait for a token balance to increase above a previous value.
///
/// Polls the SDK until the token balance for the given identifier exceeds `previous_balance`.
/// Syncs the wallet on each poll iteration.
///
/// # Arguments
/// * `sdk` - The SDK instance to query
/// * `token_identifier` - The token identifier to check balance for
/// * `previous_balance` - The balance threshold that must be exceeded
/// * `timeout_secs` - Maximum time to wait in seconds
///
/// # Returns
/// The new token balance once it exceeds `previous_balance`, or error if timeout
pub async fn wait_for_token_balance_increase(
    sdk: &BreezSdk,
    token_identifier: &str,
    previous_balance: u128,
    timeout_secs: u64,
) -> Result<u128> {
    let token_id = token_identifier.to_string();
    wait_for(
        || {
            let sdk = sdk.clone();
            let token_id = token_id.clone();
            async move {
                sdk.sync_wallet(SyncWalletRequest {}).await?;
                let info = sdk
                    .get_info(GetInfoRequest {
                        ensure_synced: Some(false),
                    })
                    .await?;
                let token_balance = info
                    .token_balances
                    .get(&token_id)
                    .map(|b| b.balance)
                    .unwrap_or(0);
                if token_balance > previous_balance {
                    Ok(token_balance)
                } else {
                    anyhow::bail!(
                        "Token balance not yet increased: {} (was {})",
                        token_balance,
                        previous_balance
                    )
                }
            }
        },
        timeout_secs,
    )
    .await
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
            payment_method: ReceivePaymentMethod::BitcoinAddress,
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

/// Result of waiting for a specific SDK event
pub enum EventResult {
    /// Deposit claim succeeded
    ClaimSucceeded,
    /// Payment succeeded with details
    PaymentSucceeded(Box<Payment>),
    /// Payment pending with details
    PaymentPending(Box<Payment>),
    /// Payment failed with details
    PaymentFailed(Box<Payment>),
    /// Synced event occurred
    Synced,
}

pub async fn clear_event_receiver(event_rx: &mut mpsc::Receiver<SdkEvent>) {
    while let Ok(event) = event_rx.try_recv() {
        info!("Clearing event from channel: {:?}", event);
    }
}

/// Generic event waiter with timeout
///
/// # Arguments
/// * `event_rx` - Event receiver channel
/// * `timeout_secs` - Maximum time to wait in seconds
/// * `matcher` - Function that matches and extracts the desired event
///
/// # Returns
/// The matched event result or error on timeout/failure
async fn wait_for_event<F>(
    event_rx: &mut mpsc::Receiver<SdkEvent>,
    timeout_secs: u64,
    event_name: &str,
    mut matcher: F,
) -> Result<EventResult>
where
    F: FnMut(SdkEvent) -> Result<Option<EventResult>>,
{
    let timeout = tokio::time::Duration::from_secs(timeout_secs);
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            anyhow::bail!(
                "Timeout waiting for {} event after {} seconds",
                event_name,
                timeout_secs
            );
        }

        match tokio::time::timeout(remaining, event_rx.recv()).await {
            Ok(Some(event)) => {
                match matcher(event) {
                    Ok(Some(result)) => return Ok(result),
                    Ok(None) => {
                        // Not the event we're looking for, keep waiting
                        continue;
                    }
                    Err(e) => {
                        // Matcher returned an error (e.g., failure event)
                        return Err(e);
                    }
                }
            }
            Ok(None) => {
                anyhow::bail!("Event channel closed unexpectedly");
            }
            Err(_) => {
                anyhow::bail!(
                    "Timeout waiting for {} event after {} seconds",
                    event_name,
                    timeout_secs
                );
            }
        }
    }
}

/// Wait for a deposit claim to succeed by listening to SDK events
///
/// # Arguments
/// * `event_rx` - Event receiver channel from build_sdk
/// * `timeout_secs` - Maximum time to wait in seconds
///
/// # Returns
/// Ok if claim succeeded, Error if timeout or failure
pub async fn wait_for_claimed_event(
    event_rx: &mut mpsc::Receiver<SdkEvent>,
    timeout_secs: u64,
) -> Result<()> {
    wait_for_event(
        event_rx,
        timeout_secs,
        "ClaimDeposits",
        |event| match event {
            SdkEvent::ClaimedDeposits { claimed_deposits } => {
                info!(
                    "Received ClaimedDeposits event: {} deposits claimed",
                    claimed_deposits.len()
                );
                Ok(Some(EventResult::ClaimSucceeded))
            }
            SdkEvent::UnclaimedDeposits { unclaimed_deposits } => Err(anyhow::anyhow!(
                "Received UnclaimedDeposits event: {} deposits unclaimed",
                unclaimed_deposits.len()
            )),
            other => {
                info!("Ignored SDK event: {:?}", other);
                Ok(None)
            }
        },
    )
    .await
    .map(|_| ())
}

/// Wait for a payment to succeed by listening to SDK events
///
/// # Arguments
/// * `event_rx` - Event receiver channel from build_sdk
/// * `timeout_secs` - Maximum time to wait in seconds
///
/// # Returns
/// The payment details from the PaymentSucceeded event
pub async fn wait_for_payment_succeeded_event(
    event_rx: &mut mpsc::Receiver<SdkEvent>,
    payment_type: PaymentType,
    timeout_secs: u64,
) -> Result<Payment> {
    wait_for_event(
        event_rx,
        timeout_secs,
        "PaymentSucceeded",
        |event| match event {
            SdkEvent::PaymentSucceeded { payment } if payment.payment_type == payment_type => {
                info!(
                    "Received PaymentSucceeded event: {} sats, type: {:?}",
                    payment.amount, payment.payment_type
                );
                Ok(Some(EventResult::PaymentSucceeded(Box::new(payment))))
            }
            other => {
                info!("Ignored SDK event: {:?}", other);
                Ok(None)
            }
        },
    )
    .await
    .and_then(|result| match result {
        EventResult::PaymentSucceeded(payment) => Ok(*payment),
        _ => Err(anyhow::anyhow!("Unexpected event result")),
    })
}

/// Wait for a PaymentSucceeded event matching both payment type and method.
/// This is more specific than `wait_for_payment_succeeded_event` and should be
/// used when multiple payments of the same type but different methods might arrive.
pub async fn wait_for_payment_succeeded_event_with_method(
    event_rx: &mut mpsc::Receiver<SdkEvent>,
    payment_type: PaymentType,
    payment_method: PaymentMethod,
    timeout_secs: u64,
) -> Result<Payment> {
    wait_for_event(
        event_rx,
        timeout_secs,
        "PaymentSucceeded",
        |event| match event {
            SdkEvent::PaymentSucceeded { payment }
                if payment.payment_type == payment_type && payment.method == payment_method =>
            {
                info!(
                    "Received PaymentSucceeded event: {} sats, type: {:?}, method: {:?}",
                    payment.amount, payment.payment_type, payment.method
                );
                Ok(Some(EventResult::PaymentSucceeded(Box::new(payment))))
            }
            SdkEvent::PaymentSucceeded { payment } => {
                info!(
                    "Ignored PaymentSucceeded event (wrong method): {} sats, type: {:?}, method: {:?}",
                    payment.amount, payment.payment_type, payment.method
                );
                Ok(None)
            }
            other => {
                info!("Ignored SDK event: {:?}", other);
                Ok(None)
            }
        },
    )
    .await
    .and_then(|result| match result {
        EventResult::PaymentSucceeded(payment) => Ok(*payment),
        _ => Err(anyhow::anyhow!("Unexpected event result")),
    })
}

pub async fn wait_for_payment_pending_event(
    event_rx: &mut mpsc::Receiver<SdkEvent>,
    payment_type: PaymentType,
    timeout_secs: u64,
) -> Result<Payment> {
    wait_for_event(
        event_rx,
        timeout_secs,
        "PaymentPending",
        |event| match event {
            SdkEvent::PaymentPending { payment } if payment.payment_type == payment_type => {
                info!(
                    "Received PaymentPending event: {} sats, type: {:?}",
                    payment.amount, payment.payment_type
                );
                Ok(Some(EventResult::PaymentPending(Box::new(payment))))
            }
            other => {
                info!("Ignored SDK event: {:?}", other);
                Ok(None)
            }
        },
    )
    .await
    .and_then(|result| match result {
        EventResult::PaymentPending(payment) => Ok(*payment),
        _ => Err(anyhow::anyhow!("Unexpected event result")),
    })
}

pub async fn wait_for_payment_failed_event(
    event_rx: &mut mpsc::Receiver<SdkEvent>,
    payment_type: PaymentType,
    timeout_secs: u64,
) -> Result<Payment> {
    wait_for_event(
        event_rx,
        timeout_secs,
        "PaymentFailed",
        |event| match event {
            SdkEvent::PaymentFailed { payment } if payment.payment_type == payment_type => {
                info!(
                    "Received PaymentFailed event: {} sats, type: {:?}",
                    payment.amount, payment.payment_type
                );
                Ok(Some(EventResult::PaymentFailed(Box::new(payment))))
            }
            other => {
                info!("Ignored SDK event: {:?}", other);
                Ok(None)
            }
        },
    )
    .await
    .and_then(|result| match result {
        EventResult::PaymentFailed(payment) => Ok(*payment),
        _ => Err(anyhow::anyhow!("Unexpected event result")),
    })
}

/// Wait for a synced SDK events
///
/// # Arguments
/// * `event_rx` - Event receiver channel from build_sdk
/// * `timeout_secs` - Maximum time to wait in seconds
pub async fn wait_for_synced_event(
    event_rx: &mut mpsc::Receiver<SdkEvent>,
    timeout_secs: u64,
) -> Result<()> {
    wait_for_event(event_rx, timeout_secs, "Synced", |event| match event {
        SdkEvent::Synced => Ok(Some(EventResult::Synced)),
        other => {
            info!("Ignored SDK event: {:?}", other);
            Ok(None)
        }
    })
    .await
    .map(|_| ())
}

/// Wait for a set of payment events in any order.
///
/// Collects PaymentSucceeded events and marks them off from the expected list.
/// Returns Ok(()) when all expected events have been received.
/// Ignores non-matching events (e.g., Synced) and continues waiting.
///
/// Each expected event is specified as a (PaymentType, PaymentMethod) tuple.
async fn wait_for_payment_events_unordered(
    event_rx: &mut mpsc::Receiver<SdkEvent>,
    expected: Vec<(PaymentType, PaymentMethod)>,
    timeout_secs: u64,
) -> Result<()> {
    let mut remaining = expected;
    let timeout = tokio::time::Duration::from_secs(timeout_secs);
    let deadline = tokio::time::Instant::now() + timeout;

    while !remaining.is_empty() {
        let time_left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if time_left.is_zero() {
            anyhow::bail!(
                "Timeout after {} seconds. Still waiting for: {:?}",
                timeout_secs,
                remaining
            );
        }

        match tokio::time::timeout(time_left, event_rx.recv()).await {
            Ok(Some(SdkEvent::PaymentSucceeded { payment })) => {
                // Find and remove the first matching expected event
                if let Some(pos) = remaining
                    .iter()
                    .position(|(pt, pm)| *pt == payment.payment_type && *pm == payment.method)
                {
                    remaining.swap_remove(pos);
                    info!(
                        "Matched SDK event: {:?}/{:?} ({} remaining)",
                        payment.payment_type,
                        payment.method,
                        remaining.len()
                    );
                } else {
                    info!(
                        "Unmatched PaymentSucceeded event: {:?}/{:?} (still waiting for: {:?})",
                        payment.payment_type, payment.method, remaining
                    );
                }
            }
            Ok(Some(other)) => {
                info!("Ignored SDK event: {:?}", other);
                continue;
            }
            Ok(None) => anyhow::bail!("Event channel closed"),
            Err(_) => anyhow::bail!(
                "Timeout after {} seconds. Still waiting for: {:?}",
                timeout_secs,
                remaining
            ),
        }
    }
    Ok(())
}

/// Wait for and consume all auto-conversion events (BTC → Token) in any order:
/// - Receive payment (incoming BTC that triggered conversion)
/// - Send Spark (BTC to swap service)
/// - Receive Token (tokens from swap service)
///
/// # Arguments
/// * `event_rx` - Event receiver channel from build_sdk
/// * `receive_method` - The payment method of the incoming payment (Spark or Lightning)
/// * `timeout_secs` - Maximum time to wait in seconds
pub async fn wait_for_auto_conversion_events(
    event_rx: &mut mpsc::Receiver<SdkEvent>,
    receive_method: PaymentMethod,
    timeout_secs: u64,
) -> Result<()> {
    wait_for_payment_events_unordered(
        event_rx,
        vec![
            (PaymentType::Receive, receive_method),
            (PaymentType::Send, PaymentMethod::Spark),
            (PaymentType::Receive, PaymentMethod::Token),
        ],
        timeout_secs,
    )
    .await
}

/// Wait for and consume all payment conversion events (Token → BTC) in any order:
/// - Send Token (to swap service)
/// - Receive Spark (BTC from swap service)
/// - Send payment (actual outgoing payment)
///
/// # Arguments
/// * `event_rx` - Event receiver channel from build_sdk
/// * `payment_method` - The payment method of the final outgoing payment (Spark or Lightning)
/// * `timeout_secs` - Maximum time to wait in seconds
pub async fn wait_for_payment_conversion_events(
    event_rx: &mut mpsc::Receiver<SdkEvent>,
    payment_method: PaymentMethod,
    timeout_secs: u64,
) -> Result<()> {
    wait_for_payment_events_unordered(
        event_rx,
        vec![
            (PaymentType::Send, PaymentMethod::Token),
            (PaymentType::Receive, PaymentMethod::Spark),
            (PaymentType::Send, payment_method),
        ],
        timeout_secs,
    )
    .await
}

pub fn generate_preimage_hash_pair() -> (String, String) {
    let mut preimage_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut preimage_bytes);
    let preimage = hex::encode(preimage_bytes);
    let payment_hash = sha256::Hash::hash(&preimage_bytes).to_string();
    (preimage, payment_hash)
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
    config.optimization_config.auto_enabled = false;

    let seed = breez_sdk_spark::Seed::Entropy(seed_bytes.to_vec());

    let postgres_config =
        breez_sdk_spark::default_postgres_storage_config(connection_string.to_string());

    let sdk = breez_sdk_spark::SdkBuilder::new(config, seed)
        .with_postgres_storage(postgres_config.clone())
        // Use PostgresTreeStore for tree state sharing across instances
        .with_postgres_tree_store(postgres_config)
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
    })
}
