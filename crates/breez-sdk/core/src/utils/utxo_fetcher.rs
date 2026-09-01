use std::{str::FromStr, sync::Arc};

use bitcoin::{Transaction, Txid, consensus::encode::deserialize_hex};
use tracing::warn;

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

impl DetailedUtxo {
    pub fn into_deposit_info(self, is_mature: bool) -> DepositInfo {
        DepositInfo {
            txid: self.txid.to_string(),
            vout: self.vout,
            amount_sats: self.value,
            is_mature,
            refund_tx: None,
            refund_tx_id: None,
            refund_state: None,
            claim_error: None,
            instant_claim_status: None,
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
        let requested_txid = Txid::from_str(txid)
            .map_err(|_| SdkError::Generic("Failed to parse txid".to_string()))?;
        let cache = ObjectCacheRepository::new(self.storage.clone());

        // Cached entries are revalidated on every read: one that fails the
        // txid rebind (poisoned by a misbehaving backend, or corrupt) is
        // evicted and refetched instead of being served forever. A read error
        // (e.g. an entry whose stored JSON does not parse) is a miss too: the
        // refetch overwrites the bad entry.
        let tx = match cache.fetch_tx(txid).await {
            Ok(Some(cached)) => match verify_tx_hex(&cached.raw_tx, requested_txid) {
                Ok(tx) => tx,
                Err(e) => {
                    warn!("Evicting cached tx {txid} failing integrity check: {e}");
                    cache.delete_tx(txid).await?;
                    self.fetch_verified(&cache, txid, requested_txid).await?
                }
            },
            Ok(None) => self.fetch_verified(&cache, txid, requested_txid).await?,
            Err(e) => {
                warn!("Failed to read cached tx {txid}, refetching: {e}");
                self.fetch_verified(&cache, txid, requested_txid).await?
            }
        };

        let txout = tx.output.get(vout as usize).ok_or(SdkError::MissingUtxo {
            tx: txid.to_string(),
            vout,
        })?;
        let amount_sats = txout.value.to_sat();
        Ok(DetailedUtxo {
            tx,
            vout,
            txid: requested_txid,
            value: amount_sats,
        })
    }

    /// Fetches the tx from the chain backend and rebinds it to the requested
    /// txid before cache insertion, so a wrong response is rejected instead of
    /// poisoning the cache.
    async fn fetch_verified(
        &self,
        cache: &ObjectCacheRepository,
        txid: &str,
        requested_txid: Txid,
    ) -> Result<Transaction, SdkError> {
        let tx_hex = self
            .chain_service
            .get_transaction_hex(txid.to_string())
            .await?;
        let tx = verify_tx_hex(&tx_hex, requested_txid)?;
        cache.save_tx(txid, &CachedTx { raw_tx: tx_hex }).await?;
        Ok(tx)
    }
}

/// Parses `tx_hex` and requires its recomputed txid to equal `requested_txid`,
/// surfacing a mismatch as an explicit integrity error rather than a missing
/// UTXO.
fn verify_tx_hex(tx_hex: &str, requested_txid: Txid) -> Result<Transaction, SdkError> {
    let tx: Transaction = deserialize_hex(tx_hex)?;
    let actual_txid = tx.compute_txid();
    if actual_txid != requested_txid {
        return Err(SdkError::ChainServiceError(format!(
            "integrity check failed: requested transaction {requested_txid}, backend returned {actual_txid}"
        )));
    }
    Ok(tx)
}

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use super::*;
    use crate::chain::{ChainServiceError, Outspend, RecommendedFees, TxStatus, Utxo};
    use bitcoin::consensus::encode::serialize_hex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Chain service that always answers `get_transaction_hex` with the same
    /// hex, counting calls. Every other method is unreachable in these tests.
    struct FixedTxChainService {
        tx_hex: String,
        calls: AtomicUsize,
    }

    impl FixedTxChainService {
        fn new(tx: &Transaction) -> Self {
            Self {
                tx_hex: serialize_hex(tx),
                calls: AtomicUsize::new(0),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[macros::async_trait]
    impl BitcoinChainService for FixedTxChainService {
        async fn get_address_utxos(
            &self,
            _address: String,
        ) -> Result<Vec<Utxo>, ChainServiceError> {
            unreachable!()
        }

        async fn get_address_txos(&self, _address: String) -> Result<Vec<Utxo>, ChainServiceError> {
            unreachable!()
        }

        async fn get_transaction_status(
            &self,
            _txid: String,
        ) -> Result<TxStatus, ChainServiceError> {
            unreachable!()
        }

        async fn get_transaction_hex(&self, _txid: String) -> Result<String, ChainServiceError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.tx_hex.clone())
        }

        async fn get_outspend(
            &self,
            _txid: String,
            _vout: u32,
        ) -> Result<Outspend, ChainServiceError> {
            unreachable!()
        }

        async fn broadcast_transaction(&self, _tx: String) -> Result<(), ChainServiceError> {
            unreachable!()
        }

        async fn recommended_fees(&self) -> Result<RecommendedFees, ChainServiceError> {
            unreachable!()
        }

        async fn tip_height(&self) -> Result<u32, ChainServiceError> {
            unreachable!()
        }
    }

    fn test_tx(value_sat: u64) -> Transaction {
        Transaction {
            version: bitcoin::transaction::Version::TWO,
            lock_time: bitcoin::absolute::LockTime::ZERO,
            input: vec![],
            output: vec![bitcoin::TxOut {
                value: bitcoin::Amount::from_sat(value_sat),
                script_pubkey: bitcoin::ScriptBuf::new(),
            }],
        }
    }

    fn test_storage() -> Arc<dyn Storage> {
        let mut dir = std::env::temp_dir();
        dir.push(format!("breez-utxo-fetcher-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        Arc::new(crate::SqliteStorage::new(&dir).unwrap())
    }

    #[tokio::test]
    async fn fetches_verifies_and_caches_a_matching_tx() {
        let tx = test_tx(1234);
        let txid = tx.compute_txid();
        let chain = Arc::new(FixedTxChainService::new(&tx));
        let fetcher = CachedUtxoFetcher::new(chain.clone(), test_storage());

        let utxo = fetcher
            .fetch_detailed_utxo(&txid.to_string(), 0)
            .await
            .unwrap();
        assert_eq!(utxo.txid, txid);
        assert_eq!(utxo.value, 1234);

        // The second read is served from the cache.
        let utxo = fetcher
            .fetch_detailed_utxo(&txid.to_string(), 0)
            .await
            .unwrap();
        assert_eq!(utxo.value, 1234);
        assert_eq!(chain.calls(), 1);
    }

    #[tokio::test]
    async fn rejects_a_mismatched_tx_without_caching_it() {
        let requested = test_tx(1234);
        let substituted = test_tx(5678);
        let txid = requested.compute_txid();
        let chain = Arc::new(FixedTxChainService::new(&substituted));
        let storage = test_storage();
        let fetcher = CachedUtxoFetcher::new(chain, storage.clone());

        let err = fetcher
            .fetch_detailed_utxo(&txid.to_string(), 0)
            .await
            .unwrap_err();
        assert!(
            matches!(&err, SdkError::ChainServiceError(m) if m.contains("integrity")),
            "unexpected error: {err:?}"
        );

        // The wrong transaction must not have been cached.
        let cache = ObjectCacheRepository::new(storage);
        assert!(cache.fetch_tx(&txid.to_string()).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn evicts_and_refetches_a_poisoned_cache_entry() {
        let tx = test_tx(1234);
        let poison = test_tx(5678);
        let txid = tx.compute_txid();
        let storage = test_storage();

        // Poison the cache: a different tx stored under the requested txid.
        ObjectCacheRepository::new(storage.clone())
            .save_tx(
                &txid.to_string(),
                &CachedTx {
                    raw_tx: serialize_hex(&poison),
                },
            )
            .await
            .unwrap();

        let chain = Arc::new(FixedTxChainService::new(&tx));
        let fetcher = CachedUtxoFetcher::new(chain.clone(), storage.clone());
        let utxo = fetcher
            .fetch_detailed_utxo(&txid.to_string(), 0)
            .await
            .unwrap();
        assert_eq!(utxo.value, 1234);
        assert_eq!(chain.calls(), 1);

        // The cache now holds the verified tx.
        let cached = ObjectCacheRepository::new(storage)
            .fetch_tx(&txid.to_string())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cached.raw_tx, serialize_hex(&tx));
    }

    #[tokio::test]
    async fn evicts_and_refetches_a_corrupt_cache_entry() {
        let tx = test_tx(1234);
        let txid = tx.compute_txid();
        let storage = test_storage();

        ObjectCacheRepository::new(storage.clone())
            .save_tx(
                &txid.to_string(),
                &CachedTx {
                    raw_tx: "not-hex".to_string(),
                },
            )
            .await
            .unwrap();

        let chain = Arc::new(FixedTxChainService::new(&tx));
        let fetcher = CachedUtxoFetcher::new(chain.clone(), storage);
        let utxo = fetcher
            .fetch_detailed_utxo(&txid.to_string(), 0)
            .await
            .unwrap();
        assert_eq!(utxo.value, 1234);
        assert_eq!(chain.calls(), 1);
    }

    #[tokio::test]
    async fn overwrites_a_cache_entry_whose_json_does_not_parse() {
        let tx = test_tx(1234);
        let txid = tx.compute_txid();
        let storage = test_storage();

        // Raw storage write bypassing CachedTx serialization: fetch_tx errors
        // on this entry instead of returning it.
        storage
            .set_cached_item(format!("tx_cache-{txid}"), "not-json".to_string())
            .await
            .unwrap();

        let chain = Arc::new(FixedTxChainService::new(&tx));
        let fetcher = CachedUtxoFetcher::new(chain.clone(), storage.clone());
        let utxo = fetcher
            .fetch_detailed_utxo(&txid.to_string(), 0)
            .await
            .unwrap();
        assert_eq!(utxo.value, 1234);
        assert_eq!(chain.calls(), 1);

        // The refetch overwrote the bad entry.
        let cached = ObjectCacheRepository::new(storage)
            .fetch_tx(&txid.to_string())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cached.raw_tx, serialize_hex(&tx));
    }

    #[tokio::test]
    async fn missing_vout_is_reported_as_missing_utxo() {
        let tx = test_tx(1234);
        let txid = tx.compute_txid();
        let chain = Arc::new(FixedTxChainService::new(&tx));
        let fetcher = CachedUtxoFetcher::new(chain, test_storage());

        let err = fetcher
            .fetch_detailed_utxo(&txid.to_string(), 1)
            .await
            .unwrap_err();
        assert!(matches!(err, SdkError::MissingUtxo { .. }));
    }
}
