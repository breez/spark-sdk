//! `PostgreSQL` storage configuration types.
//!
//! Compiled with the `db-storage-api` feature even when the `postgres` backend
//! is off, so the bindings can expose the same API in every build. `UniFFI`
//! derives must be on the definition, so these mirror the `spark-postgres`
//! types.

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

#[cfg(feature = "postgres")]
impl From<PoolQueueMode> for spark_postgres::PoolQueueMode {
    fn from(mode: PoolQueueMode) -> Self {
        match mode {
            PoolQueueMode::Fifo => spark_postgres::PoolQueueMode::Fifo,
            PoolQueueMode::Lifo => spark_postgres::PoolQueueMode::Lifo,
        }
    }
}

#[cfg(feature = "postgres")]
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
    /// If unset, Mozilla's root certificate store is used. Applies to every
    /// verifying `sslmode` (`prefer`, `require`, `verify-ca`, `verify-full`).
    pub root_ca_pem: Option<String>,

    /// Whether the SDK should run schema migrations on startup.
    ///
    /// Set to `false` when the database schema is owned and migrated by the
    /// embedding service; the SDK will trust the existing schema and skip all
    /// migrations, including writes to the schema migrations tables. Defaults
    /// to `true`.
    pub run_migration: bool,
}

#[cfg(feature = "postgres")]
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
            run_migration: config.run_migration,
        }
    }
}

#[cfg(feature = "postgres")]
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
            run_migration: config.run_migration,
        }
    }
}

impl PostgresStorageConfig {
    /// Creates a new configuration with the given connection string and pool defaults from deadpool.
    #[must_use]
    #[cfg(feature = "postgres")]
    pub fn with_defaults(connection_string: impl Into<String>) -> Self {
        spark_postgres::PostgresStorageConfig::with_defaults(connection_string).into()
    }

    /// Creates a new configuration with the given connection string and pool defaults.
    ///
    /// Without the `postgres` backend the config cannot open a pool, so these
    /// values only approximate deadpool's defaults.
    #[must_use]
    #[cfg(not(feature = "postgres"))]
    pub fn with_defaults(connection_string: impl Into<String>) -> Self {
        let cpus = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
        Self {
            connection_string: connection_string.into(),
            max_pool_size: u32::try_from(cpus.saturating_mul(4)).unwrap_or(u32::MAX),
            wait_timeout_secs: None,
            create_timeout_secs: None,
            recycle_timeout_secs: None,
            queue_mode: PoolQueueMode::default(),
            root_ca_pem: None,
            run_migration: true,
        }
    }
}

/// Creates a `PostgresStorageConfig` with the given connection string and default pool settings.
#[cfg_attr(feature = "uniffi", uniffi::export)]
#[must_use]
pub fn default_postgres_storage_config(connection_string: String) -> PostgresStorageConfig {
    PostgresStorageConfig::with_defaults(connection_string)
}
