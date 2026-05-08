//! Shareable transports for cross-SDK-instance connection reuse.

use std::sync::Arc;

use platform_utils::{HttpClient, create_http_client};

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
