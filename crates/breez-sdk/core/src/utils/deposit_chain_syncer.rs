use std::{collections::HashMap, sync::Arc};

use spark_wallet::{DefaultSigner, SparkWallet};
use tracing::{error, info};

use crate::{
    BitcoinChainService, SdkError,
    persist::Storage,
    utils::utxo_fetcher::{CachedUtxoFetcher, DetailedUtxo},
};

pub struct DepositChainSyncer {
    chain_service: Arc<dyn BitcoinChainService>,
    storage: Arc<dyn Storage>,
    spark_wallet: Arc<SparkWallet<DefaultSigner>>,
    utxo_fetcher: CachedUtxoFetcher,
}

impl DepositChainSyncer {
    pub fn new(
        chain_service: Arc<dyn BitcoinChainService>,
        storage: Arc<dyn Storage>,
        spark_wallet: Arc<SparkWallet<DefaultSigner>>,
    ) -> Self {
        Self {
            chain_service: chain_service.clone(),
            storage: storage.clone(),
            spark_wallet,
            utxo_fetcher: CachedUtxoFetcher::new(chain_service, storage),
        }
    }

    pub async fn sync(&self) -> Result<Vec<DetailedUtxo>, SdkError> {
        let addresses = self
            .spark_wallet
            .list_static_deposit_addresses(None)
            .await?;

        // First add all existing deopsits to the storage
        let mut all_utxos = HashMap::new();
        for address in addresses {
            info!("Checking static deposit address: {}", address.to_string());
            let utxos = self
                .chain_service
                .get_address_utxos(address.to_string())
                .await?;
            for utxo in utxos {
                let detailed_utxo = match self
                    .utxo_fetcher
                    .fetch_detailed_utxo(&utxo.txid, utxo.vout)
                    .await
                {
                    Ok(detailed_utxo) => detailed_utxo,
                    Err(e) => {
                        error!("Failed to convert utxo {}:{}: {e}", utxo.txid, utxo.vout);
                        continue;
                    }
                };
                self.storage.add_deposit(
                    detailed_utxo.txid.to_string(),
                    detailed_utxo.vout,
                    detailed_utxo.value,
                )?;
                all_utxos.insert(
                    format!("{}:{}", detailed_utxo.txid, detailed_utxo.vout),
                    detailed_utxo,
                );
            }
        }

        // Now remove all deposits that are no longer claimable and not refunded
        let deposits = self.storage.list_deposits()?;
        for deposit in deposits {
            let key = format!("{}:{}", deposit.txid, deposit.vout);
            if deposit.refund_tx_id.is_none() && !all_utxos.contains_key(&key) {
                self.storage.delete_deposit(deposit.txid, deposit.vout)?;
            }
        }

        Ok(all_utxos.values().cloned().collect())
    }
}
