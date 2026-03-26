//! Adapter module bridging `spark-postgres` types with breez-sdk types.
//!
//! This module provides:
//! - UniFFI-annotated wrapper types for `PostgresStorageConfig` and `PoolQueueMode`
//! - Error conversions between `spark_postgres::PostgresError` and `StorageError`
//! - Error mapping helpers for `storage.rs`

use std::sync::Arc;

use spark_wallet::TreeStore;

use crate::persist::StorageError;

// ── UniFFI wrapper types ──────────────────────────────────────────────────────
// UniFFI derives must be on the struct definition, so we define local types that
// mirror the spark-postgres types and add the UniFFI attributes.

/// Queue mode for the connection pool.
///
/// Determines the order in which connections are retrieved from the pool.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum PoolQueueMode {
    /// First In, First Out (default).
    /// Connections are used in the order they were returned to the pool.
    /// Spreads load evenly across all connections.
    #[default]
    Fifo,
    /// Last In, First Out.
    /// Most recently returned connections are used first.
    /// Keeps fewer connections "hot" and allows idle connections to close sooner.
    Lifo,
}

impl From<PoolQueueMode> for spark_postgres::PoolQueueMode {
    fn from(mode: PoolQueueMode) -> Self {
        match mode {
            PoolQueueMode::Fifo => spark_postgres::PoolQueueMode::Fifo,
            PoolQueueMode::Lifo => spark_postgres::PoolQueueMode::Lifo,
        }
    }
}

impl From<spark_postgres::PoolQueueMode> for PoolQueueMode {
    fn from(mode: spark_postgres::PoolQueueMode) -> Self {
        match mode {
            spark_postgres::PoolQueueMode::Fifo => PoolQueueMode::Fifo,
            spark_postgres::PoolQueueMode::Lifo => PoolQueueMode::Lifo,
        }
    }
}

/// Configuration for `PostgreSQL` storage connection pool.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PostgresStorageConfig {
    /// `PostgreSQL` connection string (key-value or URI format).
    ///
    /// Supported formats:
    /// - Key-value: `host=localhost user=postgres dbname=spark sslmode=require`
    /// - URI: `postgres://user:password@host:port/dbname?sslmode=require`
    pub connection_string: String,

    /// Maximum number of connections in the pool.
    /// Default: `num_cpus * 4` (from deadpool).
    pub max_pool_size: u32,

    /// Timeout in seconds waiting for a connection from the pool.
    /// `None` means wait indefinitely.
    pub wait_timeout_secs: Option<u64>,

    /// Timeout in seconds for establishing a new connection.
    /// `None` means no timeout.
    pub create_timeout_secs: Option<u64>,

    /// Timeout in seconds before recycling an idle connection.
    /// `None` means connections are not recycled based on idle time.
    pub recycle_timeout_secs: Option<u64>,

    /// Queue mode for retrieving connections from the pool.
    /// Default: FIFO.
    pub queue_mode: PoolQueueMode,

    /// Custom CA certificate(s) in PEM format for server verification.
    /// If `None`, uses Mozilla's root certificate store (via webpki-roots).
    /// Only used with `sslmode=verify-ca` or `sslmode=verify-full`.
    pub root_ca_pem: Option<String>,
}

impl From<PostgresStorageConfig> for spark_postgres::PostgresStorageConfig {
    fn from(config: PostgresStorageConfig) -> Self {
        Self {
            connection_string: config.connection_string,
            max_pool_size: config.max_pool_size,
            wait_timeout_secs: config.wait_timeout_secs,
            create_timeout_secs: config.create_timeout_secs,
            recycle_timeout_secs: config.recycle_timeout_secs,
            queue_mode: config.queue_mode.into(),
            root_ca_pem: config.root_ca_pem,
        }
    }
}

impl From<spark_postgres::PostgresStorageConfig> for PostgresStorageConfig {
    fn from(config: spark_postgres::PostgresStorageConfig) -> Self {
        Self {
            connection_string: config.connection_string,
            max_pool_size: config.max_pool_size,
            wait_timeout_secs: config.wait_timeout_secs,
            create_timeout_secs: config.create_timeout_secs,
            recycle_timeout_secs: config.recycle_timeout_secs,
            queue_mode: config.queue_mode.into(),
            root_ca_pem: config.root_ca_pem,
        }
    }
}

impl PostgresStorageConfig {
    /// Creates a new configuration with the given connection string and pool defaults from deadpool.
    #[must_use]
    pub fn with_defaults(connection_string: impl Into<String>) -> Self {
        spark_postgres::PostgresStorageConfig::with_defaults(connection_string).into()
    }
}

/// Creates a `PostgresStorageConfig` with the given connection string and default pool settings.
#[cfg_attr(feature = "uniffi", uniffi::export)]
#[must_use]
pub fn default_postgres_storage_config(connection_string: String) -> PostgresStorageConfig {
    spark_postgres::default_postgres_storage_config(connection_string).into()
}

// ── Error conversions ─────────────────────────────────────────────────────────

impl From<spark_postgres::PostgresError> for StorageError {
    fn from(e: spark_postgres::PostgresError) -> Self {
        match e {
            spark_postgres::PostgresError::Connection(msg) => StorageError::Connection(msg),
            spark_postgres::PostgresError::Initialization(msg) => {
                StorageError::InitializationError(msg)
            }
            spark_postgres::PostgresError::Database(msg) => StorageError::Implementation(msg),
        }
    }
}

impl From<tokio_postgres::Error> for StorageError {
    fn from(value: tokio_postgres::Error) -> Self {
        let pg_err: spark_postgres::PostgresError = value.into();
        pg_err.into()
    }
}

/// Maps a deadpool-postgres pool error to `StorageError`.
#[allow(clippy::needless_pass_by_value)]
pub(super) fn map_pool_error(e: deadpool_postgres::PoolError) -> StorageError {
    let pg_err = spark_postgres::map_pool_error(e);
    pg_err.into()
}

/// Maps a tokio-postgres database error to `StorageError`.
#[allow(clippy::needless_pass_by_value)]
pub(super) fn map_db_error(e: tokio_postgres::Error) -> StorageError {
    let pg_err = spark_postgres::map_db_error(e);
    pg_err.into()
}

// ── Pool and migration wrappers ───────────────────────────────────────────────

/// Creates a `PostgreSQL` connection pool from the given configuration.
pub(crate) fn create_pool(
    config: &PostgresStorageConfig,
) -> Result<deadpool_postgres::Pool, StorageError> {
    let sp_config: spark_postgres::PostgresStorageConfig = config.clone().into();
    spark_postgres::create_pool(&sp_config).map_err(StorageError::from)
}

/// Runs database migrations with version tracking and concurrency control.
pub(super) async fn run_migrations(
    pool: &deadpool_postgres::Pool,
    migrations_table: &str,
    migrations: &[&[&str]],
) -> Result<(), StorageError> {
    spark_postgres::run_migrations(pool, migrations_table, migrations)
        .await
        .map_err(StorageError::from)
}

// ── Tree store factory ────────────────────────────────────────────────────────

/// Creates a `PostgresTreeStore` instance for use with the SDK, using an existing pool.
pub(crate) async fn create_postgres_tree_store(
    pool: deadpool_postgres::Pool,
) -> Result<Arc<dyn TreeStore>, StorageError> {
    spark_postgres::create_postgres_tree_store_from_pool(pool)
        .await
        .map_err(StorageError::from)
}
