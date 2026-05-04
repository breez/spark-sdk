//! Configuration types for `MySQL` connection pooling.

/// Default maximum pool size used when callers don't provide one.
/// Mirrors the deadpool default of `num_cpus * 4` reasonably without depending on `num_cpus`.
const DEFAULT_MAX_POOL_SIZE: u32 = 32;

/// Queue mode for the connection pool.
///
/// Determines the order in which connections are retrieved from the pool.
/// Currently informational for the `MySQL` backend (`mysql_async`'s pool does not
/// expose the same FIFO/LIFO toggle as deadpool, so this is recorded but not
/// applied). Kept for API parity with the `PostgreSQL` backend.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PoolQueueMode {
    /// First In, First Out (default).
    #[default]
    Fifo,
    /// Last In, First Out.
    Lifo,
}

/// Configuration for `MySQL` storage connection pool.
#[derive(Clone, Debug)]
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

    /// Queue mode for retrieving connections from the pool.
    /// Kept for API parity with the `PostgreSQL` backend; `mysql_async` does not
    /// expose this knob today.
    pub queue_mode: PoolQueueMode,

    /// Custom CA certificate(s) in PEM format for server verification.
    /// Only used when the connection string requests TLS (`ssl-mode=verify_ca`,
    /// `ssl-mode=verify_identity`).
    pub root_ca_pem: Option<String>,
}

impl MysqlStorageConfig {
    /// Creates a new configuration with the given connection string and sensible defaults.
    #[must_use]
    pub fn with_defaults(connection_string: impl Into<String>) -> Self {
        Self {
            connection_string: connection_string.into(),
            max_pool_size: DEFAULT_MAX_POOL_SIZE,
            wait_timeout_secs: None,
            create_timeout_secs: None,
            recycle_timeout_secs: None,
            queue_mode: PoolQueueMode::default(),
            root_ca_pem: None,
        }
    }
}

/// Creates a `MysqlStorageConfig` with the given connection string and default pool settings.
#[must_use]
pub fn default_mysql_storage_config(connection_string: String) -> MysqlStorageConfig {
    MysqlStorageConfig::with_defaults(connection_string)
}
