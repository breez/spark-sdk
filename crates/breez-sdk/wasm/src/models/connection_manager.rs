use std::sync::Arc;

use wasm_bindgen::prelude::*;

/// A shareable manager for gRPC connections to the Spark operators.
///
/// Construct one via [`newConnectionManager`](new_connection_manager) and pass
/// the same handle to multiple `SdkBuilder`s via `withConnectionManager` to
/// reuse one set of HTTP/2-multiplexed connections across many SDK instances.
///
/// All SDK instances sharing a connection manager must be configured for the
/// same network and operator pool.
#[wasm_bindgen]
pub struct ConnectionManager {
    pub(crate) inner: Arc<breez_sdk_spark::ConnectionManager>,
}

#[wasm_bindgen(js_name = "newConnectionManager")]
#[must_use]
pub fn new_connection_manager() -> ConnectionManager {
    ConnectionManager {
        inner: breez_sdk_spark::new_connection_manager(),
    }
}
