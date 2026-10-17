use std::time::Duration;

use bitcoin::{Block, BlockHash, FeeRate, OutPoint, Transaction, Txid};
use thiserror::Error;

use super::{BlockHeader, ChainRepositoryError};

#[derive(Debug, Error)]
pub enum BroadcastError {
    #[error("{0}")]
    Chain(ChainError),
    /// The transaction is in the mempool, or has an unspent output in the chain.
    #[error("transaction already known")]
    AlreadyKnown,
    #[error("unknown error: {0}")]
    UnknownError(String),
}

#[derive(Debug, Error)]
pub enum ChainError {
    #[error("{0}")]
    Database(ChainRepositoryError),
    #[error("{0}")]
    General(Box<dyn std::error::Error + Sync + Send>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutputSpend {
    pub outpoint: OutPoint,
    pub spender: Txid,
    /// `None` while the spending transaction is in the mempool.
    pub block: Option<BlockHeader>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncState {
    pub height: u64,
    /// Whether bitcoind has finished its initial block download and has
    /// validated a block for every header it has.
    pub synced: bool,
}

#[async_trait::async_trait]
pub trait ChainClient {
    async fn broadcast_tx(&self, tx: Transaction) -> Result<(), BroadcastError>;
    /// Broadcasts `txs` as one package: the last spends outputs of the others.
    async fn broadcast_package(&self, txs: &[Transaction]) -> Result<(), BroadcastError>;
    /// Estimates the fee rate for confirmation within `conf_target` blocks.
    async fn estimate_fee_rate(&self, conf_target: u32) -> Result<FeeRate, ChainError>;
    async fn sync_state(&self) -> Result<SyncState, ChainError>;
    async fn get_best_block_hash(&self) -> Result<BlockHash, ChainError>;
    /// `None` above the node's tip.
    async fn get_block_hash(&self, height: u64) -> Result<Option<BlockHash>, ChainError>;
    async fn get_block(&self, hash: &BlockHash) -> Result<Block, ChainError>;
    /// Returns the spend of each of `outpoints` that is an input of a
    /// transaction in the chain or in the mempool.
    async fn get_spends(&self, outpoints: &[OutPoint]) -> Result<Vec<OutputSpend>, ChainError>;
    /// Returns once the node's tip is at `height` or higher, or after `timeout`.
    async fn wait_for_block_height(&self, height: u64, timeout: Duration)
    -> Result<(), ChainError>;
}
