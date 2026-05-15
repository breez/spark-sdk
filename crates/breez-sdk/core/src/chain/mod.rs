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

/// Constructs a shareable REST-based [`BitcoinChainService`].
///
/// Pass the returned `Arc` to multiple [`SdkBuilder`](crate::SdkBuilder)s via
/// [`SdkBuilder::with_chain_service`](crate::SdkBuilder::with_chain_service)
/// to reuse a single underlying HTTP client (and its connection pool) across
/// SDK instances. All SDKs sharing the service must use the same `network`.
///
/// For one-off, non-shared use, prefer
/// [`SdkBuilder::with_rest_chain_service`](crate::SdkBuilder::with_rest_chain_service).
#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
#[must_use]
pub async fn new_rest_chain_service(
    url: String,
    network: Network,
    api_type: ChainApiType,
    credentials: Option<Credentials>,
) -> Arc<dyn BitcoinChainService> {
    let http_client: Arc<dyn HttpClient> = Arc::new(DefaultHttpClient::default());
    Arc::new(RestClientChainService::new(
        url,
        network,
        5,
        http_client,
        credentials.map(|c| BasicAuth::new(c.username, c.password)),
        api_type,
    ))
}
