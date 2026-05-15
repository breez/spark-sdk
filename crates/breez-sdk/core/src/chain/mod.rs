use std::sync::Arc;

use platform_utils::{DefaultHttpClient, HttpClient};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    Credentials, Network,
    chain::rest_client::{BasicAuth, ChainApiType, RestClientChainService},
};

pub mod rest_client;

#[derive(Debug, Error, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Error))]
pub enum ChainServiceError {
    #[error("Invalid address: {0}")]
    InvalidAddress(String),
    #[error("Service connectivity: {0}")]
    ServiceConnectivity(String),
    #[error("Generic: {0}")]
    Generic(String),
}

impl From<platform_utils::HttpError> for ChainServiceError {
    fn from(value: platform_utils::HttpError) -> Self {
        ChainServiceError::ServiceConnectivity(value.to_string())
    }
}

impl From<bitcoin::address::ParseError> for ChainServiceError {
    fn from(value: bitcoin::address::ParseError) -> Self {
        ChainServiceError::InvalidAddress(value.to_string())
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
#[macros::async_trait]
pub trait BitcoinChainService: Send + Sync {
    async fn get_address_utxos(&self, address: String) -> Result<Vec<Utxo>, ChainServiceError>;
    async fn get_transaction_status(&self, txid: String) -> Result<TxStatus, ChainServiceError>;
    async fn get_transaction_hex(&self, txid: String) -> Result<String, ChainServiceError>;
    async fn broadcast_transaction(&self, tx: String) -> Result<(), ChainServiceError>;
    async fn recommended_fees(&self) -> Result<RecommendedFees, ChainServiceError>;
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TxStatus {
    pub confirmed: bool,
    pub block_height: Option<u32>,
    pub block_time: Option<u64>,
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Utxo {
    pub txid: String,
    pub vout: u32,
    pub value: u64,
    pub status: TxStatus,
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RecommendedFees {
    pub fastest_fee: u64,
    pub half_hour_fee: u64,
    pub hour_fee: u64,
    pub economy_fee: u64,
    pub minimum_fee: u64,
}

/// Opaque handle to a Rust-built [`BitcoinChainService`].
///
/// This exists so the chain service can cross the `UniFFI` boundary without
/// having every method call routed through `UniFFI`'s foreign-trait wrapper.
/// `BitcoinChainService` carries `#[uniffi::export(with_foreign)]` so
/// foreign-language integrations can supply their own implementations —
/// but as a side-effect, an
/// `Arc<dyn BitcoinChainService>` returned through `UniFFI` becomes a
/// foreign-callable trait reference, and every later method call from Rust
/// hops back through FFI. The async fn called inside that hop runs without
/// the surrounding tokio runtime context (e.g. reqwest's per-request
/// `tokio::time::sleep` in `Client::execute_request`), which panics with
/// "no reactor running, must be called from the context of a Tokio 1.x
/// runtime".
///
/// Wrapping the `Arc` in a concrete `UniFFI` Object handle keeps the trait
/// pointer purely on the Rust side — foreign-language callers see an
/// opaque handle with no methods. When
/// [`SdkBuilder::with_chain_service`](crate::SdkBuilder::with_chain_service)
/// receives it, the SDK extracts the inner `Arc<dyn ...>` and dispatches
/// natively.
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct BitcoinChainServiceHandle {
    pub(crate) inner: Arc<dyn BitcoinChainService>,
}

impl BitcoinChainServiceHandle {
    /// Construct a handle directly from a Rust-side `Arc<dyn BitcoinChainService>`.
    ///
    /// Useful when wrapping an SDK-internal chain service for sharing across
    /// SDK instances without going through [`new_rest_chain_service`].
    #[must_use]
    pub fn new(inner: Arc<dyn BitcoinChainService>) -> Arc<Self> {
        Arc::new(Self { inner })
    }

    /// Returns a clone of the underlying chain service `Arc`.
    #[must_use]
    pub fn inner(&self) -> Arc<dyn BitcoinChainService> {
        self.inner.clone()
    }
}

/// Wraps a foreign-language `BitcoinChainService` implementation in a handle
/// suitable for [`SdkBuilder::with_chain_service`](crate::SdkBuilder::with_chain_service).
///
/// Use this when a foreign-language integration provides its own
/// `BitcoinChainService` implementation. The resulting handle still routes
/// every method call through `UniFFI` to the foreign impl — that's correct,
/// since the impl really is foreign — but uses the same single-method
/// handle-based API as the Rust-built case.
///
/// Only built when the `uniffi` feature is enabled. Rust callers should
/// use [`BitcoinChainServiceHandle::new`] directly.
#[cfg(feature = "uniffi")]
#[uniffi::export]
#[must_use]
pub fn wrap_chain_service(
    chain_service: Arc<dyn BitcoinChainService>,
) -> Arc<BitcoinChainServiceHandle> {
    BitcoinChainServiceHandle::new(chain_service)
}

/// Constructs a shareable REST-based [`BitcoinChainService`].
///
/// Pass the returned handle to multiple [`SdkBuilder`](crate::SdkBuilder)s
/// via [`SdkBuilder::with_chain_service`](crate::SdkBuilder::with_chain_service)
/// to reuse a single underlying HTTP client (and its connection pool) across
/// SDK instances. All SDKs sharing the service must use the same `network`.
///
/// In Rust the core builder's [`SdkBuilder::with_chain_service`] takes an
/// `Arc<dyn BitcoinChainService>` directly, so call `.inner()` on the
/// returned handle first. All other bindings take the handle as-is.
///
/// For one-off, non-shared use, prefer
/// [`SdkBuilder::with_rest_chain_service`](crate::SdkBuilder::with_rest_chain_service).
#[cfg_attr(feature = "uniffi", uniffi::export)]
#[must_use]
pub fn new_rest_chain_service(
    url: String,
    network: Network,
    api_type: ChainApiType,
    credentials: Option<Credentials>,
) -> Arc<BitcoinChainServiceHandle> {
    let http_client: Arc<dyn HttpClient> = Arc::new(DefaultHttpClient::default());
    BitcoinChainServiceHandle::new(Arc::new(RestClientChainService::new(
        url,
        network,
        5,
        http_client,
        credentials.map(|c| BasicAuth::new(c.username, c.password)),
        api_type,
    )))
}
