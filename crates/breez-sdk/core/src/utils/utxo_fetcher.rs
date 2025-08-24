use std::{str::FromStr, sync::Arc};

use bitcoin::{Transaction, Txid, consensus::encode::deserialize_hex};

use crate::{
    BitcoinChainService, DepositInfo, SdkError,
    persist::{CachedTx, ObjectCacheRepository, Storage},
};

#[derive(Debug, Clone)]
pub(crate) struct DetailedUtxo {
    pub tx: Transaction,
    pub vout: u32,
    pub txid: Txid,
    pub value: u64,
}

impl From<DetailedUtxo> for DepositInfo {
    fn from(detailed_utxo: DetailedUtxo) -> Self {
        DepositInfo {
            txid: detailed_utxo.txid.to_string(),
            vout: detailed_utxo.vout,
            amount_sats: detailed_utxo.value,
            refund_tx: None,
            refund_tx_id: None,
            claim_error: None,
        }
    }
}

pub(crate) struct CachedUtxoFetcher {
    pub chain_service: Arc<dyn BitcoinChainService>,
    pub storage: Arc<dyn Storage>,
}

impl CachedUtxoFetcher {
    pub fn new(chain_service: Arc<dyn BitcoinChainService>, storage: Arc<dyn Storage>) -> Self {
        Self {
            chain_service,
            storage,
        }
    }

    pub async fn fetch_detailed_utxo(
        &self,
        txid: &str,
        vout: u32,
    ) -> Result<DetailedUtxo, SdkError> {
        let object_cache_repository = ObjectCacheRepository::new(self.storage.clone());
        let tx_hex = if let Some(tx) = object_cache_repository.fetch_tx(txid)? {
            tx.raw_tx
        } else {
            let tx_hex = self
                .chain_service
                .get_transaction_hex(txid.to_string())
                .await?;
            object_cache_repository.save_tx(
                txid,
                &CachedTx {
                    raw_tx: tx_hex.clone(),
                },
            )?;
            tx_hex
        };

        let tx: Transaction = deserialize_hex(tx_hex.as_str())?;
        let txout = tx.output.get(vout as usize).ok_or(SdkError::MissingUtxo {
            tx: txid.to_string(),
            vout,
        })?;
        let amount_sats = txout.value.to_sat();
        Ok(DetailedUtxo {
            tx,
            vout,
            txid: Txid::from_str(txid)
                .map_err(|_| SdkError::Generic("Failed to parse txid".to_string()))?,
            value: amount_sats,
        })
    }
}
