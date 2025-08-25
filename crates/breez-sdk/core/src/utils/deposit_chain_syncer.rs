use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use spark_wallet::{DefaultSigner, SparkWallet};
use tracing::{error, info};

use crate::{
    BitcoinChainService, Network, SdkError,
    persist::Storage,
    utils::utxo_fetcher::{CachedUtxoFetcher, DetailedUtxo},
};

pub struct DepositChainSyncer {
    chain_service: Arc<dyn BitcoinChainService>,
    storage: Arc<dyn Storage>,
    spark_wallet: Arc<SparkWallet<DefaultSigner>>,
    utxo_fetcher: CachedUtxoFetcher,
    network: Network,
}

impl DepositChainSyncer {
    pub fn new(
        chain_service: Arc<dyn BitcoinChainService>,
        storage: Arc<dyn Storage>,
        spark_wallet: Arc<SparkWallet<DefaultSigner>>,
        network: Network,
    ) -> Self {
        Self {
            chain_service: chain_service.clone(),
            storage: storage.clone(),
            spark_wallet,
            utxo_fetcher: CachedUtxoFetcher::new(chain_service, storage),
            network,
        }
    }

    pub async fn sync(&self) -> Result<Vec<DetailedUtxo>, SdkError> {
        let addresses = self
            .spark_wallet
            .list_static_deposit_addresses(None)
            .await?;

        // First add all existing deposits to the storage
        let mut all_utxos = HashMap::new();
        for address in addresses {
            info!("Checking static deposit address: {}", address.to_string());

            let utxos = match self.network {
                Network::Mainnet => self
                    .chain_service
                    .get_address_utxos(address.to_string())
                    .await?
                    .iter()
                    .map(|utxo| (utxo.txid.to_string(), utxo.vout))
                    .collect::<Vec<(String, u32)>>(),
                Network::Regtest => self
                    .spark_wallet
                    .get_utxos_for_address(&address.to_string())
                    .await?
                    .iter()
                    .map(|utxo| (utxo.txid.to_string(), utxo.vout))
                    .collect::<Vec<(String, u32)>>(),
            };

            for utxo in utxos {
                let detailed_utxo =
                    match self.utxo_fetcher.fetch_detailed_utxo(&utxo.0, utxo.1).await {
                        Ok(detailed_utxo) => detailed_utxo,
                        Err(e) => {
                            error!("Failed to convert utxo {}:{}: {e}", utxo.0, utxo.1);
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
        let mut refunded = HashSet::new();
        for deposit in deposits {
            let key = format!("{}:{}", deposit.txid, deposit.vout);
            match deposit.refund_tx_id {
                Some(txid) => {
                    refunded.insert(format!("{}:{}", txid, deposit.vout));
                }
                None => {
                    if !all_utxos.contains_key(&key) {
                        self.storage.delete_deposit(deposit.txid, deposit.vout)?;
                    }
                }
            }
        }

        Ok(all_utxos
            .values()
            .filter(|u| !refunded.contains(&format!("{}:{}", u.txid, u.vout)))
            .cloned()
            .collect())
    }
}
