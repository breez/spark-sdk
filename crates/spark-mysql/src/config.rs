//! Configuration types for `MySQL` connection pooling.

/// Default maximum pool size used when callers don't provide one.
/// Mirrors the deadpool default of `num_cpus * 4` reasonably without depending on `num_cpus`.
const DEFAULT_MAX_POOL_SIZE: u32 = 32;

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

    /// Timeout in seconds before recycling an idle connection.
    /// `None` means connections are not recycled based on idle time.
    pub recycle_timeout_secs: Option<u64>,

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
            recycle_timeout_secs: None,
            root_ca_pem: None,
        }
    }
}

/// Creates a `MysqlStorageConfig` with the given connection string and default pool settings.
#[must_use]
pub fn default_mysql_storage_config(connection_string: String) -> MysqlStorageConfig {
    MysqlStorageConfig::with_defaults(connection_string)
}
