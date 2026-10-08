//! A chain service for tests: it returns what a test put in it, and fails
//! every other lookup.

use std::{
    collections::HashMap,
    sync::atomic::{AtomicUsize, Ordering},
};

use bitcoin::{Amount, OutPoint, ScriptBuf, Transaction, TxIn, TxOut};

use super::{BitcoinChainService, ChainServiceError, Outspend, RecommendedFees, TxStatus, Utxo};

#[derive(Default)]
pub(crate) struct ChainStub {
    pub(crate) tip: Option<u32>,
    /// Confirmation heights, by txid.
    pub(crate) heights: HashMap<String, u32>,
    pub(crate) outspends: HashMap<(String, u32), Outspend>,
    pub(crate) transactions: HashMap<String, String>,
    /// Every output paid to an address, by address.
    pub(crate) address_txos: HashMap<String, Vec<Utxo>>,
    /// How many requests the stub received.
    pub(crate) requests: AtomicUsize,
}

impl ChainStub {
    pub(crate) fn spent(txid: &str, confirmed: bool, block_height: Option<u32>) -> Outspend {
        Outspend::Spent {
            txid: txid.to_string(),
            vin: 0,
            status: TxStatus {
                confirmed,
                block_height,
                block_time: None,
            },
        }
    }
}

pub(crate) fn tx_paying(previous_output: OutPoint, value_sats: u64) -> Transaction {
    Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: bitcoin::absolute::LockTime::ZERO,
        input: vec![TxIn {
            previous_output,
            ..Default::default()
        }],
        output: vec![TxOut {
            value: Amount::from_sat(value_sats),
            script_pubkey: ScriptBuf::new(),
        }],
    }
}

fn unreachable_chain() -> ChainServiceError {
    ChainServiceError::ServiceConnectivity("unreachable".to_string())
}

#[macros::async_trait]
impl BitcoinChainService for ChainStub {
    async fn get_address_utxos(&self, _address: String) -> Result<Vec<Utxo>, ChainServiceError> {
        Err(unreachable_chain())
    }

    async fn get_address_txos(&self, address: String) -> Result<Vec<Utxo>, ChainServiceError> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        self.address_txos
            .get(&address)
            .cloned()
            .ok_or_else(unreachable_chain)
    }

    async fn get_transaction_status(&self, txid: String) -> Result<TxStatus, ChainServiceError> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        self.heights
            .get(&txid)
            .map(|height| TxStatus {
                confirmed: true,
                block_height: Some(*height),
                block_time: None,
            })
            .ok_or_else(unreachable_chain)
    }

    async fn tip_height(&self) -> Result<u32, ChainServiceError> {
        self.tip.ok_or_else(unreachable_chain)
    }

    async fn get_transaction_hex(&self, txid: String) -> Result<String, ChainServiceError> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        self.transactions
            .get(&txid)
            .cloned()
            .ok_or_else(unreachable_chain)
    }

    async fn get_outspend(&self, txid: String, vout: u32) -> Result<Outspend, ChainServiceError> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        self.outspends
            .get(&(txid, vout))
            .cloned()
            .ok_or_else(unreachable_chain)
    }

    async fn broadcast_transaction(&self, _tx: String) -> Result<(), ChainServiceError> {
        Err(unreachable_chain())
    }

    async fn recommended_fees(&self) -> Result<RecommendedFees, ChainServiceError> {
        Err(unreachable_chain())
    }
}
