//! Orchestra rows in the provider-agnostic `cross_chain_swaps` table.
//!
//! Orchestra has no money-critical secrets to keep at rest, so the row's
//! `secrets` field stays empty. The row's `data` JSON carries the lookup
//! key and the quote-time values that don't appear on the live `Order`
//! polled per tick — live status / `sparkTxHash` / `amountOut` / refund tx
//! are read off the `/status` response. The receive poller writes exactly
//! two mutations to a row: the pre-order → in-flight transition (when
//! `/submit` first returns an order handle), and the terminal flip on
//! close-out.
//!
//! "Adapter" is a misnomer of convenience — there's no external trait
//! being implemented here (cf. `boltz_storage_adapter.rs` which adapts the
//! SDK's [`Storage`] to `boltz_client::BoltzStorage`). The file name
//! follows that precedent for discoverability.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::{
    Storage,
    error::SdkError,
    persist::{StorageError, StoredCrossChainSwap},
};

pub(crate) const PROVIDER_TAG_ORCHESTRA: &str = "orchestra";

/// Persisted shape of an Orchestra row's `data`.
///
/// The receive poller drives the row through two states:
///
/// * **Pre-order** (`order_id` / `read_token` are absent): the poller calls
///   `POST /submit` with the quote id and a fresh idempotency key on every
///   tick. Until Orchestra detects the deposit the call returns 400 (any
///   error is treated as "not yet"); once it does the call returns
///   `{ orderId, readToken }` and the adapter mutates the row's `data` to
///   populate both. This is the **only** mid-flight mutation written
///   before the terminal flip.
/// * **Order in flight** (`order_id` and `read_token` are set): the poller
///   switches to `GET /status?id={orderId}&readToken={token}` until the
///   order reaches a terminal state.
///
/// Live status / `sparkTxHash` / `amountOut` / refund tx are always read off
/// the live `Order` returned on each poll — never cached.
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
    /// Wallet's Spark address. Also lifted into the prepared response and
    /// into `ConversionInfo::Orchestra.recipient_address`.
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
    /// Source asset decimals — needed for `ConversionInfo` rendering and not
    /// available on the live `Order`.
    pub source_decimals: u32,
    pub destination_chain: String,
    pub destination_asset: String,
    /// Destination asset decimals — used for fee math on receive. `8` for BTC
    /// sats; the token's metadata decimals (e.g. 6 for USDB) for token
    /// destinations. Defaults to `8` for rows persisted before this field
    /// existed (BTC was the only destination until tokens shipped).
    #[serde(default = "default_destination_decimals")]
    pub destination_decimals: u32,
    /// Destination Spark token id (`Some("...")` for USDB) or `None` for BTC.
    #[serde(default)]
    pub token_identifier: Option<String>,
    /// Orchestra `amountIn` at quote time. The live `Order.amount_in` may
    /// differ if Orchestra repriced a late deposit; the quote-time value is
    /// what gets surfaced in `ConversionInfo`.
    pub amount_in: String,
    /// Orchestra `estimatedOut` at quote time. The live `Order.amount_out` is
    /// what the receiver actually gets and is read off the poll response.
    pub expected_amount_out: String,
    #[serde(default)]
    pub fee_amount: Option<String>,
    /// Quote expiry, surfaced for UI countdown. The receive poller does not
    /// gate on expiry: Orchestra reprices late deposits, so the SDK keeps
    /// polling until Orchestra reports terminal.
    pub expires_at: u64,
}

fn default_destination_decimals() -> u32 {
    8
}

/// Bridges the SDK's [`Storage`] for Orchestra rows in the
/// `cross_chain_swaps` table. Thin wrapper around `set_cross_chain_swap` /
/// `get_cross_chain_swap` / `list_active_cross_chain_swaps` so the cross-chain
/// flow doesn't have to know about the storage trait shape or the
/// provider-keying convention.
pub(crate) struct OrchestraStorageAdapter {
    storage: Arc<dyn Storage>,
}

impl OrchestraStorageAdapter {
    pub(crate) fn new(storage: Arc<dyn Storage>) -> Self {
        Self { storage }
    }

    /// Assembles a `StoredCrossChainSwap` row for the given Orchestra row
    /// data. Counterpart of `BoltzStorageAdapter::to_stored`. Pure: no
    /// async/storage I/O — Orchestra has no money-critical secrets, so the
    /// `secrets` field stays empty and no signer is involved.
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

    /// Returns the active rows the receive poller should sweep. A corrupt
    /// row is logged and skipped rather than stalling convergence for the
    /// rest — same defence-in-depth as the Boltz adapter.
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

    /// Pre-order → in-flight transition: persist the `order_id` and
    /// `read_token` Orchestra hands back from the first successful
    /// `/submit`. Returns the updated `(row, data)` so the caller can
    /// poll `/status` immediately in the same tick rather than waiting
    /// for the next monitor iteration. After this the row's `data` is
    /// stable until `mark_terminal`.
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

    /// Flip a row terminal. The only mutation the poller writes after
    /// `attach_order_handle`.
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

    /// Mid-flight write: populates `order_id` + `read_token` from a
    /// successful `/submit`, preserves the rest of `data`, and keeps the
    /// row non-terminal. Any change to that contract would silently break
    /// the receive poller's state transition.
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

    /// `list_active` skips terminal rows (the storage layer already
    /// filters those via `list_active_cross_chain_swaps`, but the test
    /// pins the contract from end to end).
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

    /// `mark_terminal` flips `is_terminal` and the row drops out of the
    /// next `list_active`. Both halves matter: a row that stays active
    /// after close-out would be polled forever.
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
