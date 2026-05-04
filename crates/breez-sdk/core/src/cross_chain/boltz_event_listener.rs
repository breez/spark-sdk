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

use boltz_client::{BoltzEventListener, BoltzSwapEvent, events, models::BoltzSwapStatus};
use tracing::{debug, error};

use crate::{
    ConversionInfo, ConversionStatus, PaymentMetadata, Storage,
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
            // swap KV via `BoltzStorage::update_swap` independently, and the
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
            // invoice. A later swap event carries the full state and retries.
            debug!(
                swap_id = %swap.id,
                payment_id = %payment_id,
                "Payment has no ConversionInfo attached, skipping"
            );
            return Ok(());
        };

        let ConversionInfo::Boltz {
            swap_id,
            chain,
            chain_id,
            asset,
            recipient_address,
            invoice,
            invoice_amount_sats,
            estimated_out,
            fee,
            max_slippage_bps,
            quote_degraded,
            asset_decimals,
            asset_contract,
            ..
        } = conversion_info
        else {
            debug!(
                swap_id = %swap.id,
                payment_id = %payment_id,
                "Payment has non-Boltz ConversionInfo, skipping"
            );
            return Ok(());
        };

        let new_status = map_boltz_status_to_conversion(&swap.status);

        let updated = PaymentMetadata {
            conversion_info: Some(ConversionInfo::Boltz {
                swap_id,
                chain,
                chain_id,
                asset,
                recipient_address,
                invoice,
                invoice_amount_sats,
                estimated_out,
                delivered_amount: swap.delivered_amount.map(u128::from),
                lz_guid: swap.lz_guid.clone(),
                status: new_status,
                fee,
                max_slippage_bps,
                quote_degraded,
                asset_decimals,
                asset_contract,
            }),
            ..Default::default()
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
            estimated_out,
            delivered_amount,
            lz_guid,
            status,
            fee,
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
                estimated_out,
                delivered_amount,
                lz_guid,
                status,
                fee,
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
        | BoltzSwapStatus::Claiming => ConversionStatus::Pending,
        BoltzSwapStatus::Completed => ConversionStatus::Completed,
        BoltzSwapStatus::Failed { .. } | BoltzSwapStatus::Expired => ConversionStatus::Failed,
    }
}

#[cfg(test)]
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
mod tests {
    use std::path::PathBuf;

    use boltz_client::models::{BoltzSwap, BoltzSwapStatus, ChainId};

    use super::*;
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
            claim_key_index: 0,
            chain_id: 42161,
            claim_address: "0xclaim".to_string(),
            destination_address: "0xdest".to_string(),
            destination_chain: ChainId::new("arbitrum one"),
            refund_address: "0xrefund".to_string(),
            erc20swap_address: "0xswap".to_string(),
            router_address: "0xrouter".to_string(),
            invoice: "lnbc1000n".to_string(),
            invoice_amount_sats: 100_000,
            onchain_amount: 99_500,
            expected_usdt_amount: 70_900_000,
            slippage_bps: 100,
            timeout_block_height: 123_456,
            lockup_tx_id: None,
            claim_tx_hash: None,
            delivered_amount: None,
            lz_guid: None,
            created_at: 1_700_000_000,
            updated_at: 1_700_000_000,
        }
    }

    #[test]
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
