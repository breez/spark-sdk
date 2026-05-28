use std::sync::Arc;

use breez_sdk_common::breez_server::{BreezServer, PRODUCTION_BREEZSERVER_URL};
use platform_utils::{HttpClient, create_http_client};

use spark_wallet::{BalancedConnectionManager, ConnectionManager, DefaultConnectionManager};

use crate::{
    Network, SdkError, default_user_agent, jwt_header_provider::BreezJwtHeaderProvider,
    persist::backend::StorageBackend,
};

/// Process-shared resources that can back many `BreezSdk` instances.
///
/// Construct one with [`new_shared_sdk_context`] and pass the same `Arc` to every
/// [`SdkBuilder`](crate::SdkBuilder) whose SDKs should share those resources
/// (a single HTTP client across SSP / chain / LNURL / JWT / etc., a gRPC
/// channel pool to the Spark operators, the Breez backend gRPC client, …).
/// Useful for multi-tenant servers that load many wallets in one process.
///
/// To share a database connection pool across SDKs, pass a
/// [`StorageBackend`](crate::StorageBackend) as
/// [`SdkContextConfig::storage`]: every SDK built from the context reuses it.
///
/// The struct is intentionally opaque — all fields are crate-private. There
/// is no way to inject pre-built sub-components: the factory builds them
/// from settings so callers don't need to know about session stores or
/// connection-manager wiring.
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
    /// The network the context was built for. Kept so `SdkBuilder::build()`
    /// can cross-check against `Config.network` and refuse a mismatch.
    pub(crate) network: Network,
    /// The api key the context was built with. Kept so `SdkBuilder::build()`
    /// can cross-check against `Config.api_key` and refuse a mismatch.
    pub(crate) api_key: Option<String>,
    pub(crate) connection_manager: Arc<dyn ConnectionManager>,
    /// The storage backend SDKs built from this context share. `None` when the
    /// context carries no storage; each `SdkBuilder` then supplies its own.
    pub(crate) storage_backend: Option<Arc<dyn StorageBackend>>,
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

    /// Shared storage backend for SDKs built from this context. When set,
    /// every SDK built from the context reuses it (and its database
    /// connection pool). Construct via
    /// [`default_storage`](crate::default_storage),
    /// [`postgres_storage`](crate::postgres_storage),
    /// [`mysql_storage`](crate::mysql_storage) or
    /// [`custom_storage`](crate::custom_storage).
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub storage: Option<Arc<dyn StorageBackend>>,
}

impl SdkContextConfig {
    /// Config with the given network and every other field defaulted. Use
    /// directly for the bare case, or with struct update syntax to override
    /// specific fields: `SdkContextConfig { storage: Some(storage),
    /// ..SdkContextConfig::new(network) }`.
    #[must_use]
    pub fn new(network: Network) -> Self {
        Self {
            network,
            api_key: None,
            connections_per_operator: None,
            storage: None,
        }
    }
}

/// Constructs an [`SdkContext`] from a `SdkContextConfig`.
///
/// The returned `Arc` is cheap to clone and can back many SDK instances,
/// sharing their HTTP client and operator gRPC channels.
// Async-on-tokio so UniFFI runs it on the managed runtime: building the
// shared resources `tokio::spawn`s internally (gRPC channel; mainnet JWT
// task) and aborts off-runtime, despite no `.await` here.
#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
pub async fn new_shared_sdk_context(config: SdkContextConfig) -> Result<Arc<SdkContext>, SdkError> {
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
    let api_key = config.api_key;
    let jwt_header_provider = if matches!(config.network, Network::Mainnet)
        && let Some(ref key) = api_key
    {
        Some(BreezJwtHeaderProvider::new(
            key.clone(),
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

    // Every SDK built from this context shares the one storage backend (and
    // its database connection pool).
    let storage_backend = config.storage;

    Ok(Arc::new(SdkContext {
        http_client,
        breez_server,
        jwt_header_provider,
        network: config.network,
        api_key,
        connection_manager,
        storage_backend,
    }))
}

#[cfg(all(test, not(target_family = "wasm")))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn default_config_yields_context_with_shared_clients_and_no_db() {
        let ctx = new_shared_sdk_context(SdkContextConfig::new(Network::Regtest))
            .await
            .expect("default context");
        // Just confirming the Arcs are non-null.
        let _http = Arc::clone(&ctx.http_client);
        let _breez = Arc::clone(&ctx.breez_server);
        let _so = Arc::clone(&ctx.connection_manager);
        // Default config has no api_key, so no JWT provider is constructed.
        assert!(ctx.jwt_header_provider.is_none());
        // Network and api_key are stored verbatim for the builder cross-check.
        assert_eq!(ctx.network, Network::Regtest);
        assert!(ctx.api_key.is_none());
        assert!(ctx.storage_backend.is_none());
    }

    #[tokio::test]
    async fn mainnet_with_api_key_constructs_jwt_provider_and_stores_inputs() {
        let ctx = new_shared_sdk_context(SdkContextConfig {
            api_key: Some("test-key".to_string()),
            ..SdkContextConfig::new(Network::Mainnet)
        })
        .await
        .expect("mainnet context");
        assert!(ctx.jwt_header_provider.is_some());
        assert_eq!(ctx.network, Network::Mainnet);
        assert_eq!(ctx.api_key.as_deref(), Some("test-key"));
    }

    #[tokio::test]
    async fn regtest_with_api_key_skips_jwt_but_still_stores_inputs() {
        let ctx = new_shared_sdk_context(SdkContextConfig {
            api_key: Some("test-key".to_string()),
            ..SdkContextConfig::new(Network::Regtest)
        })
        .await
        .expect("regtest context");
        // Regtest never gets a JWT provider — there's no Breez endpoint to
        // mint a token. But the inputs are still stored so the builder
        // cross-check can detect a network mismatch.
        assert!(ctx.jwt_header_provider.is_none());
        assert_eq!(ctx.network, Network::Regtest);
        assert_eq!(ctx.api_key.as_deref(), Some("test-key"));
    }
}
