use std::sync::Arc;

use breez_sdk_spark::{ChainApiType, Credentials, Network};
use flutter_rust_bridge::frb;

/// Rust-built handle to a [`breez_sdk_spark::BitcoinChainService`].
///
/// Construct via [`new_rest_chain_service`] and pass the same handle to
/// multiple `SdkBuilder`s via `with_chain_service` to share one underlying
/// HTTP client across SDK instances.
pub struct BitcoinChainServiceHandle {
    pub(crate) inner: Arc<dyn breez_sdk_spark::BitcoinChainService>,
}

/// Constructs a shareable REST-based Bitcoin chain service.
///
/// Pass the returned handle to multiple `SdkBuilder`s via `with_chain_service`
/// to reuse one HTTP client across SDK instances. All SDKs sharing the handle
/// must use the same `network`.
///
/// For one-off, non-shared use, prefer `with_rest_chain_service`.
#[frb(sync)]
#[must_use]
pub fn new_rest_chain_service(
    url: String,
    network: Network,
    api_type: ChainApiType,
    credentials: Option<Credentials>,
) -> BitcoinChainServiceHandle {
    BitcoinChainServiceHandle {
        inner: breez_sdk_spark::new_rest_chain_service(url, network, api_type, credentials),
    }
}
