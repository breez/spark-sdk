use std::sync::Arc;

use crate::{
    SdkError, SspConnectionManager,
    connection_manager::{ConnectionManager, new_connection_manager, new_ssp_connection_manager},
};

#[cfg(feature = "mysql")]
use crate::persist::mysql::{
    MysqlConnectionPool, MysqlStorageConfig, create_mysql_connection_pool,
};
#[cfg(feature = "postgres")]
use crate::persist::postgres::{
    PostgresConnectionPool, PostgresStorageConfig, create_postgres_connection_pool,
};

/// Process-shared resources that can back many `BreezSdk` instances.
///
/// Construct one with [`new_sdk_context`] and pass the same `Arc` to every
/// [`SdkBuilder`](crate::SdkBuilder) whose SDKs should share those resources
/// (gRPC channels to the Spark operators, the SSP HTTP client, a database
/// connection pool, …). Useful for multi-tenant servers that load many
/// wallets in one process.
///
/// The struct is intentionally opaque — all fields are crate-private. There
/// is no way to inject pre-built sub-components: the factory builds them
/// from settings so callers don't need to know about session managers,
/// connection-manager wiring, or pool plumbing.
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct SdkContext {
    pub(crate) ssp_connection_manager: Arc<SspConnectionManager>,
    pub(crate) so_connection_manager: Arc<ConnectionManager>,
    #[cfg(feature = "postgres")]
    pub(crate) postgres_pool: Option<Arc<PostgresConnectionPool>>,
    #[cfg(feature = "mysql")]
    pub(crate) mysql_pool: Option<Arc<MysqlConnectionPool>>,
}

/// Settings for [`new_sdk_context`]. All fields are optional; the defaults
/// match the single-SDK happy path.
#[derive(Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SdkContextConfig {
    /// Number of gRPC connections per Spark operator. `None` (or `Some(1)`)
    /// keeps a single connection per operator (the right choice for most
    /// deployments); `Some(n)` opens `n` channels per operator and balances
    /// requests across them.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub connections_per_operator: Option<u32>,

    /// `PostgreSQL` backend configuration. When set, the context builds a
    /// shared connection pool and SDKs constructed with this context store
    /// their data in `PostgreSQL`.
    #[cfg(feature = "postgres")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub postgres_config: Option<PostgresStorageConfig>,

    /// `MySQL` backend configuration. When set, the context builds a shared
    /// connection pool and SDKs constructed with this context store their
    /// data in `MySQL`.
    #[cfg(feature = "mysql")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub mysql_config: Option<MysqlStorageConfig>,
}

/// Constructs an [`SdkContext`] from a `SdkContextConfig`.
///
/// The returned `Arc` is cheap to clone and can back many SDK instances. The
/// default config (`SdkContextConfig::default()`) yields an in-memory,
/// single-tenant setup; supply a DB config to back the SDKs with a shared
/// `PostgreSQL` or `MySQL` pool.
#[cfg_attr(feature = "uniffi", uniffi::export)]
#[allow(clippy::needless_pass_by_value)]
pub fn new_sdk_context(config: SdkContextConfig) -> Result<Arc<SdkContext>, SdkError> {
    let ssp_connection_manager = new_ssp_connection_manager(None);
    let so_connection_manager = new_connection_manager(config.connections_per_operator);

    #[cfg(feature = "postgres")]
    let postgres_pool = match config.postgres_config {
        Some(cfg) => Some(create_postgres_connection_pool(&cfg)?),
        None => None,
    };

    #[cfg(feature = "mysql")]
    let mysql_pool = match config.mysql_config {
        Some(cfg) => Some(create_mysql_connection_pool(&cfg)?),
        None => None,
    };

    Ok(Arc::new(SdkContext {
        ssp_connection_manager,
        so_connection_manager,
        #[cfg(feature = "postgres")]
        postgres_pool,
        #[cfg(feature = "mysql")]
        mysql_pool,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_yields_context_with_default_managers_and_no_db() {
        let ctx = new_sdk_context(SdkContextConfig::default()).expect("default context");
        // Connection managers are always present; we don't reach into their
        // internals here — just confirming the Arcs are non-null is enough.
        let _ssp = Arc::clone(&ctx.ssp_connection_manager);
        let _so = Arc::clone(&ctx.so_connection_manager);
        #[cfg(feature = "postgres")]
        assert!(ctx.postgres_pool.is_none());
        #[cfg(feature = "mysql")]
        assert!(ctx.mysql_pool.is_none());
    }
}
