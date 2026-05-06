use std::sync::Arc;

use spark_wallet::{ConnectionManager as InnerConnectionManager, DefaultConnectionManager};

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
