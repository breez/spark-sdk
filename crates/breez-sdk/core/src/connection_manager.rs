//! Shareable transports for cross-SDK-instance connection reuse.

use std::sync::Arc;

use platform_utils::{HttpClient, create_http_client};
use spark_wallet::{
    BalancedConnectionManager, ConnectionManager as InnerConnectionManager,
    DefaultConnectionManager,
};

use crate::default_user_agent;

/// A shared HTTP transport for SSP GraphQL traffic.
///
/// All SDK instances that are built with the same `SspConnectionManager` send
/// SSP requests over the same pooled `reqwest::Client`. This means each
/// process opens at most one TCP+TLS+HTTP/2 connection to the SSP regardless
/// of how many wallets are loaded — useful for multi-tenant servers running
/// many SDK instances.
///
/// # Caveats
///
/// - The user-agent of the first SDK to construct this manager is reused for
///   all subsequent instances. This is rarely a problem since SDK instances
///   in one process typically share a build version.
/// - Connections close when the last `Arc<SspConnectionManager>` is dropped.
///   `BreezSdk::disconnect` does not close them.
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct SspConnectionManager {
    pub(crate) client: Arc<dyn HttpClient>,
}

/// Construct a new shared SSP connection manager.
///
/// Typically built inside an [`SdkContext`](crate::SdkContext) by
/// [`new_sdk_context`](crate::new_sdk_context); SDKs that share the same
/// `SdkContext` automatically share the underlying HTTP connection pool.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn new_ssp_connection_manager(user_agent: Option<String>) -> Arc<SspConnectionManager> {
    let user_agent = user_agent.unwrap_or_else(default_user_agent);
    Arc::new(SspConnectionManager {
        client: create_http_client(Some(&user_agent)),
    })
}

/// A shareable manager for gRPC connections to the Spark operators.
///
/// Typically built inside an [`SdkContext`](crate::SdkContext); SDKs that
/// share the same `SdkContext` automatically share the underlying gRPC
/// channels to the Spark operators. Channels close when the last
/// `Arc<ConnectionManager>` (via its containing `SdkContext`) is dropped;
/// [`BreezSdk::disconnect`](crate::BreezSdk::disconnect) does not affect them.
///
/// All SDK instances sharing a `ConnectionManager` must be configured for the
/// same network and operator pool. The TLS settings and user agent of the
/// first SDK to connect to a given operator are reused for everyone afterwards.
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct ConnectionManager {
    pub(crate) inner: Arc<dyn InnerConnectionManager>,
}

/// Creates a new shareable [`ConnectionManager`].
///
/// `connections_per_operator` controls per-operator connection pooling:
/// `None` keeps a single connection per operator (suitable for almost every
/// deployment); `Some(n)` opens `n` connections per operator and balances
/// requests across them.
#[cfg_attr(feature = "uniffi", uniffi::export)]
#[must_use]
pub fn new_connection_manager(connections_per_operator: Option<u32>) -> Arc<ConnectionManager> {
    let inner: Arc<dyn InnerConnectionManager> = match connections_per_operator {
        Some(n) if n > 1 => Arc::new(BalancedConnectionManager::new(n)),
        _ => Arc::new(DefaultConnectionManager::new()),
    };
    Arc::new(ConnectionManager { inner })
}
