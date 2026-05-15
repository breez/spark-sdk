use std::sync::Arc;

use wasm_bindgen::prelude::*;

/// Shared transport for SSP GraphQL traffic across SDK instances.
///
/// All SDK instances built with the same `SspConnectionManager` share a single
/// underlying HTTP client (and its h2 connection pool) for SSP requests.
#[wasm_bindgen]
pub struct SspConnectionManager {
    pub(crate) inner: Arc<breez_sdk_spark::SspConnectionManager>,
}

#[wasm_bindgen(js_name = "newSspConnectionManager")]
pub fn new_ssp_connection_manager(user_agent: Option<String>) -> SspConnectionManager {
    SspConnectionManager {
        inner: breez_sdk_spark::new_ssp_connection_manager(user_agent),
    }
}
