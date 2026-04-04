//! Configuration types for `PostgreSQL` connection pooling.

/// Queue mode for the connection pool.
///
/// Determines the order in which connections are retrieved from the pool.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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

impl From<PoolQueueMode> for deadpool::managed::QueueMode {
    fn from(mode: PoolQueueMode) -> Self {
        match mode {
            PoolQueueMode::Fifo => deadpool::managed::QueueMode::Fifo,
            PoolQueueMode::Lifo => deadpool::managed::QueueMode::Lifo,
        }
    }
}

impl From<deadpool::managed::QueueMode> for PoolQueueMode {
    fn from(mode: deadpool::managed::QueueMode) -> Self {
        match mode {
            deadpool::managed::QueueMode::Fifo => PoolQueueMode::Fifo,
            deadpool::managed::QueueMode::Lifo => PoolQueueMode::Lifo,
        }
    }
}

/// Returns the default pool configuration values from deadpool.
fn default_pool_config() -> deadpool_postgres::PoolConfig {
    deadpool_postgres::PoolConfig::default()
}

/// Configuration for `PostgreSQL` storage connection pool.
#[derive(Clone, Debug)]
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

impl PostgresStorageConfig {
    /// Creates a new configuration with the given connection string and pool defaults from deadpool.
    ///
    /// Default values:
    /// - `max_pool_size`: `num_cpus * 4`
    /// - `wait_timeout_secs`: `None` (wait indefinitely)
    /// - `create_timeout_secs`: `None` (no timeout)
    /// - `recycle_timeout_secs`: `None` (no timeout)
    /// - `queue_mode`: FIFO
    #[must_use]
    pub fn with_defaults(connection_string: impl Into<String>) -> Self {
        let defaults = default_pool_config();
        Self {
            connection_string: connection_string.into(),
            max_pool_size: u32::try_from(defaults.max_size).unwrap_or(u32::MAX),
            wait_timeout_secs: defaults.timeouts.wait.map(|d| d.as_secs()),
            create_timeout_secs: defaults.timeouts.create.map(|d| d.as_secs()),
            recycle_timeout_secs: defaults.timeouts.recycle.map(|d| d.as_secs()),
            queue_mode: defaults.queue_mode.into(),
            root_ca_pem: None,
        }
    }
}

/// Creates a `PostgresStorageConfig` with the given connection string and default pool settings.
///
/// This is a convenience function for creating a config with sensible defaults from deadpool.
/// Use this instead of manually constructing `PostgresStorageConfig` when you want defaults.
///
/// Default values:
/// - `max_pool_size`: `num_cpus * 4`
/// - `wait_timeout_secs`: `None` (wait indefinitely)
/// - `create_timeout_secs`: `None` (no timeout)
/// - `recycle_timeout_secs`: `None` (no timeout)
/// - `queue_mode`: FIFO
/// - `root_ca_pem`: `None` (uses Mozilla's root certificate store)
#[must_use]
pub fn default_postgres_storage_config(connection_string: String) -> PostgresStorageConfig {
    PostgresStorageConfig::with_defaults(connection_string)
}
