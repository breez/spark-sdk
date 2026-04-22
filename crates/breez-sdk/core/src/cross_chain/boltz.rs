//! Boltz reverse-swap cross-chain provider.
//!
//! Implements [`CrossChainService`] for Boltz's sats → USDT reverse swap.
//! Routing/quoting happens via the inner [`boltz_client::BoltzService`];
//! payment rows are written at send time only (after the lightning leg
//! succeeds) and updated silently by [`super::boltz_event_listener`] as the
//! WebSocket drives the swap to a terminal state.

use std::sync::Arc;

use boltz_client::{
    BoltzError, BoltzService as BoltzClient,
    config::{BoltzConfig as BoltzClientConfig, MAX_SLIPPAGE_BPS},
    models::{ChainId, PreparedSwap},
};
use spark_wallet::SparkWallet;
use tracing::{debug, error, info};

use super::{
    CrossChainFeeMode, CrossChainPrepared, CrossChainProvider, CrossChainProviderContext,
    CrossChainRouteFilter, CrossChainRoutePair, CrossChainSendResult, CrossChainService,
    SourceAsset,
};
use crate::{
    ConversionInfo, ConversionStatus, Network, PaymentMetadata, Storage, error::SdkError,
    sdk::LightningSender, utils::payments::insert_or_cache_payment_metadata,
};

pub(crate) struct BoltzService {
    client: Arc<BoltzClient>,
    spark_wallet: Arc<SparkWallet>,
    storage: Arc<dyn Storage>,
    /// Shared helper that owns the "pay LN invoice + persist Payment row +
    /// poll SSP" sequence. Reused by `send_bolt11_invoice` on the SDK and
    /// by this provider so Boltz hold-invoice pays behave identically to
    /// direct LN sends.
    lightning_sender: Arc<LightningSender>,
}

impl BoltzService {
    /// Construct the SDK-side wrapper. Does not perform I/O; the caller is
    /// expected to construct the inner [`BoltzClient`] (which owns the
    /// WebSocket + background monitor) and pass it in already initialized.
    pub(crate) fn new(
        client: Arc<BoltzClient>,
        spark_wallet: Arc<SparkWallet>,
        storage: Arc<dyn Storage>,
        lightning_sender: Arc<LightningSender>,
    ) -> Self {
        info!("Boltz service initialized");
        Self {
            client,
            spark_wallet,
            storage,
            lightning_sender,
        }
    }

    /// Best-effort helper to build a boltz-client [`BoltzClientConfig`] for
    /// the given network. Returns `None` on non-mainnet networks since Boltz
    /// only supports mainnet today.
    pub(crate) fn default_client_config(network: Network) -> Option<BoltzClientConfig> {
        const BREEZ_REFERRAL_ID: &str = "breez-sdk";
        match network {
            Network::Mainnet => Some(BoltzClientConfig::mainnet(BREEZ_REFERRAL_ID.to_string())),
            Network::Regtest => None,
        }
    }

    /// One-shot prepare for `FeesExcluded`: `amount` is the provider invoice
    /// target. The wallet pays `amount + ln_fee_sats` in total at send time.
    async fn prepare_fees_excluded(
        &self,
        recipient_address: &str,
        route: &CrossChainRoutePair,
        chain: ChainId,
        invoice_amount_sats: u64,
        max_slippage_bps: Option<u32>,
    ) -> Result<CrossChainPrepared, SdkError> {
        debug!(
            "Boltz: preparing reverse swap (FeesExcluded) to {recipient_address} on {}, amount {invoice_amount_sats} sats",
            route.chain
        );

        let (prepared, created) = self
            .create_swap(
                recipient_address,
                chain,
                invoice_amount_sats,
                max_slippage_bps,
            )
            .await?;

        let ln_fee_sats = self.fetch_ln_fee(&created.invoice).await?;

        Ok(Self::build_prepared(
            route,
            recipient_address,
            &prepared,
            created,
            ln_fee_sats,
            max_slippage_bps,
            CrossChainFeeMode::FeesExcluded,
        ))
    }

    /// Two-phase prepare for `FeesIncluded`: size the real invoice to
    /// `amount - ln_fee_probe_sats` so `invoice_sats + ln_fee_probe <= amount`.
    ///
    /// Mirrors LNURL pay's `FeesIncluded` pattern at `lnurl.rs`. A throwaway swap
    /// is created to probe the LN fee; it times out server-side (~24h). The
    /// probe value is stored on the prepare as `source_transfer_fee_sats`
    /// and enforced as a hard cap at send time.
    async fn prepare_fees_included(
        &self,
        recipient_address: &str,
        route: &CrossChainRoutePair,
        chain: ChainId,
        total_sats: u64,
        max_slippage_bps: Option<u32>,
    ) -> Result<CrossChainPrepared, SdkError> {
        debug!(
            "Boltz: preparing reverse swap (FeesIncluded) to {recipient_address} on {}, total {total_sats} sats",
            route.chain
        );

        // Phase 1: throwaway invoice at `total_sats` to probe LN fee.
        let (_throwaway_prepared, throwaway_created) = self
            .create_swap(
                recipient_address,
                chain.clone(),
                total_sats,
                max_slippage_bps,
            )
            .await?;
        let ln_fee_probe_sats = self.fetch_ln_fee(&throwaway_created.invoice).await?;

        if total_sats <= ln_fee_probe_sats {
            return Err(SdkError::InvalidInput(format!(
                "Amount too small for cross-chain send: {total_sats} sats <= LN fee {ln_fee_probe_sats} sats."
            )));
        }

        // Phase 2: real invoice sized to leave room for the probed fee.
        let real_invoice_sats = total_sats.saturating_sub(ln_fee_probe_sats);
        let (prepared, created) = self
            .create_swap(
                recipient_address,
                chain,
                real_invoice_sats,
                max_slippage_bps,
            )
            .await?;
        let ln_fee_final_sats = self.fetch_ln_fee(&created.invoice).await?;

        // Mirrors LNURL's guard: if fee moved between queries, fail so caller retries.
        if ln_fee_final_sats > ln_fee_probe_sats {
            return Err(SdkError::Generic(
                "Boltz LN fee increased between prepare queries. Please retry.".to_string(),
            ));
        }

        // Store the probe (not the final) as the budget — matches LNURL's
        // `fee_sats = first_fee` and keeps `invoice_sats + max_fee <= amount`.
        Ok(Self::build_prepared(
            route,
            recipient_address,
            &prepared,
            created,
            ln_fee_probe_sats,
            max_slippage_bps,
            CrossChainFeeMode::FeesIncluded,
        ))
    }

    async fn create_swap(
        &self,
        recipient_address: &str,
        chain: ChainId,
        invoice_amount_sats: u64,
        max_slippage_bps: Option<u32>,
    ) -> Result<(PreparedSwap, boltz_client::models::CreatedSwap), SdkError> {
        let prepared: PreparedSwap = self
            .client
            .prepare_reverse_swap_from_sats(
                recipient_address,
                chain,
                invoice_amount_sats,
                max_slippage_bps,
            )
            .await
            .map_err(|e| boltz_err_to_sdk(&e))?;

        // `create_reverse_swap` commits a HD key index, POSTs to Boltz to
        // create the swap on the server, and writes a `BoltzSwap` row into
        // the adapter cache KV. After this call Boltz is holding the swap
        // state, so the only path back to a clean state is a timeout.
        let created = self
            .client
            .create_reverse_swap(&prepared)
            .await
            .map_err(|e| boltz_err_to_sdk(&e))?;

        Ok((prepared, created))
    }

    async fn fetch_ln_fee(&self, invoice: &str) -> Result<u64, SdkError> {
        self.spark_wallet
            .fetch_lightning_send_fee_estimate(invoice, None)
            .await
            .map_err(|e| {
                SdkError::Generic(format!(
                    "Failed to fetch lightning send fee estimate for Boltz invoice: {e}"
                ))
            })
    }

    #[allow(clippy::too_many_arguments)]
    fn build_prepared(
        route: &CrossChainRoutePair,
        recipient_address: &str,
        prepared: &PreparedSwap,
        created: boltz_client::models::CreatedSwap,
        ln_fee_sats: u64,
        max_slippage_bps: Option<u32>,
        fee_mode: CrossChainFeeMode,
    ) -> CrossChainPrepared {
        // `fee_amount` is the Boltz spread only (invoice sats - on-chain sats
        // paid out). The LN routing budget is exposed separately as
        // `source_transfer_fee_sats` — not double-counted here.
        let boltz_spread_sats = created
            .invoice_amount_sats
            .saturating_sub(prepared.estimated_onchain_amount);
        let fee_amount = u128::from(boltz_spread_sats);
        let estimated_out = u128::from(prepared.usdt_amount);
        let invoice_amount_sats = created.invoice_amount_sats;
        let resolved_slippage = max_slippage_bps.unwrap_or(prepared.slippage_bps);

        let provider_context = CrossChainProviderContext::Boltz {
            swap_id: created.swap_id.clone(),
            invoice: created.invoice,
            max_slippage_bps: resolved_slippage,
        };

        CrossChainPrepared {
            amount_in: u128::from(invoice_amount_sats),
            estimated_out,
            fee_amount,
            fee_asset: None,
            source_transfer_fee_sats: ln_fee_sats,
            fee_mode,
            expires_at: prepared.expires_at.to_string(),
            pair: route.clone(),
            recipient_address: recipient_address.to_string(),
            token_identifier: None,
            provider_context,
        }
    }
}

#[macros::async_trait]
impl CrossChainService for BoltzService {
    async fn get_routes(
        &self,
        filter: &CrossChainRouteFilter,
    ) -> Result<Vec<CrossChainRoutePair>, SdkError> {
        let address_details = match filter {
            CrossChainRouteFilter::Send { address_details } => address_details,
            // v1 Boltz is reverse-swap only (BTC/sats -> external). Submarine
            // swaps (USDT -> LN) are out of scope for v1 and will populate
            // this branch when they land.
            CrossChainRouteFilter::Receive { .. } => return Ok(Vec::new()),
        };

        // `chains_accepting` validates the raw recipient address against
        // every destination's transport and returns only those whose parser
        // accepts it — this replaces the old hand-written address-family
        // filter and automatically picks up any new chains that USDT0
        // publishes.
        let routes = self
            .client
            .chains_accepting(&address_details.address)
            .into_iter()
            .map(spec_to_route_pair)
            .collect();
        Ok(routes)
    }

    async fn prepare(
        &self,
        recipient_address: &str,
        route: &CrossChainRoutePair,
        amount: u128,
        token_identifier: Option<String>,
        max_slippage_bps: Option<u32>,
        fee_mode: CrossChainFeeMode,
    ) -> Result<CrossChainPrepared, SdkError> {
        // v1 Boltz is BTC-only. Tokens must be rejected before we commit any
        // state on Boltz's side.
        if token_identifier.is_some() {
            return Err(SdkError::InvalidInput(
                "Boltz does not support token sends in v1".to_string(),
            ));
        }

        let total_sats = u64::try_from(amount).map_err(|_| {
            SdkError::InvalidInput(format!(
                "Amount {amount} exceeds u64::MAX sats for Boltz reverse swap"
            ))
        })?;

        let chain = ChainId::new(&route.chain);

        if let Some(bps) = max_slippage_bps
            && bps > MAX_SLIPPAGE_BPS
        {
            return Err(SdkError::InvalidInput(format!(
                "max_slippage_bps {bps} exceeds Boltz maximum {MAX_SLIPPAGE_BPS}"
            )));
        }

        match fee_mode {
            CrossChainFeeMode::FeesExcluded => {
                self.prepare_fees_excluded(
                    recipient_address,
                    route,
                    chain,
                    total_sats,
                    max_slippage_bps,
                )
                .await
            }
            CrossChainFeeMode::FeesIncluded => {
                self.prepare_fees_included(
                    recipient_address,
                    route,
                    chain,
                    total_sats,
                    max_slippage_bps,
                )
                .await
            }
        }
    }

    #[allow(clippy::large_futures)]
    async fn send(&self, prepared: &CrossChainPrepared) -> Result<CrossChainSendResult, SdkError> {
        let CrossChainProviderContext::Boltz {
            swap_id,
            invoice,
            max_slippage_bps,
        } = &prepared.provider_context
        else {
            return Err(SdkError::Generic(
                "Boltz send called with non-Boltz provider context".to_string(),
            ));
        };

        let invoice_amount_sats = u64::try_from(prepared.amount_in)
            .map_err(|e| SdkError::Generic(format!("Boltz invoice amount exceeds u64: {e}")))?;

        let ln_fee_budget = prepared.source_transfer_fee_sats;

        // Compute the LN payment amount based on fee_mode. For FeesIncluded,
        // mirror LNURL's overpayment logic so the wallet actually consumes the
        // user's budget when current_fee < ln_fee_probe.
        let ln_amount_sats = match prepared.fee_mode {
            CrossChainFeeMode::FeesExcluded => {
                // Pay the invoice as-is; `max_fee_sat = ln_fee_budget` protects
                // against fee drift (validated downstream).
                None
            }
            CrossChainFeeMode::FeesIncluded => {
                let current_fee = self
                    .spark_wallet
                    .fetch_lightning_send_fee_estimate(invoice, None)
                    .await
                    .map_err(|e| {
                        SdkError::Generic(format!(
                            "Failed to re-estimate Boltz LN fee at send: {e}"
                        ))
                    })?;

                if current_fee > ln_fee_budget {
                    return Err(SdkError::Generic(
                        "Fee increased since prepare. Please retry.".to_string(),
                    ));
                }

                let overpayment_uncapped = ln_fee_budget.saturating_sub(current_fee);
                let max_allowed_overpayment = current_fee.max(1);
                if overpayment_uncapped > max_allowed_overpayment {
                    return Err(SdkError::Generic(format!(
                        "Fee overpayment ({overpayment_uncapped} sats) exceeds allowed maximum ({max_allowed_overpayment} sats)"
                    )));
                }

                Some(invoice_amount_sats.saturating_add(overpayment_uncapped))
            }
        };

        // Delegate the LN leg to the shared helper. It pays the hold
        // invoice, builds the Payment row, persists it, and spawns SSP-side
        // polling — the same path `send_bolt11_invoice` takes. On failure
        // the hold invoice eventually times out on Boltz's side; no payment
        // row is written, so there is nothing to reconcile on ours.
        let sdk_payment = self
            .lightning_sender
            .pay_and_persist_lightning_invoice(
                invoice,
                ln_amount_sats,
                ln_fee_budget,
                false,
                prepared.amount_in,
                None,
            )
            .await
            .map_err(|e| SdkError::Generic(format!("Boltz lightning payment failed: {e}")))?;
        let spark_payment_id = sdk_payment.id.clone();

        debug!("Boltz: lightning payment {spark_payment_id} sent for swap {swap_id}");

        let metadata = PaymentMetadata {
            conversion_info: Some(ConversionInfo::Boltz {
                swap_id: swap_id.clone(),
                chain: prepared.pair.chain.clone(),
                chain_id: prepared.pair.chain_id.clone(),
                asset: prepared.pair.asset.clone(),
                recipient_address: prepared.recipient_address.clone(),
                invoice: invoice.clone(),
                invoice_amount_sats,
                estimated_out: prepared.estimated_out,
                delivered_amount: None,
                lz_guid: None,
                status: ConversionStatus::Pending,
                fee: Some(prepared.fee_amount),
                max_slippage_bps: *max_slippage_bps,
                quote_degraded: false,
                asset_decimals: u32::from(prepared.pair.decimals),
                asset_contract: prepared.pair.contract_address.clone(),
            }),
            ..Default::default()
        };

        let payment_id = insert_or_cache_payment_metadata(
            &spark_payment_id,
            metadata,
            &self.spark_wallet,
            &self.storage,
            true,
        )
        .await
        .unwrap_or_else(|e| {
            error!("Failed to persist Boltz metadata for payment {spark_payment_id}: {e:?}");
            spark_payment_id.clone()
        });

        Ok(CrossChainSendResult {
            order_id: swap_id.clone(),
            payment_id,
        })
    }
}

fn boltz_err_to_sdk(err: &BoltzError) -> SdkError {
    SdkError::Generic(format!("Boltz: {err}"))
}

/// Build a [`CrossChainRoutePair`] from a Boltz [`ChainSpec`]. Surfaces
/// `chain_id` for EVM chains as a decimal string; non-EVM transports
/// (Solana, Tron) get `None`, matching the USDT0 deployments feed.
fn spec_to_route_pair(spec: &boltz_client::models::ChainSpec) -> CrossChainRoutePair {
    CrossChainRoutePair {
        provider: CrossChainProvider::Boltz,
        chain: spec.id.as_str().to_string(),
        chain_id: spec.evm_chain_id.map(|id| id.to_string()),
        asset: spec.asset_symbol().to_string(),
        contract_address: spec.token_address.clone(),
        decimals: 6,
        exact_out_eligible: false,
        supported_sources: vec![SourceAsset::Bitcoin],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use boltz_client::models::{ChainSpec, NetworkTransport, Usdt0Kind};

    fn test_spec(evm_chain_id: Option<u64>, transport: NetworkTransport) -> ChainSpec {
        ChainSpec {
            id: ChainId::new("arbitrum one"),
            is_source: false,
            display_name: "Arbitrum One".to_string(),
            transport,
            evm_chain_id,
            lz_eid: 30110,
            oft_address: "0xoft".to_string(),
            token_address: Some("0xtoken".to_string()),
            mesh: Usdt0Kind::Native,
        }
    }

    #[test]
    fn spec_to_pair_maps_evm_chain_id_to_decimal_string() {
        let spec = test_spec(Some(42161), NetworkTransport::Evm);
        let pair = spec_to_route_pair(&spec);

        assert_eq!(pair.provider, CrossChainProvider::Boltz);
        assert_eq!(
            pair.chain_id,
            Some("42161".to_string()),
            "EVM chain id should render as a decimal string"
        );
        assert_eq!(pair.chain, "arbitrum one");
        assert_eq!(pair.asset, "USDT0");
        assert_eq!(pair.contract_address.as_deref(), Some("0xtoken"));
        assert_eq!(pair.decimals, 6);
        assert!(!pair.exact_out_eligible);
    }

    #[test]
    fn spec_to_pair_preserves_none_for_non_evm_transports() {
        let spec = test_spec(None, NetworkTransport::Solana);
        let pair = spec_to_route_pair(&spec);

        assert_eq!(
            pair.chain_id, None,
            "Non-EVM transports (Solana, Tron) expose no chain_id"
        );
    }
}
