#![cfg_attr(
    all(target_family = "wasm", target_os = "unknown"),
    allow(clippy::arc_with_non_send_sync)
)]

use std::sync::Arc;

use breez_sdk_common::{
    breez_server::{BreezServer, PRODUCTION_BREEZSERVER_URL},
    fiat::FiatService,
    rest::ReqwestRestClient,
};

use crate::{
    BitcoinChainService, FiatCurrency, Network, Rate, RecommendedFees,
    chain::rest_client::{ChainApiType, RestClientChainService},
    error::SdkError,
    models::Config,
};

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
use crate::chain::rest_client::BasicAuth;

/// A standalone API for fiat rates, fiat currencies, and recommended BTC fees.
///
/// These operations don't require a wallet connection — they only need
/// a Breez server for fiat data and a chain service for fee estimation.
pub struct FiatApi {
    fiat_service: Arc<dyn FiatService>,
    chain_service: Arc<dyn BitcoinChainService>,
}

impl FiatApi {
    /// Create a new `FiatApi` from a `Config`.
    ///
    /// Sets up the Breez server (for fiat rates/currencies) and a chain
    /// service (for recommended fees) using the network from the config.
    pub fn new(config: &Config) -> Result<Self, SdkError> {
        let breez_server: Arc<dyn FiatService> = Arc::new(
            BreezServer::new(PRODUCTION_BREEZSERVER_URL, config.api_key.clone())
                .map_err(|e| SdkError::Generic(e.to_string()))?,
        );

        let inner_client =
            ReqwestRestClient::new().map_err(|e| SdkError::Generic(e.to_string()))?;

        let chain_service: Arc<dyn BitcoinChainService> = match config.network {
            Network::Mainnet => Arc::new(RestClientChainService::new(
                "https://blockstream.info/api".to_string(),
                config.network,
                5,
                Box::new(inner_client),
                None,
                ChainApiType::Esplora,
            )),
            #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
            Network::Regtest => Arc::new(RestClientChainService::new(
                "https://regtest-mempool.us-west-2.sparkinfra.net/api".to_string(),
                config.network,
                5,
                Box::new(inner_client),
                match (
                    std::env::var("CHAIN_SERVICE_USERNAME"),
                    std::env::var("CHAIN_SERVICE_PASSWORD"),
                ) {
                    (Ok(username), Ok(password)) => Some(BasicAuth::new(username, password)),
                    _ => Some(BasicAuth::new(
                        "spark-sdk".to_string(),
                        "mCMk1JqlBNtetUNy".to_string(),
                    )),
                },
                ChainApiType::MempoolSpace,
            )),
            #[cfg(all(target_family = "wasm", target_os = "unknown"))]
            Network::Regtest => Arc::new(RestClientChainService::new(
                "https://regtest-mempool.us-west-2.sparkinfra.net/api".to_string(),
                config.network,
                5,
                Box::new(inner_client),
                None,
                ChainApiType::MempoolSpace,
            )),
        };

        Ok(Self {
            fiat_service: breez_server,
            chain_service,
        })
    }

    /// List fiat currencies for which there is a known exchange rate.
    pub async fn currencies(&self) -> Result<Vec<FiatCurrency>, SdkError> {
        Ok(self
            .fiat_service
            .fetch_fiat_currencies()
            .await?
            .into_iter()
            .map(From::from)
            .collect())
    }

    /// List the latest fiat exchange rates.
    pub async fn rates(&self) -> Result<Vec<Rate>, SdkError> {
        Ok(self
            .fiat_service
            .fetch_fiat_rates()
            .await?
            .into_iter()
            .map(From::from)
            .collect())
    }

    /// Get the recommended BTC fees.
    pub async fn recommended_fees(&self) -> Result<RecommendedFees, SdkError> {
        Ok(self.chain_service.recommended_fees().await?)
    }
}
