use std::sync::Arc;

use breez_sdk_common::breez_server::{BreezServer, PRODUCTION_BREEZSERVER_URL};
use platform_utils::{HttpClient, create_http_client};

use spark_wallet::{BalancedConnectionManager, ConnectionManager, DefaultConnectionManager};

use crate::{Network, SdkError, default_user_agent, jwt_header_provider::BreezJwtHeaderProvider};

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
/// Construct one with [`new_shared_sdk_context`] and pass the same `Arc` to every
/// [`SdkBuilder`](crate::SdkBuilder) whose SDKs should share those resources
/// (a single HTTP client across SSP / chain / LNURL / JWT / etc., a gRPC
/// channel pool to the Spark operators, the Breez backend gRPC client, a
/// database connection pool, …). Useful for multi-tenant servers that load
/// many wallets in one process.
///
/// The struct is intentionally opaque — all fields are crate-private. There
/// is no way to inject pre-built sub-components: the factory builds them
/// from settings so callers don't need to know about session managers,
/// connection-manager wiring, or pool plumbing.
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct SdkContext {
    /// Single shared HTTP client used for every reqwest-based call out of the
    /// SDK: SSP GraphQL, chain service, LNURL, JWT fetch, etc.
    pub(crate) http_client: Arc<dyn HttpClient>,
    /// Single shared gRPC client to the Breez backend (fiat, `MoonPay`, payment
    /// notifier, signer, support, swapper).
    pub(crate) breez_server: Arc<BreezServer>,
    /// Shared Breez partner JWT header provider. Only set when
    /// `network == Mainnet && api_key.is_some()` at context construction.
    /// All SDKs sharing the context reuse one in-memory JWT and one
    /// background refresh task.
    pub(crate) jwt_header_provider: Option<Arc<BreezJwtHeaderProvider>>,
    pub(crate) connection_manager: Arc<dyn ConnectionManager>,
    #[cfg(feature = "postgres")]
    pub(crate) postgres_pool: Option<Arc<PostgresConnectionPool>>,
    #[cfg(feature = "mysql")]
    pub(crate) mysql_pool: Option<Arc<MysqlConnectionPool>>,
}

/// Settings for [`new_shared_sdk_context`]. All fields are optional; the defaults
/// match the single-SDK happy path.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SdkContextConfig {
    /// Network the shared resources target. Defaults to [`Network::Mainnet`].
    /// Used to gate the partner JWT header provider — only constructed on
    /// Mainnet, since Regtest has no JWT-issuing Breez endpoint.
    pub network: Network,

    /// Breez API key. When set together with `network == Mainnet`, the
    /// context constructs a shared partner JWT header provider that all
    /// SDKs built from this context will attach to their SO requests.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub api_key: Option<String>,

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

impl SdkContextConfig {
    /// Config with the given network and every other field defaulted. Use
    /// directly for the bare case, or with struct update syntax to override
    /// specific fields: `SdkContextConfig { postgres_config: Some(cfg),
    /// ..SdkContextConfig::new(network) }`.
    #[must_use]
    pub fn new(network: Network) -> Self {
        Self {
            network,
            api_key: None,
            connections_per_operator: None,
            #[cfg(feature = "postgres")]
            postgres_config: None,
            #[cfg(feature = "mysql")]
            mysql_config: None,
        }
    }
}

/// Constructs an [`SdkContext`] from a `SdkContextConfig`.
///
/// The returned `Arc` is cheap to clone and can back many SDK instances.
/// `SdkContextConfig::new(network)` yields an in-memory, single-tenant setup;
/// supply a DB config to back the SDKs with a shared `PostgreSQL` or `MySQL`
/// pool.
#[cfg_attr(feature = "uniffi", uniffi::export)]
#[allow(clippy::needless_pass_by_value)]
pub fn new_shared_sdk_context(config: SdkContextConfig) -> Result<Arc<SdkContext>, SdkError> {
    let user_agent = default_user_agent();
    let http_client = create_http_client(Some(&user_agent));
    let breez_server = Arc::new(
        BreezServer::new(PRODUCTION_BREEZSERVER_URL, None, &user_agent)
            .map_err(|e| SdkError::Generic(e.to_string()))?,
    );
    // The Breez partner JWT is only issued by the mainnet Breez endpoint, and
    // only when an API key is configured. Skip the provider entirely otherwise
    // — there is no token to fetch. SDKs sharing this context will share the
    // one in-memory JWT and one background refresh task.
    let jwt_header_provider = if matches!(config.network, Network::Mainnet)
        && let Some(api_key) = config.api_key
    {
        Some(BreezJwtHeaderProvider::new(
            api_key,
            None,
            http_client.clone(),
        ))
    } else {
        None
    };
    // SDKs that share the same context share the same gRPC channels to the
    // Spark operators. `connections_per_operator` lets the rare deployment
    // open multiple connections per operator and balance requests across
    // them; `None` (or `Some(1)`) keeps a single multiplexed connection.
    let connection_manager: Arc<dyn ConnectionManager> = match config.connections_per_operator {
        Some(n) if n > 1 => Arc::new(BalancedConnectionManager::new(n)),
        _ => Arc::new(DefaultConnectionManager::new()),
    };

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
        http_client,
        breez_server,
        jwt_header_provider,
        connection_manager,
        #[cfg(feature = "postgres")]
        postgres_pool,
        #[cfg(feature = "mysql")]
        mysql_pool,
    }))
}

#[cfg(all(test, not(target_family = "wasm")))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn default_config_yields_context_with_shared_clients_and_no_db() {
        let ctx = new_shared_sdk_context(SdkContextConfig::new(Network::Regtest))
            .expect("default context");
        // Just confirming the Arcs are non-null.
        let _http = Arc::clone(&ctx.http_client);
        let _breez = Arc::clone(&ctx.breez_server);
        let _so = Arc::clone(&ctx.connection_manager);
        // Default config has no api_key, so no JWT provider is constructed.
        assert!(ctx.jwt_header_provider.is_none());
        #[cfg(feature = "postgres")]
        assert!(ctx.postgres_pool.is_none());
        #[cfg(feature = "mysql")]
        assert!(ctx.mysql_pool.is_none());
    }
}
