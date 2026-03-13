use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use spark_wallet::SparkWallet;
use tracing::{error, info, warn};

use crate::{
    BitcoinChainService, SdkError,
    persist::Storage,
    utils::utxo_fetcher::{CachedUtxoFetcher, DetailedUtxo},
};

const UTXO_PAGE_SIZE: u32 = 100;

pub struct DepositChainSyncer {
    storage: Arc<dyn Storage>,
    spark_wallet: Arc<SparkWallet>,
    utxo_fetcher: CachedUtxoFetcher,
    chain_service: Arc<dyn BitcoinChainService>,
}

#[derive(Eq, Hash, PartialEq, Clone)]
struct TxOutput {
    txid: String,
    vout: u32,
}

impl DepositChainSyncer {
    pub fn new(
        chain_service: Arc<dyn BitcoinChainService>,
        storage: Arc<dyn Storage>,
        spark_wallet: Arc<SparkWallet>,
    ) -> Self {
        Self {
            storage: storage.clone(),
            spark_wallet,
            utxo_fetcher: CachedUtxoFetcher::new(chain_service.clone(), storage),
            chain_service,
        }
    }

    pub async fn sync(&self) -> Result<Vec<DetailedUtxo>, SdkError> {
        let mut detailed_utxos = HashMap::new();
        let mut cursor = None;
        let mut hit_error = false;

        // Process UTXOs page by page, fetching tx details sequentially.
        // On fetch errors we stop processing but still reconcile what succeeded.
        loop {
            let (utxos, next_cursor) = self
                .spark_wallet
                .get_utxos_for_identity(UTXO_PAGE_SIZE, cursor)
                .await?;

            for utxo in &utxos {
                let txid_str = utxo.txid.to_string();
                match self
                    .utxo_fetcher
                    .fetch_detailed_utxo(&txid_str, utxo.vout)
                    .await
                {
                    Ok(detailed_utxo) => {
                        self.storage
                            .add_deposit(
                                detailed_utxo.txid.to_string(),
                                detailed_utxo.vout,
                                detailed_utxo.value,
                            )
                            .await?;
                        detailed_utxos.insert(
                            format!("{}:{}", detailed_utxo.txid, detailed_utxo.vout),
                            detailed_utxo,
                        );
                    }
                    Err(e) => {
                        warn!(
                            "Failed to fetch utxo details, processing {} fetched so far: {e}",
                            detailed_utxos.len()
                        );
                        hit_error = true;
                        break;
                    }
                }
            }

            if hit_error || next_cursor.is_none() {
                break;
            }
            cursor = next_cursor;
        }

        let refunded = self.reconcile_deposits(&detailed_utxos).await?;

        Ok(detailed_utxos
            .values()
            .filter(|u| {
                !refunded.contains(&TxOutput {
                    txid: u.txid.to_string(),
                    vout: u.vout,
                })
            })
            .cloned()
            .collect())
    }

    /// Removes stale deposits and checks refund confirmations.
    /// Returns the set of refunded outputs.
    async fn reconcile_deposits(
        &self,
        all_utxos: &HashMap<String, DetailedUtxo>,
    ) -> Result<HashSet<TxOutput>, SdkError> {
        let deposits = self.storage.list_deposits().await?;
        let mut refunded = HashSet::new();
        let mut refunded_deposits = HashMap::new();
        for deposit in deposits {
            let key = format!("{}:{}", deposit.txid, deposit.vout);
            match deposit.refund_tx_id.clone() {
                Some(txid) => {
                    info!(
                        "Found refund transaction {}:{} deposit tx: {}",
                        txid, deposit.vout, deposit.txid
                    );
                    refunded.insert(TxOutput {
                        txid: deposit.txid.clone(),
                        vout: deposit.vout,
                    });
                    refunded_deposits.insert(txid, deposit.clone());
                }
                None => {
                    if !all_utxos.contains_key(&key) {
                        self.storage
                            .delete_deposit(deposit.txid, deposit.vout)
                            .await?;
                    }
                }
            }
        }

        for (refund_tx_id, deposit) in &refunded_deposits {
            info!(
                "Checking refund transaction {}:{}",
                deposit.txid, deposit.vout
            );
            let status = self
                .chain_service
                .get_transaction_status(refund_tx_id.clone())
                .await;
            match status {
                Ok(status) => {
                    if status.confirmed {
                        self.storage
                            .delete_deposit(deposit.txid.clone(), deposit.vout)
                            .await?;
                    }
                }
                Err(e) => {
                    error!(
                        "Failed to download refund transaction {}:{}: {e}",
                        refund_tx_id, deposit.vout
                    );
                }
            }
        }

        Ok(refunded)
    }
}
