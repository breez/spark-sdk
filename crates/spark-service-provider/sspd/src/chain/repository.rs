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

#[derive(Debug)]
pub struct Spender {
    pub outpoint: OutPoint,
    pub txid: Txid,
    pub block_height: u64,
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
    async fn add_watch_address(&self, address: &Address) -> Result<(), ChainRepositoryError>;
    async fn filter_watch_addresses(
        &self,
        addresses: &[Address],
    ) -> Result<Vec<Address>, ChainRepositoryError>;
    async fn get_txos_for_address(
        &self,
        address: &Address,
    ) -> Result<Vec<Txo>, ChainRepositoryError>;
    /// Only spends by confirmed transactions are returned.
    async fn get_spenders(
        &self,
        outpoints: &[OutPoint],
    ) -> Result<Vec<Spender>, ChainRepositoryError>;
}
