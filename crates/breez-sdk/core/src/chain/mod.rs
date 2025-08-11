use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::Network;
pub mod rest_client;

#[derive(Debug, Error)]
pub enum ChainServiceError {
    #[error("Invalid address: {0}")]
    InvalidAddress(String),
    #[error("Http error: {status} - {message}")]
    HttpError { status: u16, message: String },
    #[error("General error: {0}")]
    GenericError(String),
}

#[breez_sdk_macros::async_trait]
pub trait BitcoinChainService {
    async fn get_address_utxos(&self, address: &str) -> Result<Vec<Utxo>, ChainServiceError>;
    async fn get_transaction_hex(&self, txid: &str) -> Result<String, ChainServiceError>;
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PrevOut {
    pub value: u64,
    pub scriptpubkey: ScriptBuf,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Vin {
    pub txid: String,
    pub vout: u32,
    // None if coinbase
    pub prevout: Option<PrevOut>,
    pub scriptsig: ScriptBuf,
    pub sequence: u32,
    pub is_coinbase: bool,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Vout {
    pub value: u64,
    pub scriptpubkey: ScriptBuf,
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct TxStatus {
    pub confirmed: bool,
    pub block_height: Option<u32>,
    pub block_time: Option<u64>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Tx {
    pub txid: String,
    pub version: i32,
    pub locktime: u32,
    pub vin: Vec<Vin>,
    pub vout: Vec<Vout>,
    /// Transaction size in raw bytes (NOT virtual bytes).
    pub size: usize,
    /// Transaction weight units.
    pub weight: u64,
    pub status: TxStatus,
    pub fee: u64,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
struct TxOutput {
    txid: String,
    vout: u32,
    amount: u64,
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Utxo {
    pub txid: String,
    pub vout: u32,
    pub value: u64,
    pub status: TxStatus,
}

impl TryFrom<Network> for bitcoin::Network {
    type Error = ChainServiceError;

    fn try_from(value: Network) -> Result<Self, Self::Error> {
        match value {
            Network::Mainnet => Ok(bitcoin::Network::Bitcoin),
            Network::Regtest => Ok(bitcoin::Network::Regtest),
        }
    }
}

impl From<bitcoin::address::ParseError> for ChainServiceError {
    fn from(value: bitcoin::address::ParseError) -> Self {
        ChainServiceError::InvalidAddress(value.to_string())
    }
}
