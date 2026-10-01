//! `MySQL` storage configuration types.
//!
//! Compiled with the `uniffi` feature even when the `mysql` backend is off, so
//! the bindings expose the same API in every build. `UniFFI` derives must be on
//! the definition, so these mirror the `spark-mysql` types.

/// Controls whether `MySQL` migrations create database-enforced foreign keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum MysqlForeignKeyMode {
    /// Create foreign-key constraints in the managed schema.
    #[default]
    Enforced,
    /// Omit foreign-key constraints from the managed schema.
    Disabled,
}

#[cfg(feature = "mysql")]
impl From<MysqlForeignKeyMode> for spark_mysql::MysqlForeignKeyMode {
    fn from(mode: MysqlForeignKeyMode) -> Self {
        match mode {
            MysqlForeignKeyMode::Enforced => spark_mysql::MysqlForeignKeyMode::Enforced,
            MysqlForeignKeyMode::Disabled => spark_mysql::MysqlForeignKeyMode::Disabled,
        }
    }
}

#[cfg(feature = "mysql")]
impl From<spark_mysql::MysqlForeignKeyMode> for MysqlForeignKeyMode {
    fn from(mode: spark_mysql::MysqlForeignKeyMode) -> Self {
        match mode {
            spark_mysql::MysqlForeignKeyMode::Enforced => MysqlForeignKeyMode::Enforced,
            spark_mysql::MysqlForeignKeyMode::Disabled => MysqlForeignKeyMode::Disabled,
        }
    }
}

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

    /// Timeout in seconds before recycling an idle connection.
    /// `None` means connections are not recycled based on idle time.
    pub recycle_timeout_secs: Option<u64>,

    /// Custom CA certificate(s) in PEM format for server verification.
    /// If unset, Mozilla's root certificate store is used. Applies to every
    /// verifying `ssl-mode` (`preferred`, `required`, `verify_ca`,
    /// `verify_identity`).
    pub root_ca_pem: Option<String>,

    /// Whether the SDK should run schema migrations on startup.
    ///
    /// Set to `false` when the database schema is owned and migrated by the
    /// embedding service; the SDK will trust the existing schema and skip all
    /// migrations, including writes to the schema migrations tables. Defaults
    /// to `true`.
    pub run_migration: bool,

    /// Whether migrations should create database-enforced foreign keys.
    ///
    /// Use `Disabled` for environments that manage relationships in application
    /// code and require schema changes without foreign-key constraints.
    pub foreign_key_mode: MysqlForeignKeyMode,
}

#[cfg(feature = "mysql")]
impl From<MysqlStorageConfig> for spark_mysql::MysqlStorageConfig {
    fn from(config: MysqlStorageConfig) -> Self {
        Self {
            connection_string: config.connection_string,
            max_pool_size: config.max_pool_size,
            recycle_timeout_secs: config.recycle_timeout_secs,
            root_ca_pem: config.root_ca_pem,
            run_migration: config.run_migration,
            foreign_key_mode: config.foreign_key_mode.into(),
        }
    }
}

#[cfg(feature = "mysql")]
impl From<spark_mysql::MysqlStorageConfig> for MysqlStorageConfig {
    fn from(config: spark_mysql::MysqlStorageConfig) -> Self {
        Self {
            connection_string: config.connection_string,
            max_pool_size: config.max_pool_size,
            recycle_timeout_secs: config.recycle_timeout_secs,
            root_ca_pem: config.root_ca_pem,
            run_migration: config.run_migration,
            foreign_key_mode: config.foreign_key_mode.into(),
        }
    }
}

impl MysqlStorageConfig {
    /// Creates a new configuration with the given connection string and sensible defaults.
    #[must_use]
    #[cfg(feature = "mysql")]
    pub fn with_defaults(connection_string: impl Into<String>) -> Self {
        spark_mysql::MysqlStorageConfig::with_defaults(connection_string).into()
    }

    /// Creates a new configuration with the given connection string and sensible defaults.
    ///
    /// Mirrors `spark_mysql::MysqlStorageConfig::with_defaults` for builds
    /// without the `mysql` backend.
    #[must_use]
    #[cfg(not(feature = "mysql"))]
    pub fn with_defaults(connection_string: impl Into<String>) -> Self {
        Self {
            connection_string: connection_string.into(),
            max_pool_size: 32,
            recycle_timeout_secs: None,
            root_ca_pem: None,
            run_migration: true,
            foreign_key_mode: MysqlForeignKeyMode::default(),
        }
    }
}

/// Creates a `MysqlStorageConfig` with the given connection string and default pool settings.
#[cfg_attr(feature = "uniffi", uniffi::export)]
#[must_use]
pub fn default_mysql_storage_config(connection_string: String) -> MysqlStorageConfig {
    MysqlStorageConfig::with_defaults(connection_string)
}
