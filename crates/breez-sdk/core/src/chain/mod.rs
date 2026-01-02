use serde::{Deserialize, Serialize};
use thiserror::Error;

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
