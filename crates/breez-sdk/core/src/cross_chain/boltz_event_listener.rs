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
    ConversionInfo, ConversionStatus, PaymentDetails, PaymentMetadata, Storage,
    cross_chain::boltz::swap_payment_map_key,
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
        let mapping_key = swap_payment_map_key(&swap.id);
        let Some(payment_id) = self
            .storage
            .get_cached_item(mapping_key.clone())
            .await
            .map_err(|e| format!("read mapping {mapping_key}: {e}"))?
        else {
            // Prepare-without-send orphan: no payment row exists. The
            // adapter KV row is updated by boltz-client via
            // `BoltzStorage::update_swap` — that path is independent of
            // this listener.
            debug!(
                swap_id = %swap.id,
                "No payment mapping for Boltz swap, skipping payment-row update"
            );
            return Ok(());
        };

        let existing = self
            .storage
            .get_payment_by_id(payment_id.clone())
            .await
            .map_err(|e| format!("fetch payment {payment_id}: {e}"))?;

        let Some(conversion_info) = extract_conversion_info(existing.details) else {
            return Err(format!(
                "Payment {payment_id} has no ConversionInfo attached"
            ));
        };

        let ConversionInfo::Boltz {
            swap_id,
            destination_chain,
            destination_asset,
            destination_address,
            invoice,
            invoice_amount_sats,
            estimated_out,
            fee,
            max_slippage_bps,
            quote_degraded,
            ..
        } = conversion_info
        else {
            return Err(format!(
                "Payment {payment_id} has non-Boltz ConversionInfo attached"
            ));
        };

        let new_status = map_boltz_status_to_conversion(&swap.status);

        let updated = PaymentMetadata {
            conversion_info: Some(ConversionInfo::Boltz {
                swap_id,
                destination_chain,
                destination_asset,
                destination_address,
                invoice,
                invoice_amount_sats,
                estimated_out,
                delivered_amount: swap.delivered_amount.map(u128::from),
                lz_guid: swap.lz_guid.clone(),
                status: new_status,
                fee,
                max_slippage_bps,
                quote_degraded,
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
        let mapping_key = swap_payment_map_key(&swap.id);
        let Some(payment_id) = self
            .storage
            .get_cached_item(mapping_key.clone())
            .await
            .map_err(|e| format!("read mapping {mapping_key}: {e}"))?
        else {
            return Ok(());
        };

        let existing = self
            .storage
            .get_payment_by_id(payment_id.clone())
            .await
            .map_err(|e| format!("fetch payment {payment_id}: {e}"))?;

        let Some(ConversionInfo::Boltz {
            swap_id,
            destination_chain,
            destination_asset,
            destination_address,
            invoice,
            invoice_amount_sats,
            estimated_out,
            delivered_amount,
            lz_guid,
            status,
            fee,
            max_slippage_bps,
            ..
        }) = extract_conversion_info(existing.details)
        else {
            return Ok(());
        };

        let updated = PaymentMetadata {
            conversion_info: Some(ConversionInfo::Boltz {
                swap_id,
                destination_chain,
                destination_asset,
                destination_address,
                invoice,
                invoice_amount_sats,
                estimated_out,
                delivered_amount,
                lz_guid,
                status,
                fee,
                max_slippage_bps,
                quote_degraded: true,
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

/// Extract `ConversionInfo` from whichever [`PaymentDetails`] variant carries
/// it. Boltz conversions live on `Lightning` details (the hold-invoice pay);
/// `Spark` and `Token` exist for other cross-chain providers and are included
/// so a single helper serves every caller.
fn extract_conversion_info(details: Option<PaymentDetails>) -> Option<ConversionInfo> {
    match details? {
        PaymentDetails::Spark {
            conversion_info, ..
        }
        | PaymentDetails::Token {
            conversion_info, ..
        }
        | PaymentDetails::Lightning {
            conversion_info, ..
        } => conversion_info,
        _ => None,
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
    async fn missing_mapping_is_silent_noop() {
        let dir = create_temp_dir("boltz_event_missing_mapping");
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&dir).unwrap());
        let listener = BoltzSdkEventListener::new(Arc::clone(&storage));

        // No mapping cache entry, no payment row, no metadata — nothing to
        // write. The listener should short-circuit without erroring.
        let swap = make_swap("orphan_swap", BoltzSwapStatus::InvoicePaid);
        listener.handle_swap_updated(&swap).await.unwrap();
    }
}
