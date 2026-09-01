use std::str::FromStr;
use std::sync::Arc;

use bitcoin::{Transaction, Txid, consensus::encode::deserialize_hex};

use super::{BitcoinChainService, ChainServiceError, Outspend, RecommendedFees, TxStatus, Utxo};

/// Wraps a [`BitcoinChainService`] and rebinds every fetched transaction to
/// the requested txid, so callers of the trait get the integrity check
/// regardless of which backend (built-in or user-supplied) is configured.
pub(crate) struct ValidatingChainService {
    inner: Arc<dyn BitcoinChainService>,
}

impl ValidatingChainService {
    pub(crate) fn new(inner: Arc<dyn BitcoinChainService>) -> Self {
        Self { inner }
    }
}

#[macros::async_trait]
impl BitcoinChainService for ValidatingChainService {
    async fn get_address_utxos(&self, address: String) -> Result<Vec<Utxo>, ChainServiceError> {
        self.inner.get_address_utxos(address).await
    }

    async fn get_address_txos(&self, address: String) -> Result<Vec<Utxo>, ChainServiceError> {
        self.inner.get_address_txos(address).await
    }

    async fn get_transaction_status(&self, txid: String) -> Result<TxStatus, ChainServiceError> {
        self.inner.get_transaction_status(txid).await
    }

    async fn get_transaction_hex(&self, txid: String) -> Result<String, ChainServiceError> {
        let tx_hex = self.inner.get_transaction_hex(txid.clone()).await?;
        verify_transaction_hex(&txid, &tx_hex)?;
        Ok(tx_hex)
    }

    async fn get_outspend(&self, txid: String, vout: u32) -> Result<Outspend, ChainServiceError> {
        self.inner.get_outspend(txid, vout).await
    }

    async fn broadcast_transaction(&self, tx: String) -> Result<(), ChainServiceError> {
        self.inner.broadcast_transaction(tx).await
    }

    async fn recommended_fees(&self) -> Result<RecommendedFees, ChainServiceError> {
        self.inner.recommended_fees().await
    }

    async fn tip_height(&self) -> Result<u32, ChainServiceError> {
        self.inner.tip_height().await
    }
}

/// Requires the recomputed txid of `tx_hex` to equal the requested `txid`,
/// rejecting a substituted or malformed backend response.
fn verify_transaction_hex(txid: &str, tx_hex: &str) -> Result<(), ChainServiceError> {
    let requested_txid = Txid::from_str(txid)
        .map_err(|e| ChainServiceError::Generic(format!("invalid txid {txid}: {e}")))?;
    let tx: Transaction = deserialize_hex(tx_hex).map_err(|e| {
        ChainServiceError::Generic(format!("invalid transaction hex for {txid}: {e}"))
    })?;
    let actual_txid = tx.compute_txid();
    if actual_txid != requested_txid {
        return Err(ChainServiceError::Generic(format!(
            "integrity check failed: requested transaction {requested_txid}, backend returned {actual_txid}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::{Transaction, consensus::encode::serialize_hex};

    use macros::async_test_all;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    /// Chain service that always answers `get_transaction_hex` with the same
    /// hex. Every other method is unreachable in these tests.
    struct FixedTxChainService {
        tx_hex: String,
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

    fn test_tx() -> Transaction {
        Transaction {
            version: bitcoin::transaction::Version::TWO,
            lock_time: bitcoin::absolute::LockTime::ZERO,
            input: vec![],
            output: vec![bitcoin::TxOut {
                value: bitcoin::Amount::from_sat(1234),
                script_pubkey: bitcoin::ScriptBuf::new(),
            }],
        }
    }

    fn validating_service(tx_hex: &str) -> ValidatingChainService {
        ValidatingChainService::new(Arc::new(FixedTxChainService {
            tx_hex: tx_hex.to_string(),
        }))
    }

    #[async_test_all]
    async fn test_passes_through_a_matching_tx() {
        let tx = test_tx();
        let tx_hex = serialize_hex(&tx);
        let service = validating_service(&tx_hex);

        let result = service
            .get_transaction_hex(tx.compute_txid().to_string())
            .await
            .unwrap();
        assert_eq!(result, tx_hex);
    }

    #[async_test_all]
    async fn test_rejects_a_substituted_tx() {
        let service = validating_service(&serialize_hex(&test_tx()));

        let other_txid = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let err = service
            .get_transaction_hex(other_txid.to_string())
            .await
            .unwrap_err();
        assert!(
            matches!(&err, ChainServiceError::Generic(m) if m.contains("integrity")),
            "unexpected error: {err:?}"
        );
    }

    #[async_test_all]
    async fn test_rejects_invalid_hex() {
        let service = validating_service("not-a-transaction");
        let err = service
            .get_transaction_hex(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, ChainServiceError::Generic(m) if m.contains("invalid transaction hex")),
            "unexpected error: {err:?}"
        );
    }
}
