use std::sync::Arc;

use flutter_rust_bridge::frb;

/// A shareable manager for gRPC connections to the Spark operators.
///
/// Construct one via [`new_connection_manager`] and pass the same handle to
/// multiple `SdkBuilder`s via `with_connection_manager` to reuse one set of
/// HTTP/2-multiplexed connections across many SDK instances.
///
/// All SDK instances sharing a connection manager must be configured for the
/// same network and operator pool.
pub struct ConnectionManager {
    pub(crate) inner: Arc<breez_sdk_spark::ConnectionManager>,
}

#[frb(sync)]
#[must_use]
pub fn new_connection_manager() -> ConnectionManager {
    ConnectionManager {
        inner: breez_sdk_spark::new_connection_manager(),
    }
}
