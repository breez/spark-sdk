use std::sync::Arc;

use flutter_rust_bridge::frb;

/// A shared HTTP transport for SSP GraphQL traffic.
///
/// Construct one via [`new_ssp_connection_manager`] and pass the same handle
/// to multiple `SdkBuilder`s via `with_ssp_connection_manager` to reuse one
/// HTTP/2-multiplexed connection across many SDK instances.
pub struct SspConnectionManager {
    pub(crate) inner: Arc<breez_sdk_spark::SspConnectionManager>,
}

#[frb(sync)]
#[must_use]
pub fn new_ssp_connection_manager(user_agent: Option<String>) -> SspConnectionManager {
    SspConnectionManager {
        inner: breez_sdk_spark::new_ssp_connection_manager(user_agent),
    }
}
