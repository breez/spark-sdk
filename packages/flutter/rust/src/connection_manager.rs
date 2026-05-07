use std::sync::Arc;

use flutter_rust_bridge::frb;

/// A shareable manager for gRPC connections to the Spark operators.
///
/// Construct one via [`new_connection_manager`] and pass the same handle to
/// multiple `SdkBuilder`s via `with_connection_manager` to reuse connections
/// across SDK instances.
///
/// All SDK instances sharing a connection manager must be configured for the
/// same network and operator pool.
pub struct ConnectionManager {
    pub(crate) inner: Arc<breez_sdk_spark::ConnectionManager>,
}

/// Creates a connection manager.
///
/// `max_tenants_per_connection` controls per-operator connection pooling:
/// `None` keeps a single connection per operator; `Some(n)` opens an
/// additional connection for every `n` SDK instances sharing this manager.
#[frb(sync)]
#[must_use]
pub fn new_connection_manager(max_tenants_per_connection: Option<u32>) -> ConnectionManager {
    ConnectionManager {
        inner: breez_sdk_spark::new_connection_manager(max_tenants_per_connection),
    }
}
