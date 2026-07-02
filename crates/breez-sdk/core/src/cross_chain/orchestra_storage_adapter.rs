//! Orchestra rows in the provider-agnostic `cross_chain_swaps` table.
//!
//! Orchestra has no money-critical secrets to keep at rest, so the row's
//! `secrets` field stays empty. The row's `data` JSON is the SDK's
//! authoritative record of a prepared send quote: the send stage reads the
//! deposit address, amount, and source asset back from here rather than from
//! the caller-held prepared response, so a mutated or reused response can't
//! redirect funds or switch the asset.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::{
    Storage,
    error::SdkError,
    persist::{StorageError, StoredCrossChainSwap},
};

pub(crate) const PROVIDER_TAG_ORCHESTRA: &str = "orchestra";

/// SDK-authoritative record of a prepared Orchestra send quote, serialized
/// into [`StoredCrossChainSwap::data`].
///
/// `deposit_address`, `deposit_amount`, and `source_token_identifier` are the
/// money-path fields the send stage transfers against; the remaining fields
/// reconstruct `ConversionInfo::Orchestra` without re-fetching the quote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OrchestraSendData {
    /// Orchestra quote id. Equals [`StoredCrossChainSwap::id`].
    pub quote_id: String,
    /// Spark address Orchestra expects the deposit transfer to land on.
    pub deposit_address: String,
    /// Spark-side deposit amount in the source asset's base units.
    pub deposit_amount: u128,
    /// Source asset on the Spark side: `None` = BTC (sats), `Some(id)` = token.
    #[serde(default)]
    pub source_token_identifier: Option<String>,
    /// Quote expiry (RFC3339), checked before the deposit transfer.
    pub expires_at: String,
    /// Destination recipient + route, kept for `ConversionInfo` rendering.
    pub recipient_address: String,
    pub chain: String,
    #[serde(default)]
    pub chain_id: Option<String>,
    pub asset: String,
    pub asset_decimals: u32,
    #[serde(default)]
    pub asset_contract: Option<String>,
    pub estimated_out: u128,
    pub asset_amount_in: u128,
    pub fee_amount: u128,
    pub service_fee_amount: u128,
    #[serde(default)]
    pub service_fee_asset: Option<String>,
}

/// Thin bridge over the SDK [`Storage`] for Orchestra rows in
/// `cross_chain_swaps`. Orchestra keeps no at-rest secrets, so `secrets` is
/// always empty and no signer is involved (cf. `BoltzStorageAdapter`).
pub(crate) struct OrchestraStorageAdapter {
    storage: Arc<dyn Storage>,
}

impl OrchestraStorageAdapter {
    pub(crate) fn new(storage: Arc<dyn Storage>) -> Self {
        Self { storage }
    }

    /// Assembles a row for the given send-quote data. Pure: no I/O.
    fn to_stored(
        data: &OrchestraSendData,
        is_terminal: bool,
    ) -> Result<StoredCrossChainSwap, SdkError> {
        let serialized = serde_json::to_string(data)
            .map_err(|e| SdkError::Generic(format!("Failed to serialize Orchestra row: {e}")))?;
        Ok(StoredCrossChainSwap {
            provider: PROVIDER_TAG_ORCHESTRA.to_string(),
            id: data.quote_id.clone(),
            is_terminal,
            updated_at: current_unix_seconds(),
            data: serialized,
            secrets: String::new(),
        })
    }

    /// Persist (or overwrite) a prepared send quote as an active row.
    pub(crate) async fn upsert(&self, data: &OrchestraSendData) -> Result<(), SdkError> {
        let stored = Self::to_stored(data, false)?;
        self.storage
            .set_cross_chain_swap(stored)
            .await
            .map_err(|e| map_storage_err(&e))
    }

    /// Load a prepared send quote by its Orchestra quote id.
    pub(crate) async fn get(&self, quote_id: &str) -> Result<Option<OrchestraSendData>, SdkError> {
        let stored = self
            .storage
            .get_cross_chain_swap(PROVIDER_TAG_ORCHESTRA.to_string(), quote_id.to_string())
            .await
            .map_err(|e| map_storage_err(&e))?;
        match stored {
            Some(row) => {
                let data = serde_json::from_str(&row.data).map_err(|e| {
                    SdkError::Generic(format!("Failed to parse Orchestra row '{}': {e}", row.id))
                })?;
                Ok(Some(data))
            }
            None => Ok(None),
        }
    }

    /// Flip a row terminal: retained and queryable by id, but out of the
    /// active set. Idempotent; a missing row is a no-op.
    pub(crate) async fn mark_terminal(&self, quote_id: &str) -> Result<(), SdkError> {
        let stored = self
            .storage
            .get_cross_chain_swap(PROVIDER_TAG_ORCHESTRA.to_string(), quote_id.to_string())
            .await
            .map_err(|e| map_storage_err(&e))?;
        let Some(mut row) = stored else {
            return Ok(());
        };
        row.is_terminal = true;
        row.updated_at = current_unix_seconds();
        self.storage
            .set_cross_chain_swap(row)
            .await
            .map_err(|e| map_storage_err(&e))
    }
}

fn map_storage_err(e: &StorageError) -> SdkError {
    SdkError::StorageError(e.to_string())
}

fn current_unix_seconds() -> u64 {
    use platform_utils::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn sample_data() -> OrchestraSendData {
        OrchestraSendData {
            quote_id: "q_abc".to_string(),
            deposit_address: "spark1deposit".to_string(),
            deposit_amount: 1_020_434,
            source_token_identifier: None,
            expires_at: "2099-01-01T00:00:00Z".to_string(),
            recipient_address: "0xdead".to_string(),
            chain: "base".to_string(),
            chain_id: Some("8453".to_string()),
            asset: "USDC".to_string(),
            asset_decimals: 6,
            asset_contract: Some("0xUSDC".to_string()),
            estimated_out: 1_000_000,
            asset_amount_in: 1_020_434,
            fee_amount: 20_434,
            service_fee_amount: 500,
            service_fee_asset: Some("USDC".to_string()),
        }
    }

    /// `data` is serialized at prepare and deserialized untouched at send, so
    /// every field (incl. the `u128` amounts) must survive the round-trip.
    #[test]
    fn data_roundtrip_preserves_all_fields() {
        let data = sample_data();
        let json = serde_json::to_string(&data).unwrap();
        let decoded: OrchestraSendData = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, data);
    }

    /// A token source must round-trip as `Some(id)` (BTC is `None`).
    #[test]
    fn data_roundtrip_token_source() {
        let mut data = sample_data();
        data.source_token_identifier = Some("btkn1usdb".to_string());
        let json = serde_json::to_string(&data).unwrap();
        let decoded: OrchestraSendData = serde_json::from_str(&json).unwrap();
        assert_eq!(
            decoded.source_token_identifier,
            Some("btkn1usdb".to_string())
        );
    }

    /// Unknown fields from a future schema must not break deserialization.
    #[test]
    fn data_accepts_extra_unknown_fields() {
        let mut json: serde_json::Value = serde_json::to_value(sample_data()).unwrap();
        json["someFutureField"] = serde_json::json!("ignored");
        let decoded: OrchestraSendData = serde_json::from_value(json).unwrap();
        assert_eq!(decoded.quote_id, "q_abc");
    }
}

#[cfg(all(test, feature = "sqlite"))]
mod storage_tests {
    use std::path::PathBuf;

    use super::tests::sample_data;
    use super::*;
    use crate::persist::sqlite::SqliteStorage;

    fn temp_dir(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "breez-orchestra-storage-test-{}-{}",
            name,
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn make_adapter() -> (OrchestraStorageAdapter, Arc<dyn Storage>) {
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&temp_dir("adapter")).unwrap());
        (OrchestraStorageAdapter::new(Arc::clone(&storage)), storage)
    }

    #[tokio::test]
    async fn upsert_get_roundtrip_with_empty_secrets() {
        let (adapter, storage) = make_adapter();
        let data = sample_data();
        adapter.upsert(&data).await.unwrap();

        let fetched = adapter.get("q_abc").await.unwrap().unwrap();
        assert_eq!(fetched, data);

        // Orchestra keeps no at-rest secrets.
        let row = storage
            .get_cross_chain_swap(PROVIDER_TAG_ORCHESTRA.to_string(), "q_abc".to_string())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.provider, PROVIDER_TAG_ORCHESTRA);
        assert!(row.secrets.is_empty());
        assert!(!row.is_terminal);
    }

    #[tokio::test]
    async fn get_missing_row_is_none() {
        let (adapter, _storage) = make_adapter();
        assert!(adapter.get("nope").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn mark_terminal_drops_from_active_but_stays_queryable() {
        let (adapter, storage) = make_adapter();
        adapter.upsert(&sample_data()).await.unwrap();

        let active = storage
            .list_active_cross_chain_swaps(PROVIDER_TAG_ORCHESTRA.to_string())
            .await
            .unwrap();
        assert_eq!(active.len(), 1);

        adapter.mark_terminal("q_abc").await.unwrap();

        let active = storage
            .list_active_cross_chain_swaps(PROVIDER_TAG_ORCHESTRA.to_string())
            .await
            .unwrap();
        assert!(active.is_empty(), "terminal row must leave the active set");
        assert!(
            adapter.get("q_abc").await.unwrap().is_some(),
            "terminal row must stay queryable by id"
        );
    }

    #[tokio::test]
    async fn mark_terminal_missing_row_is_noop() {
        let (adapter, _storage) = make_adapter();
        adapter.mark_terminal("nope").await.unwrap();
    }
}
