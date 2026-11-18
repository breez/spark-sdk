//! Orchestra rows in the provider-agnostic `cross_chain_swaps` table.
//!
//! Orchestra has no money-critical secrets to keep at rest, so `secrets` is
//! always empty. The row's `data` JSON carries the lookup key and the
//! quote-time values that don't appear on the live `Order` polled per tick.
//! Live status / `sparkTxHash` / `amountOut` are read off the `/status`
//! response. Only a failed or refunded receive records its outcome, on the
//! closed row, which syncs so every device can show the same failed payment.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::{
    ConversionStatus, PaymentTokenMetadata, Storage,
    error::SdkError,
    persist::{StorageError, StoredCrossChainSwap},
    utils::time::now_secs,
};

pub(crate) const PROVIDER_TAG_ORCHESTRA: &str = "orchestra";

/// Persisted shape of an Orchestra row's `data`. The receive poller drives
/// the row through two states:
///
/// * **Pre-order** (`order_id` absent): look the quote up on
///   `GET /status?quoteId=` with `quote_read_token`. Orchestra picks the
///   deposit up on its own, and an order in the response gives `order_id`. A
///   row without a usable quote token probes `POST /submit` instead, whose
///   200 returns `{ orderId, readToken }` for the adapter to write.
/// * **Order in flight** (`order_id` set): poll `/status` until terminal, by
///   order id with `read_token` when `/submit` issued one, otherwise by quote
///   id with `quote_read_token`. `/submit` is idempotent, so a rejected token
///   is replaced by probing it.
/// * **Lightning-funded** (`source_chain` is `lightning`): `order_id` is set
///   from creation, before anyone pays. The order is polled only once `paid`
///   records its payment, and an unpaid one is closed locally after expiry.
///
/// Live status / `sparkTxHash` / `amountOut` are read off the poll response.
/// Only the outcome of a failed or refunded order is kept, in `failure`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OrchestraSwapData {
    pub quote_id: String,
    /// Orchestra order id. Populated once `/status?quoteId=` or `/submit`
    /// returns the order, then stable.
    #[serde(default)]
    pub order_id: Option<String>,
    /// `X-Read-Token` for `/status?id=`, issued by `/submit` and bound to
    /// `order_id`.
    #[serde(default)]
    pub read_token: Option<String>,
    /// `X-Read-Token` for `/status?quoteId=`, issued by `/quote` and bound to
    /// `quote_id` for 24h from quote creation. Absent on rows written before
    /// quote tokens were stored.
    #[serde(default)]
    pub quote_read_token: Option<String>,
    /// Wallet's Spark address (the receive destination).
    pub recipient_address: String,
    /// Amountless Spark invoice the provider fulfils on delivery, which ties
    /// the inbound payment to this row. Absent on rows written before
    /// invoice delivery was supported: those link up through the order's
    /// `sparkTxHash` alone.
    #[serde(default)]
    pub spark_invoice: Option<String>,
    /// Whether a payment fulfilling `spark_invoice` has come in. Recorded on
    /// Lightning-funded rows only.
    #[serde(default)]
    pub paid: bool,
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
    /// Quote service fee, in `fee_asset` units.
    #[serde(default)]
    pub fee_amount: Option<String>,
    /// Quote `feeAsset` ticker as Orchestra reports it (`"BTC"` for sats).
    /// Unset on rows written before it was stored.
    #[serde(default)]
    pub fee_asset: Option<String>,
    /// Decimals of `fee_asset`. Unset when Orchestra did not report them.
    #[serde(default)]
    pub fee_asset_decimals: Option<u32>,
    /// Quote expiry, unix seconds. Not authoritative for the receive
    /// lifecycle: Orchestra may reprice late deposits.
    pub expires_at: u64,
    /// How the order ended, when Orchestra failed or refunded it. Written as
    /// the row closes.
    #[serde(default)]
    pub failure: Option<ReceiveFailure>,
}

fn default_destination_decimals() -> u32 {
    8
}

/// The outcome of a receive whose order Orchestra failed or refunded: what a
/// device needs to record the failed payment without calling Orchestra or
/// Spark.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReceiveFailure {
    /// `Refunded` or `Failed`.
    pub status: ConversionStatus,
    /// The deposit Orchestra received, in source-asset base units.
    #[serde(default)]
    pub deposit_amount: Option<String>,
    /// The deposit transaction on the source chain.
    #[serde(default)]
    pub deposit_tx_hash: Option<String>,
    /// The transaction that returned the deposit to the sender.
    #[serde(default)]
    pub refund_tx_hash: Option<String>,
    /// When Orchestra created the order, unix seconds.
    pub created_at: u64,
    /// Metadata of `token_identifier`, set on token destinations.
    #[serde(default)]
    pub token_metadata: Option<PaymentTokenMetadata>,
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
            updated_at: now_secs(),
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
    /// `read_token` once Orchestra reports the order, returning the updated
    /// `(row, data)` so the caller can poll `/status` in the same tick.
    /// `data` is stable after this until `mark_terminal`.
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
                "Failed to serialize Orchestra row with its order handle: {e}"
            ))
        })?;
        row.updated_at = now_secs();
        self.storage
            .set_cross_chain_swap(row.clone())
            .await
            .map_err(|e| map_storage_err(&e))?;
        Ok((row, data))
    }

    /// Closes a row, writing `data` in the same update so the closed row
    /// syncs with it.
    pub(crate) async fn close_with(
        &self,
        mut row: StoredCrossChainSwap,
        data: &OrchestraSwapData,
    ) -> Result<(), SdkError> {
        row.data = serde_json::to_string(data)
            .map_err(|e| SdkError::Generic(format!("Failed to serialize Orchestra row: {e}")))?;
        self.mark_terminal(row).await
    }

    /// Flip a row terminal.
    pub(crate) async fn mark_terminal(
        &self,
        mut row: StoredCrossChainSwap,
    ) -> Result<(), SdkError> {
        row.is_terminal = true;
        row.updated_at = now_secs();
        self.storage
            .set_cross_chain_swap(row)
            .await
            .map_err(|e| map_storage_err(&e))
    }
}

fn map_storage_err(e: &StorageError) -> SdkError {
    SdkError::StorageError(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn sample_data() -> OrchestraSwapData {
        OrchestraSwapData {
            quote_id: "q_abc".to_string(),
            order_id: None,
            read_token: None,
            quote_read_token: None,
            recipient_address: "sp1...".to_string(),
            spark_invoice: Some("spark1inv...".to_string()),
            paid: false,
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
            fee_asset: Some("USDC".to_string()),
            fee_asset_decimals: Some(6),
            expires_at: 1_700_000_120,
            failure: None,
        }
    }

    pub(super) fn sample_data_usdb_destination() -> OrchestraSwapData {
        OrchestraSwapData {
            quote_id: "q_usdb".to_string(),
            order_id: Some("o_usdb".to_string()),
            read_token: Some("rt_usdb".to_string()),
            quote_read_token: None,
            recipient_address: "sp1...".to_string(),
            spark_invoice: None,
            paid: false,
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
            fee_asset: None,
            fee_asset_decimals: None,
            expires_at: 1_700_000_120,
            failure: None,
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

    /// Rows written before the fee asset was stored still decode, with it
    /// unset.
    #[test]
    fn data_written_without_the_fee_asset_decodes() {
        let mut json: serde_json::Value = serde_json::to_value(sample_data()).unwrap();
        let obj = json.as_object_mut().unwrap();
        obj.remove("feeAsset");
        obj.remove("feeAssetDecimals");
        let decoded: OrchestraSwapData = serde_json::from_value(json).unwrap();
        assert_eq!(decoded.fee_asset, None);
        assert_eq!(decoded.fee_asset_decimals, None);
    }

    /// Rows written before quote tokens were stored decode without one.
    #[test]
    fn data_written_without_a_quote_read_token_decodes() {
        let mut data = sample_data();
        data.quote_read_token = Some("qrt_abc".to_string());
        let json = serde_json::to_string(&data).unwrap();
        assert_eq!(
            serde_json::from_str::<OrchestraSwapData>(&json).unwrap(),
            data
        );

        let mut json: serde_json::Value = serde_json::to_value(&data).unwrap();
        json.as_object_mut().unwrap().remove("quoteReadToken");
        let decoded: OrchestraSwapData = serde_json::from_value(json).unwrap();
        assert_eq!(decoded.quote_read_token, None);
    }

    pub(super) fn sample_failure() -> ReceiveFailure {
        ReceiveFailure {
            status: ConversionStatus::Refunded,
            deposit_amount: Some("98".to_string()),
            deposit_tx_hash: Some("0xdeposit".to_string()),
            refund_tx_hash: Some("0xrefund".to_string()),
            created_at: 1_700_000_000,
            token_metadata: Some(PaymentTokenMetadata {
                identifier: "btkn1usdb".to_string(),
                issuer_public_key: "02issuer".to_string(),
                name: "USDB".to_string(),
                ticker: "USDB".to_string(),
                decimals: 6,
                max_supply: u128::MAX,
                is_freezable: false,
            }),
        }
    }

    /// A closed row's failure, token metadata included, survives the trip
    /// through the row's JSON.
    #[test]
    fn data_roundtrip_preserves_the_failure() {
        let data = OrchestraSwapData {
            failure: Some(sample_failure()),
            ..sample_data_usdb_destination()
        };
        let json = serde_json::to_string(&data).unwrap();
        let decoded: OrchestraSwapData = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, data);
    }

    /// Rows written before failures were recorded decode without one.
    #[test]
    fn data_written_without_a_failure_decodes() {
        let mut json: serde_json::Value = serde_json::to_value(sample_data()).unwrap();
        json.as_object_mut().unwrap().remove("failure");
        let decoded: OrchestraSwapData = serde_json::from_value(json).unwrap();
        assert_eq!(decoded.failure, None);
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

    /// `close_with` writes the data and closes the row in one update.
    #[tokio::test]
    async fn close_with_writes_the_data_and_closes_the_row() {
        let adapter = make_adapter();
        let mut data = sample_data();
        adapter.upsert(&data).await.unwrap();
        let (row, _) = adapter.list_active().await.unwrap().remove(0);

        data.failure = Some(sample_failure());
        adapter.close_with(row, &data).await.unwrap();

        let after = adapter
            .storage
            .get_cross_chain_swap(PROVIDER_TAG_ORCHESTRA.to_string(), data.quote_id.clone())
            .await
            .unwrap()
            .unwrap();
        assert!(after.is_terminal);
        let stored: OrchestraSwapData = serde_json::from_str(&after.data).unwrap();
        assert_eq!(stored.failure, Some(sample_failure()));
        assert!(adapter.list_active().await.unwrap().is_empty());
    }
}
