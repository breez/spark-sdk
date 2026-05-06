//! Adapter module bridging `spark-mysql` types with breez-sdk types.
//!
//! Provides:
//! - UniFFI-annotated wrapper type for `MysqlStorageConfig`
//! - Error conversions between `spark_mysql::MysqlError` and `StorageError`
//! - Error mapping helpers for `storage.rs`

use std::sync::Arc;

pub use spark_mysql::Migration;
use spark_mysql::mysql_async;
use spark_wallet::{TokenOutputStore, TreeStore};

use crate::persist::StorageError;

/// Configuration for `MySQL` storage connection pool.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MysqlStorageConfig {
    /// `MySQL` connection string (URL form).
    ///
    /// Supported format:
    /// - URL: `mysql://user:password@host:3306/dbname?ssl-mode=required`
    pub connection_string: String,

    /// Maximum number of connections in the pool.
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

    /// Custom CA certificate(s) in PEM format for server verification.
    /// Only used when the connection string requests TLS
    /// (`ssl-mode=verify_ca` or `ssl-mode=verify_identity`).
    pub root_ca_pem: Option<String>,
}

impl From<MysqlStorageConfig> for spark_mysql::MysqlStorageConfig {
    fn from(config: MysqlStorageConfig) -> Self {
        Self {
            connection_string: config.connection_string,
            max_pool_size: config.max_pool_size,
            wait_timeout_secs: config.wait_timeout_secs,
            create_timeout_secs: config.create_timeout_secs,
            recycle_timeout_secs: config.recycle_timeout_secs,
            root_ca_pem: config.root_ca_pem,
        }
    }
}

impl From<spark_mysql::MysqlStorageConfig> for MysqlStorageConfig {
    fn from(config: spark_mysql::MysqlStorageConfig) -> Self {
        Self {
            connection_string: config.connection_string,
            max_pool_size: config.max_pool_size,
            wait_timeout_secs: config.wait_timeout_secs,
            create_timeout_secs: config.create_timeout_secs,
            recycle_timeout_secs: config.recycle_timeout_secs,
            root_ca_pem: config.root_ca_pem,
        }
    }
}

impl MysqlStorageConfig {
    /// Creates a new configuration with the given connection string and sensible defaults.
    #[must_use]
    pub fn with_defaults(connection_string: impl Into<String>) -> Self {
        spark_mysql::MysqlStorageConfig::with_defaults(connection_string).into()
    }
}

/// Creates a `MysqlStorageConfig` with the given connection string and default pool settings.
#[cfg_attr(feature = "uniffi", uniffi::export)]
#[must_use]
pub fn default_mysql_storage_config(connection_string: String) -> MysqlStorageConfig {
    spark_mysql::default_mysql_storage_config(connection_string).into()
}

// ── Error conversions ─────────────────────────────────────────────────────────

impl From<spark_mysql::MysqlError> for StorageError {
    fn from(e: spark_mysql::MysqlError) -> Self {
        match e {
            spark_mysql::MysqlError::Connection(msg) => StorageError::Connection(msg),
            spark_mysql::MysqlError::Initialization(msg) => StorageError::InitializationError(msg),
            spark_mysql::MysqlError::Database(msg) => StorageError::Implementation(msg),
        }
    }
}

impl From<mysql_async::Error> for StorageError {
    fn from(value: mysql_async::Error) -> Self {
        let my_err: spark_mysql::MysqlError = value.into();
        my_err.into()
    }
}

/// Maps a `mysql_async` error to `StorageError`.
#[allow(clippy::needless_pass_by_value)]
pub(super) fn map_db_error(e: mysql_async::Error) -> StorageError {
    let my_err = spark_mysql::map_db_error(e);
    my_err.into()
}

// ── Pool and migration wrappers ───────────────────────────────────────────────

/// Creates a `MySQL` connection pool from the given configuration.
pub(crate) fn create_pool(config: &MysqlStorageConfig) -> Result<mysql_async::Pool, StorageError> {
    let sm_config: spark_mysql::MysqlStorageConfig = config.clone().into();
    spark_mysql::create_pool(&sm_config).map_err(StorageError::from)
}

/// Runs database migrations with version tracking and concurrency control.
pub(super) async fn run_migrations(
    pool: &mysql_async::Pool,
    migrations_table: &str,
    migrations: &[&[Migration]],
) -> Result<(), StorageError> {
    spark_mysql::run_migrations(pool, migrations_table, migrations)
        .await
        .map_err(StorageError::from)
}

// ── Store factories ───────────────────────────────────────────────────────────

/// Creates a `MysqlTreeStore` instance for use with the SDK, using an existing pool.
pub(crate) async fn create_mysql_tree_store(
    pool: mysql_async::Pool,
) -> Result<Arc<dyn TreeStore>, StorageError> {
    spark_mysql::create_mysql_tree_store_from_pool(pool)
        .await
        .map_err(StorageError::from)
}

/// Creates a `MysqlTokenStore` instance for use with the SDK, using an existing pool.
pub(crate) async fn create_mysql_token_store(
    pool: mysql_async::Pool,
) -> Result<Arc<dyn TokenOutputStore>, StorageError> {
    spark_mysql::create_mysql_token_store_from_pool(pool)
        .await
        .map_err(StorageError::from)
}
