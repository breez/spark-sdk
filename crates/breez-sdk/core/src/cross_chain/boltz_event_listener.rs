//! Forwards [`boltz_client::BoltzSwapEvent`]s into SDK payment metadata.
//!
//! Boltz drives swap-state changes via its own WebSocket; the listener
//! translates those into silent payment-metadata updates. The user-facing
//! signal for the lightning leg is the `PaymentSucceeded` / `PaymentFailed`
//! event already emitted by `spark_wallet.pay_lightning_invoice`, so the
//! listener intentionally does not emit any SDK event of its own — matching
//! Orchestra's current behavior.
//!
//! Follow-up: decide whether any cross-chain provider should emit a
//! distinct terminal event when the destination-chain claim lands. The
//! gap is less pressing for Boltz (UIs react to the LN-leg event) than
//! for Orchestra, but both are silent today.

use std::sync::Arc;

use boltz_client::{
    BoltzEventListener, BoltzService, BoltzSwapEvent, events,
    models::{BoltzSwap, BoltzSwapStatus},
};
use tracing::{debug, error, info, warn};

use crate::{
    ConversionInfo, ConversionStatus, PaymentMetadata, Storage,
    persist::{ConversionFilter, StorageListPaymentsRequest, StoragePaymentDetailsFilter},
    utils::conversions::extract_conversion_info,
};

pub(crate) struct BoltzSdkEventListener {
    storage: Arc<dyn Storage>,
}

impl BoltzSdkEventListener {
    pub(crate) fn new(storage: Arc<dyn Storage>) -> Self {
        Self { storage }
    }

    async fn handle_swap_updated(
        &self,
        swap: &boltz_client::models::BoltzSwap,
    ) -> Result<(), String> {
        let Some(existing) = self
            .storage
            .get_payment_by_invoice(swap.invoice.clone())
            .await
            .map_err(|e| format!("fetch payment by invoice for swap {}: {e}", swap.id))?
        else {
            // Prepare-without-send orphan, or send still in flight: no payment
            // row yet carries this hold invoice. Boltz-client updates its own
            // swap KV via `BoltzStorage::upsert_swap` independently, and the
            // next WS transition will re-sync the payment metadata once the
            // row exists.
            debug!(
                swap_id = %swap.id,
                "No payment row for Boltz swap invoice, skipping payment-row update"
            );
            return Ok(());
        };

        let payment_id = existing.id.clone();

        let Some(conversion_info) = extract_conversion_info(existing.details) else {
            // Race window between `insert_payment` and `insert_payment_metadata`
            // in the send flow, or a non-conversion payment sharing this
            // invoice. A later swap event carries the full state and retries;
            // a dropped terminal event is recovered by the send-time
            // read-after-write in `BoltzService::send` and the startup
            // `reconcile_pending_boltz_conversions` pass.
            debug!(
                swap_id = %swap.id,
                payment_id = %payment_id,
                "Payment has no ConversionInfo attached, skipping"
            );
            return Ok(());
        };

        let Some(updated) = boltz_metadata_from_swap(conversion_info, swap) else {
            debug!(
                swap_id = %swap.id,
                payment_id = %payment_id,
                "Payment has non-Boltz ConversionInfo, skipping"
            );
            return Ok(());
        };

        self.storage
            .insert_payment_metadata(payment_id.clone(), updated)
            .await
            .map_err(|e| format!("persist updated metadata for {payment_id}: {e}"))?;
        Ok(())
    }

    async fn handle_quote_degraded(
        &self,
        swap: &boltz_client::models::BoltzSwap,
    ) -> Result<(), String> {
        let Some(existing) = self
            .storage
            .get_payment_by_invoice(swap.invoice.clone())
            .await
            .map_err(|e| format!("fetch payment by invoice for swap {}: {e}", swap.id))?
        else {
            debug!(
                swap_id = %swap.id,
                "No payment row for Boltz swap invoice, skipping quote-degraded update"
            );
            return Ok(());
        };

        let payment_id = existing.id.clone();

        let Some(ConversionInfo::Boltz {
            swap_id,
            chain,
            chain_id,
            asset,
            recipient_address,
            invoice,
            invoice_amount_sats,
            asset_amount_in,
            estimated_out,
            delivered_amount,
            bridge_ref,
            status,
            fee_amount,
            service_fee_amount,
            service_fee_asset,
            max_slippage_bps,
            asset_decimals,
            asset_contract,
            ..
        }) = extract_conversion_info(existing.details)
        else {
            debug!(
                swap_id = %swap.id,
                payment_id = %payment_id,
                "Payment has no Boltz ConversionInfo, skipping quote-degraded update"
            );
            return Ok(());
        };

        let updated = PaymentMetadata {
            conversion_info: Some(ConversionInfo::Boltz {
                swap_id,
                chain,
                chain_id,
                asset,
                recipient_address,
                invoice,
                invoice_amount_sats,
                asset_amount_in,
                estimated_out,
                delivered_amount,
                bridge_ref,
                status,
                fee_amount,
                service_fee_amount,
                service_fee_asset,
                max_slippage_bps,
                quote_degraded: true,
                asset_decimals,
                asset_contract,
            }),
            ..Default::default()
        };
        self.storage
            .insert_payment_metadata(payment_id, updated)
            .await
            .map_err(|e| format!("persist degraded-flag update: {e}"))?;
        Ok(())
    }
}

#[macros::async_trait]
impl BoltzEventListener for BoltzSdkEventListener {
    async fn on_event(&self, event: BoltzSwapEvent) {
        match &event {
            events::BoltzSwapEvent::SwapUpdated { swap } => {
                if let Err(e) = self.handle_swap_updated(swap).await {
                    error!(swap_id = %swap.id, "Boltz SwapUpdated handling failed: {e}");
                }
            }
            events::BoltzSwapEvent::QuoteDegraded { swap, .. } => {
                if let Err(e) = self.handle_quote_degraded(swap).await {
                    error!(swap_id = %swap.id, "Boltz QuoteDegraded handling failed: {e}");
                }
            }
        }
    }
}

fn map_boltz_status_to_conversion(status: &BoltzSwapStatus) -> ConversionStatus {
    match status {
        BoltzSwapStatus::Created
        | BoltzSwapStatus::InvoicePaid
        | BoltzSwapStatus::TbtcLocked
        | BoltzSwapStatus::Claiming
        | BoltzSwapStatus::Settling => ConversionStatus::Pending,
        BoltzSwapStatus::Completed => ConversionStatus::Completed,
        BoltzSwapStatus::Failed { .. } | BoltzSwapStatus::Expired => ConversionStatus::Failed,
    }
}

/// Mirror the current `swap` state onto `existing`, producing the
/// [`PaymentMetadata`] update to persist. Returns `None` if `existing` is not a
/// Boltz conversion (the caller should skip it). Immutable prepare-time fields
/// pass through unchanged; only `status`, `delivered_amount`, and `bridge_ref`
/// are refreshed from the swap row.
///
/// Shared by the WS-driven [`BoltzSdkEventListener::handle_swap_updated`], the
/// send-time read-after-write in `BoltzService::send`, and
/// [`reconcile_pending_boltz_conversions`], so all three apply identical
/// status mapping.
pub(crate) fn boltz_metadata_from_swap(
    existing: ConversionInfo,
    swap: &BoltzSwap,
) -> Option<PaymentMetadata> {
    let ConversionInfo::Boltz {
        swap_id,
        chain,
        chain_id,
        asset,
        recipient_address,
        invoice,
        invoice_amount_sats,
        asset_amount_in,
        estimated_out,
        fee_amount,
        service_fee_amount,
        service_fee_asset,
        max_slippage_bps,
        quote_degraded,
        asset_decimals,
        asset_contract,
        ..
    } = existing
    else {
        return None;
    };

    let new_status = map_boltz_status_to_conversion(&swap.status);
    let delivered_amount = swap.delivered_amount.map(u128::from);
    let updated_fee_amount = super::compute_terminal_fee_amount(
        &new_status,
        asset_amount_in,
        delivered_amount,
        fee_amount,
    );

    Some(PaymentMetadata {
        conversion_info: Some(ConversionInfo::Boltz {
            swap_id,
            chain,
            chain_id,
            asset,
            recipient_address,
            invoice,
            invoice_amount_sats,
            asset_amount_in,
            estimated_out,
            delivered_amount,
            bridge_ref: swap.bridge_ref.clone(),
            status: new_status,
            fee_amount: updated_fee_amount,
            service_fee_amount,
            service_fee_asset,
            max_slippage_bps,
            quote_degraded,
            asset_decimals,
            asset_contract,
        }),
        ..Default::default()
    })
}

/// Startup safety net for Boltz conversions whose terminal `SwapUpdated` event
/// was never applied to the payment row.
///
/// The WS event copies the terminal swap state onto the payment metadata, but
/// it is fire-once and conditional on the payment row + `ConversionInfo`
/// already existing. A fast swap can reach terminal before the send flow
/// attaches `ConversionInfo` (the event is then dropped with no later event to
/// retry), and a storage error or WS gap can likewise drop it. In all those
/// cases the boltz-client swap row is terminal while `conversion_info.status`
/// stays `Pending` forever.
///
/// This pass scans Send payments for Boltz conversions still `Pending`, and for
/// each whose retained swap row (`get_swap`, a local read) has reached a
/// terminal state, applies that state via [`boltz_metadata_from_swap`]. It runs
/// once after `resume_swaps`. Only the instance that owns the swap row can
/// resolve it; other devices pick up the corrected metadata through sync.
pub(crate) async fn reconcile_pending_boltz_conversions(
    client: &BoltzService,
    storage: &Arc<dyn Storage>,
) {
    // Bound the scan to Lightning payments carrying a non-terminal Boltz
    // conversion (the swap's hold-invoice leg), so history size doesn't matter.
    let payments = match storage
        .list_payments(StorageListPaymentsRequest {
            payment_details_filter: Some(vec![StoragePaymentDetailsFilter::Lightning {
                htlc_status: None,
                conversion_filter: Some(ConversionFilter::BoltzPending),
            }]),
            ..Default::default()
        })
        .await
    {
        Ok(payments) => payments,
        Err(e) => {
            warn!("Boltz reconcile: failed to list pending conversions: {e}");
            return;
        }
    };

    for payment in payments {
        let payment_id = payment.id.clone();
        let Some(conversion_info) = extract_conversion_info(payment.details) else {
            continue;
        };
        let ConversionInfo::Boltz {
            swap_id, status, ..
        } = &conversion_info
        else {
            continue;
        };
        if !matches!(status, ConversionStatus::Pending) {
            continue;
        }
        let swap_id = swap_id.clone();

        let swap = match client.get_swap(&swap_id).await {
            Ok(Some(swap)) if swap.status.is_terminal() => swap,
            // Absent (not owned by this instance) or still in flight: leave it
            // for the WS path / a later run.
            Ok(_) => continue,
            Err(e) => {
                debug!("Boltz reconcile: get_swap {swap_id} failed: {e}");
                continue;
            }
        };

        let Some(updated) = boltz_metadata_from_swap(conversion_info, &swap) else {
            continue;
        };
        match storage
            .insert_payment_metadata(payment_id.clone(), updated)
            .await
        {
            Ok(()) => info!(
                payment_id = %payment_id,
                swap_id = %swap_id,
                status = ?swap.status,
                "Boltz reconcile: applied terminal swap state to stale pending conversion"
            ),
            Err(e) => error!(
                payment_id = %payment_id,
                swap_id = %swap_id,
                "Boltz reconcile: failed to persist reconciled metadata: {e}"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use boltz_client::models::{Asset, BoltzSwap, BoltzSwapStatus, BridgeKind};

    use super::*;
    use macros::test_all;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    fn make_swap_min(id: &str, status: BoltzSwapStatus) -> BoltzSwap {
        BoltzSwap {
            id: id.to_string(),
            status,
            bridge_kind: BridgeKind::Direct,
            claim_key_index: 0,
            chain_id: 42161,
            claim_address: "0xclaim".to_string(),
            destination_address: "0xdest".to_string(),
            destination_chain: "Arbitrum One".to_string(),
            asset: Asset::Usdt,
            refund_address: "0xrefund".to_string(),
            erc20swap_address: "0xswap".to_string(),
            router_address: "0xrouter".to_string(),
            invoice: "lnbc1".to_string(),
            invoice_amount_sats: 1_013,
            onchain_amount: 1_000,
            expected_output_amount: 657_084,
            slippage_bps: 100,
            timeout_block_height: 123_456,
            lockup_tx_id: None,
            claim_tx_hash: None,
            pending_call_id: None,
            delivered_amount: None,
            bridge_ref: None,
            created_at: 1_700_000_000,
            updated_at: 1_700_000_000,
        }
    }

    fn pending_boltz_conversion() -> ConversionInfo {
        ConversionInfo::Boltz {
            swap_id: "swap1".to_string(),
            invoice: "lnbc1".to_string(),
            invoice_amount_sats: 1_013,
            bridge_ref: None,
            max_slippage_bps: 100,
            quote_degraded: false,
            chain: "Arbitrum One".to_string(),
            chain_id: Some("42161".to_string()),
            asset: "USDT".to_string(),
            recipient_address: "0xrecipient".to_string(),
            asset_amount_in: Some(664_652),
            estimated_out: 656_122,
            delivered_amount: None,
            status: ConversionStatus::Pending,
            fee_amount: Some(8_530),
            service_fee_amount: Some(8_530),
            service_fee_asset: Some("USDT".to_string()),
            asset_decimals: 6,
            asset_contract: Some("0xUSDT".to_string()),
        }
    }

    #[test_all]
    fn boltz_metadata_from_swap_applies_terminal_and_preserves_prepare_fields() {
        let mut swap = make_swap_min("swap1", BoltzSwapStatus::Completed);
        swap.delivered_amount = Some(657_084);

        let updated = boltz_metadata_from_swap(pending_boltz_conversion(), &swap)
            .expect("Boltz variant yields an update");

        let Some(ConversionInfo::Boltz {
            status,
            delivered_amount,
            estimated_out,
            max_slippage_bps,
            fee_amount,
            recipient_address,
            ..
        }) = updated.conversion_info
        else {
            panic!("expected a Boltz conversion");
        };
        assert_eq!(status, ConversionStatus::Completed);
        assert_eq!(delivered_amount, Some(657_084));
        // Immutable prepare-time fields pass through unchanged.
        assert_eq!(estimated_out, 656_122);
        assert_eq!(max_slippage_bps, 100);
        // fee_amount is recomputed on terminal: asset_amount_in - delivered = 664_652 - 657_084.
        assert_eq!(fee_amount, Some(7_568));
        assert_eq!(recipient_address, "0xrecipient");
    }

    #[test_all]
    fn boltz_metadata_from_swap_maps_failed() {
        let swap = make_swap_min("swap1", BoltzSwapStatus::Expired);
        let updated =
            boltz_metadata_from_swap(pending_boltz_conversion(), &swap).expect("Boltz variant");
        assert!(matches!(
            updated.conversion_info,
            Some(ConversionInfo::Boltz {
                status: ConversionStatus::Failed,
                ..
            })
        ));
    }

    #[test_all]
    fn boltz_metadata_from_swap_returns_none_for_non_boltz() {
        let amm = ConversionInfo::Amm {
            pool_id: "pool".to_string(),
            conversion_id: "cid".to_string(),
            status: ConversionStatus::Pending,
            fee: None,
            purpose: None,
            amount_adjustment: None,
        };
        let swap = make_swap_min("swap1", BoltzSwapStatus::Completed);
        assert!(boltz_metadata_from_swap(amm, &swap).is_none());
    }

    #[test_all]
    fn status_mapping_covers_all_variants() {
        assert_eq!(
            map_boltz_status_to_conversion(&BoltzSwapStatus::Created),
            ConversionStatus::Pending
        );
        assert_eq!(
            map_boltz_status_to_conversion(&BoltzSwapStatus::InvoicePaid),
            ConversionStatus::Pending
        );
        assert_eq!(
            map_boltz_status_to_conversion(&BoltzSwapStatus::TbtcLocked),
            ConversionStatus::Pending
        );
        assert_eq!(
            map_boltz_status_to_conversion(&BoltzSwapStatus::Claiming),
            ConversionStatus::Pending
        );
        assert_eq!(
            map_boltz_status_to_conversion(&BoltzSwapStatus::Settling),
            ConversionStatus::Pending
        );
        assert_eq!(
            map_boltz_status_to_conversion(&BoltzSwapStatus::Completed),
            ConversionStatus::Completed
        );
        assert_eq!(
            map_boltz_status_to_conversion(&BoltzSwapStatus::Expired),
            ConversionStatus::Failed
        );
        assert_eq!(
            map_boltz_status_to_conversion(&BoltzSwapStatus::Failed {
                reason: "x".to_string()
            }),
            ConversionStatus::Failed
        );
    }

    #[cfg(feature = "sqlite")]
    mod storage_tests {
        use std::path::PathBuf;
        use std::sync::Arc;

        use boltz_client::models::{Asset, BoltzSwap, BoltzSwapStatus, BridgeKind};

        use super::super::*;
        use crate::persist::sqlite::SqliteStorage;

        fn create_temp_dir(name: &str) -> PathBuf {
            let mut path = std::env::temp_dir();
            path.push(format!("breez-test-{}-{}", name, uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).unwrap();
            path
        }

        fn make_swap(id: &str, status: BoltzSwapStatus) -> BoltzSwap {
            BoltzSwap {
                id: id.to_string(),
                status,
                bridge_kind: BridgeKind::Oft,
                claim_key_index: 0,
                chain_id: 42161,
                claim_address: "0xclaim".to_string(),
                destination_address: "0xdest".to_string(),
                destination_chain: "Arbitrum One".to_string(),
                asset: Asset::Usdt,
                refund_address: "0xrefund".to_string(),
                erc20swap_address: "0xswap".to_string(),
                router_address: "0xrouter".to_string(),
                invoice: "lnbc1000n".to_string(),
                invoice_amount_sats: 100_000,
                onchain_amount: 99_500,
                expected_output_amount: 70_900_000,
                slippage_bps: 100,
                timeout_block_height: 123_456,
                lockup_tx_id: None,
                claim_tx_hash: None,
                pending_call_id: None,
                delivered_amount: None,
                bridge_ref: None,
                created_at: 1_700_000_000,
                updated_at: 1_700_000_000,
            }
        }

        #[tokio::test]
        async fn missing_payment_is_silent_noop() {
            let dir = create_temp_dir("boltz_event_missing_payment");
            let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&dir).unwrap());
            let listener = BoltzSdkEventListener::new(Arc::clone(&storage));

            // No payment row carrying this invoice — the listener should
            // short-circuit without erroring.
            let swap = make_swap("orphan_swap", BoltzSwapStatus::InvoicePaid);
            listener.handle_swap_updated(&swap).await.unwrap();
        }
    }
}
