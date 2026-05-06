//! Shareable transports for cross-SDK-instance connection reuse.

use std::sync::Arc;

use platform_utils::{HttpClient, create_http_client};
use spark_wallet::{ConnectionManager as InnerConnectionManager, DefaultConnectionManager};

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
/// Pass the returned `Arc<SspConnectionManager>` to
/// [`SdkBuilder::with_ssp_connection_manager`](crate::SdkBuilder::with_ssp_connection_manager)
/// when building each SDK instance that should share the underlying HTTP
/// connection pool.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn new_ssp_connection_manager(user_agent: Option<String>) -> Arc<SspConnectionManager> {
    let user_agent = user_agent.unwrap_or_else(default_user_agent);
    Arc::new(SspConnectionManager {
        client: create_http_client(Some(&user_agent)),
    })
}

/// A shareable manager for gRPC connections to the Spark operators.
///
/// Construct one via [`new_connection_manager`] and pass the same `Arc` to
/// multiple [`SdkBuilder`](crate::SdkBuilder)s via
/// [`SdkBuilder::with_connection_manager`](crate::SdkBuilder::with_connection_manager).
/// Every SDK that shares a `ConnectionManager` reuses the same HTTP/2-multiplexed
/// channels to the operators, which is meaningful for server-side deployments
/// hosting many wallets in one process.
///
/// All SDK instances sharing a `ConnectionManager` must be configured for the
/// same network and operator pool: the cache is keyed only by operator address,
/// so the TLS settings and user agent of the first SDK to connect are reused
/// for everyone afterwards.
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct ConnectionManager {
    pub(crate) inner: Arc<dyn InnerConnectionManager>,
}

/// Creates a new shareable [`ConnectionManager`].
///
/// Connections close when the last `Arc<ConnectionManager>` is dropped;
/// [`BreezSdk::disconnect`](crate::BreezSdk::disconnect) does not affect it.
#[cfg_attr(feature = "uniffi", uniffi::export)]
#[must_use]
pub fn new_connection_manager() -> Arc<ConnectionManager> {
    Arc::new(ConnectionManager {
        inner: Arc::new(DefaultConnectionManager::new()),
    })
}
