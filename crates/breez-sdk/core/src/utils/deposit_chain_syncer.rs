use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use spark_wallet::SparkWallet;
use tracing::{error, info};

use crate::{
    BitcoinChainService, SdkError,
    persist::Storage,
    utils::utxo_fetcher::{CachedUtxoFetcher, DetailedUtxo},
};

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
        let addresses = self
            .spark_wallet
            .list_static_deposit_addresses(None)
            .await?;

        // First add all existing deposits to the storage
        let mut all_utxos = HashMap::new();
        for address in addresses.items {
            info!("Checking static deposit address: {}", address.to_string());

            let utxos = self
                .spark_wallet
                .get_utxos_for_address(&address.to_string())
                .await?
                .iter()
                .map(|utxo| (utxo.txid.to_string(), utxo.vout))
                .collect::<Vec<(String, u32)>>();

            for utxo in utxos {
                let detailed_utxo =
                    match self.utxo_fetcher.fetch_detailed_utxo(&utxo.0, utxo.1).await {
                        Ok(detailed_utxo) => detailed_utxo,
                        Err(e) => {
                            error!("Failed to convert utxo {}:{}: {e}", utxo.0, utxo.1);
                            continue;
                        }
                    };
                self.storage
                    .add_deposit(
                        detailed_utxo.txid.to_string(),
                        detailed_utxo.vout,
                        detailed_utxo.value,
                    )
                    .await?;
                all_utxos.insert(
                    format!("{}:{}", detailed_utxo.txid, detailed_utxo.vout),
                    detailed_utxo,
                );
            }
        }

        // Now remove all deposits that are no longer claimable and not refunded
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

        Ok(all_utxos
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
}
