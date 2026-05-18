//! Shareable gRPC channel manager for the Spark operators.
//!
//! HTTP transport (SSP, Breez server, chain service, JWT fetch, LNURL) is now
//! consolidated under the single `http_client` field on
//! [`SdkContext`](crate::SdkContext); see that type for the HTTP side.

use std::sync::Arc;

use spark_wallet::{
    BalancedConnectionManager, ConnectionManager as InnerConnectionManager,
    DefaultConnectionManager,
};

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
