use std::collections::{HashMap, HashSet};

use bitcoin::{Address, BlockHash, OutPoint, Txid};
use thiserror::Error;

use super::types::{BlockHeader, Txo};

#[derive(Debug, Error)]
pub enum ChainRepositoryError {
    #[error("{0}")]
    General(Box<dyn std::error::Error + Send + Sync>),
}

#[derive(Debug)]
pub struct AddressUtxo {
    pub address: Address,
    pub utxo: Txo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spender {
    pub outpoint: OutPoint,
    pub txid: Txid,
    pub block_height: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct WatchedOutput {
    pub outpoint: OutPoint,
    /// The input of the transaction that has this output, when the caller has
    /// that transaction and has registered that input too.
    pub input: Option<OutPoint>,
}

#[derive(Debug, Default)]
pub struct OutputStates {
    pub spent: HashMap<OutPoint, Spender>,
    /// The outputs that the spend check has yet to check with bitcoind.
    pub unchecked: HashSet<OutPoint>,
}

/// A watched output that is on chain and unspent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfirmedOutput {
    pub outpoint: OutPoint,
    /// The height of the block that contains its transaction.
    pub height: u64,
}

/// The confirmations bitcoind counted for transaction `txid` while bitcoind's
/// tip was `best_block`.
#[derive(Debug, Clone, Copy)]
pub struct TransactionConfirmations {
    pub txid: Txid,
    pub best_block: BlockHash,
    pub confirmations: u64,
}

#[derive(Debug)]
pub struct SpentTxo {
    pub outpoint: OutPoint,
    pub spending_tx: Txid,
    pub spending_input_index: u32,
}

#[async_trait::async_trait]
pub trait ChainRepository {
    /// Stores `block` with `tx_outputs`, and the spends among `tx_inputs` of
    /// outputs already stored, as one change.
    async fn add_block(
        &self,
        block: &BlockHeader,
        tx_outputs: &[AddressUtxo],
        tx_inputs: &[SpentTxo],
    ) -> Result<(), ChainRepositoryError>;
    async fn undo_block(&self, hash: BlockHash) -> Result<(), ChainRepositoryError>;
    async fn get_block_hashes(&self, height: u64) -> Result<Vec<BlockHash>, ChainRepositoryError>;
    async fn get_tip(&self) -> Result<Option<BlockHeader>, ChainRepositoryError>;
    async fn get_base(&self) -> Result<Option<BlockHeader>, ChainRepositoryError>;
    async fn add_watch_addresses(&self, addresses: &[Address]) -> Result<(), ChainRepositoryError>;
    async fn filter_watch_addresses(
        &self,
        addresses: &[Address],
    ) -> Result<Vec<Address>, ChainRepositoryError>;
    /// The outputs at `addresses` that are in a block of the chain record.
    async fn get_txos_for_addresses(
        &self,
        addresses: &[Address],
    ) -> Result<Vec<Txo>, ChainRepositoryError>;
    /// Only spends by confirmed transactions are returned.
    async fn get_spenders(
        &self,
        outpoints: &[OutPoint],
    ) -> Result<Vec<Spender>, ChainRepositoryError>;
}
