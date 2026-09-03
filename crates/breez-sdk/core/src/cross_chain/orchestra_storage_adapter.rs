//! Orchestra rows in the provider-agnostic `cross_chain_swaps` table.
//!
//! Orchestra has no money-critical secrets to keep at rest, so `secrets` is
//! always empty. The row's `data` JSON carries the lookup key and the
//! quote-time values that don't appear on the live `Order` polled per tick.
//! Live status / `sparkTxHash` / `amountOut` / refund tx are read off the
//! `/status` response, never cached.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::{
    Storage,
    error::SdkError,
    persist::{StorageError, StoredCrossChainSwap},
};

pub(crate) const PROVIDER_TAG_ORCHESTRA: &str = "orchestra";

/// Persisted shape of an Orchestra row's `data`. The receive poller drives
/// the row through two states:
///
/// * **Pre-order** (`order_id` / `read_token` absent): probe `POST /submit`
///   with a fresh idempotency key each tick. Any error means "no deposit
///   yet". A 200 returns `{ orderId, readToken }` and the adapter writes
///   both. This is the only mid-flight mutation before the terminal flip.
/// * **Order in flight** (`order_id` / `read_token` set): poll
///   `GET /status?id={orderId}&readToken={token}` until terminal.
///
/// Live status / `sparkTxHash` / `amountOut` / refund tx are always read
/// off the poll response, never cached here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OrchestraSwapData {
    pub quote_id: String,
    /// Orchestra order id. Populated on the first `/submit` 200, then stable.
    #[serde(default)]
    pub order_id: Option<String>,
    /// `X-Read-Token` for `/status` calls. Born together with `order_id` on
    /// the same `/submit` 200.
    #[serde(default)]
    pub read_token: Option<String>,
    /// Wallet's Spark address (the receive destination).
    pub recipient_address: String,
    pub source_chain: String,
    pub source_asset: String,
    /// Source chain identifier (e.g. EVM `chainId` as a decimal string).
    /// `None` for non-EVM chains.
    #[serde(default)]
    pub source_chain_id: Option<String>,
    /// Source token contract address. `None` for native-asset routes.
    #[serde(default)]
    pub source_contract_address: Option<String>,
    /// Source asset decimals. Cached here because `ConversionInfo` rendering
    /// needs them and the live `Order` doesn't carry them.
    pub source_decimals: u32,
    pub destination_chain: String,
    pub destination_asset: String,
    /// Destination asset decimals: 8 for BTC sats, the token metadata
    /// decimals (e.g. 6 for USDB) for token destinations.
    #[serde(default = "default_destination_decimals")]
    pub destination_decimals: u32,
    /// Destination Spark token id (`Some("...")` for USDB) or `None` for BTC.
    #[serde(default)]
    pub token_identifier: Option<String>,
    /// Orchestra `amountIn` at quote time. The live `Order.amount_in` may
    /// differ if Orchestra repriced a late deposit, but the quote-time
    /// value is what gets surfaced in `ConversionInfo`.
    pub amount_in: String,
    /// Orchestra `estimatedOut` at quote time. The live `Order.amount_out` is
    /// what the receiver actually gets and is read off the poll response.
    pub expected_amount_out: String,
    #[serde(default)]
    pub fee_amount: Option<String>,
    /// Quote expiry, unix seconds. Not authoritative for the receive
    /// lifecycle: Orchestra may reprice late deposits.
    pub expires_at: u64,
}

fn default_destination_decimals() -> u32 {
    8
}

/// Thin wrapper over [`Storage`]'s `cross_chain_swaps` methods, keyed to
/// the Orchestra provider tag.
pub(crate) struct OrchestraStorageAdapter {
    storage: Arc<dyn Storage>,
}

impl OrchestraStorageAdapter {
    pub(crate) fn new(storage: Arc<dyn Storage>) -> Self {
        Self { storage }
    }

    /// Assembles a `StoredCrossChainSwap` row for the given Orchestra data.
    /// Pure (no I/O): Orchestra keeps no money-critical secrets, so
    /// `secrets` stays empty and no signer is involved.
    fn to_stored(data: &OrchestraSwapData) -> Result<StoredCrossChainSwap, SdkError> {
        let serialized = serde_json::to_string(data)
            .map_err(|e| SdkError::Generic(format!("Failed to serialize Orchestra row: {e}")))?;
        Ok(StoredCrossChainSwap {
            provider: PROVIDER_TAG_ORCHESTRA.to_string(),
            id: data.quote_id.clone(),
            is_terminal: false,
            updated_at: current_unix_seconds(),
            data: serialized,
            secrets: String::new(),
        })
    }

    /// Upsert an Orchestra row.
    pub(crate) async fn upsert(&self, data: &OrchestraSwapData) -> Result<(), SdkError> {
        let stored = Self::to_stored(data)?;
        self.storage
            .set_cross_chain_swap(stored)
            .await
            .map_err(|e| map_storage_err(&e))
    }

    /// Returns the active rows the receive poller should sweep. Corrupt
    /// rows are logged and skipped so one bad row doesn't stall the rest.
    pub(crate) async fn list_active(
        &self,
    ) -> Result<Vec<(StoredCrossChainSwap, OrchestraSwapData)>, SdkError> {
        let rows = self
            .storage
            .list_active_cross_chain_swaps(PROVIDER_TAG_ORCHESTRA.to_string())
            .await
            .map_err(|e| map_storage_err(&e))?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            match serde_json::from_str::<OrchestraSwapData>(&row.data) {
                Ok(data) => out.push((row, data)),
                Err(e) => {
                    tracing::warn!(
                        "Skipping Orchestra row '{}': failed to parse data: {e}",
                        row.id
                    );
                }
            }
        }
        Ok(out)
    }

    /// Pre-order to in-flight transition: persist `order_id` and
    /// `read_token` from the first successful `/submit`, returning the
    /// updated `(row, data)` so the caller can poll `/status` in the same
    /// tick. `data` is stable after this until `mark_terminal`.
    pub(crate) async fn attach_order_handle(
        &self,
        mut row: StoredCrossChainSwap,
        mut data: OrchestraSwapData,
        order_id: String,
        read_token: Option<String>,
    ) -> Result<(StoredCrossChainSwap, OrchestraSwapData), SdkError> {
        data.order_id = Some(order_id);
        data.read_token = read_token;
        row.data = serde_json::to_string(&data).map_err(|e| {
            SdkError::Generic(format!(
                "Failed to serialize Orchestra row after submit response: {e}"
            ))
        })?;
        row.updated_at = current_unix_seconds();
        self.storage
            .set_cross_chain_swap(row.clone())
            .await
            .map_err(|e| map_storage_err(&e))?;
        Ok((row, data))
    }

    /// Flip a row terminal.
    pub(crate) async fn mark_terminal(
        &self,
        mut row: StoredCrossChainSwap,
    ) -> Result<(), SdkError> {
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

    pub(super) fn sample_data() -> OrchestraSwapData {
        OrchestraSwapData {
            quote_id: "q_abc".to_string(),
            order_id: None,
            read_token: None,
            recipient_address: "sp1...".to_string(),
            source_chain: "base".to_string(),
            source_asset: "USDC".to_string(),
            source_chain_id: Some("8453".to_string()),
            source_contract_address: Some("0xUSDC".to_string()),
            source_decimals: 6,
            destination_chain: "spark".to_string(),
            destination_asset: "BTC".to_string(),
            destination_decimals: 8,
            token_identifier: None,
            amount_in: "100000000".to_string(),
            expected_amount_out: "100000".to_string(),
            fee_amount: Some("500".to_string()),
            expires_at: 1_700_000_120,
        }
    }

    pub(super) fn sample_data_usdb_destination() -> OrchestraSwapData {
        OrchestraSwapData {
            quote_id: "q_usdb".to_string(),
            order_id: Some("o_usdb".to_string()),
            read_token: Some("rt_usdb".to_string()),
            recipient_address: "sp1...".to_string(),
            source_chain: "arbitrum".to_string(),
            source_asset: "USDC".to_string(),
            source_chain_id: Some("42161".to_string()),
            source_contract_address: Some("0xUSDC".to_string()),
            source_decimals: 6,
            destination_chain: "spark".to_string(),
            destination_asset: "USDB".to_string(),
            destination_decimals: 6,
            token_identifier: Some(
                "btkn1xgrvjwey5ngcagvap2dzzvsy4uk8ua9x69k82dwvt5e7ef9drm9qztux87".to_string(),
            ),
            amount_in: "1050000".to_string(),
            expected_amount_out: "1000000".to_string(),
            fee_amount: Some("20000".to_string()),
            expires_at: 1_700_000_120,
        }
    }

    /// JSON round-trip preserves every field. Important because the row's
    /// `data` is serialised at `prepare_receive` time and the poller
    /// deserialises it untouched on every tick.
    #[test]
    fn data_roundtrip_preserves_all_fields() {
        let data = sample_data();
        let json = serde_json::to_string(&data).unwrap();
        let decoded: OrchestraSwapData = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, data);
    }

    /// Same round-trip guarantee for token-destination rows: `token_identifier`
    /// is `Some`, decimals match at 6, `order_id`/`read_token` populated.
    #[test]
    fn data_roundtrip_preserves_token_destination_fields() {
        let data = sample_data_usdb_destination();
        let json = serde_json::to_string(&data).unwrap();
        let decoded: OrchestraSwapData = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, data);
    }

    /// Forward compatibility: future fields added by the server must not
    /// break deserialization of older row schemas.
    #[test]
    fn data_accepts_extra_unknown_fields() {
        let mut json: serde_json::Value = serde_json::to_value(sample_data()).unwrap();
        json["someFutureField"] = serde_json::json!("ignored");
        let decoded: OrchestraSwapData = serde_json::from_value(json).unwrap();
        assert_eq!(decoded.quote_id, "q_abc");
    }
}

#[cfg(all(test, feature = "sqlite"))]
mod storage_tests {
    use std::path::PathBuf;

    use super::tests::*; // sample_data
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

    fn make_adapter() -> OrchestraStorageAdapter {
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&temp_dir("adapter")).unwrap());
        OrchestraStorageAdapter::new(storage)
    }

    fn stored_row_for(data: &OrchestraSwapData) -> StoredCrossChainSwap {
        StoredCrossChainSwap {
            provider: PROVIDER_TAG_ORCHESTRA.to_string(),
            id: data.quote_id.clone(),
            is_terminal: false,
            updated_at: 0,
            data: serde_json::to_string(data).unwrap(),
            secrets: String::new(),
        }
    }

    /// Mid-flight write: populates `order_id` + `read_token`, preserves
    /// the rest of `data`, keeps the row non-terminal.
    #[tokio::test]
    async fn attach_order_handle_persists_handle_and_keeps_row_active() {
        let adapter = make_adapter();
        let initial_data = sample_data();
        adapter
            .storage
            .set_cross_chain_swap(stored_row_for(&initial_data))
            .await
            .unwrap();

        let row = adapter
            .storage
            .get_cross_chain_swap(
                PROVIDER_TAG_ORCHESTRA.to_string(),
                initial_data.quote_id.clone(),
            )
            .await
            .unwrap()
            .expect("seeded row");

        adapter
            .attach_order_handle(
                row,
                initial_data.clone(),
                "ord_xyz".to_string(),
                Some("rt_xyz".to_string()),
            )
            .await
            .unwrap();

        let after_row = adapter
            .storage
            .get_cross_chain_swap(
                PROVIDER_TAG_ORCHESTRA.to_string(),
                initial_data.quote_id.clone(),
            )
            .await
            .unwrap()
            .expect("row still present");
        assert!(!after_row.is_terminal, "still active until terminal poll");

        let after: OrchestraSwapData = serde_json::from_str(&after_row.data).unwrap();
        assert_eq!(after.order_id.as_deref(), Some("ord_xyz"));
        assert_eq!(after.read_token.as_deref(), Some("rt_xyz"));
        // Every other field stays exactly as written at quote time.
        assert_eq!(after.quote_id, initial_data.quote_id);
        assert_eq!(after.recipient_address, initial_data.recipient_address);
        assert_eq!(after.amount_in, initial_data.amount_in);
        assert_eq!(after.expected_amount_out, initial_data.expected_amount_out);
        assert_eq!(after.fee_amount, initial_data.fee_amount);
    }

    /// `list_active` returns non-terminal rows only.
    #[tokio::test]
    async fn list_active_returns_only_non_terminal_rows() {
        let adapter = make_adapter();
        let row_data = sample_data();
        adapter
            .storage
            .set_cross_chain_swap(stored_row_for(&row_data))
            .await
            .unwrap();

        let active = adapter.list_active().await.unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].1.quote_id, "q_abc");
    }

    /// `mark_terminal` flips `is_terminal` and drops the row from the next
    /// `list_active`.
    #[tokio::test]
    async fn mark_terminal_flips_flag_and_drops_from_active_list() {
        let adapter = make_adapter();
        let data = sample_data();
        adapter
            .storage
            .set_cross_chain_swap(stored_row_for(&data))
            .await
            .unwrap();
        let row = adapter
            .storage
            .get_cross_chain_swap(PROVIDER_TAG_ORCHESTRA.to_string(), data.quote_id.clone())
            .await
            .unwrap()
            .unwrap();
        adapter.mark_terminal(row).await.unwrap();

        let after = adapter
            .storage
            .get_cross_chain_swap(PROVIDER_TAG_ORCHESTRA.to_string(), data.quote_id.clone())
            .await
            .unwrap()
            .unwrap();
        assert!(after.is_terminal);

        let active = adapter.list_active().await.unwrap();
        assert!(active.is_empty());
    }
}
