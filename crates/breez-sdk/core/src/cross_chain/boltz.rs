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
    CrossChainPrepared, CrossChainProvider, CrossChainRouteFilter, CrossChainRoutePair,
    CrossChainSendResult, CrossChainService,
};
use crate::{
    ConversionInfo, ConversionStatus, Network, PaymentMetadata, Storage, error::SdkError,
    sdk::LightningSender, utils::payments::insert_or_cache_payment_metadata,
};

/// Cache KV key used to look up the payment row attached to a given Boltz swap.
pub(crate) fn swap_payment_map_key(swap_id: &str) -> String {
    format!("boltz_swap_{swap_id}")
}

/// Cache KV key used to stash provider-side context between `prepare` and
/// `send`. Keyed by the Boltz swap id so a single adapter KV lookup reloads
/// everything the send stage needs.
fn prepared_context_key(swap_id: &str) -> String {
    format!("boltz_prepared_ctx_{swap_id}")
}

/// Provider-specific context stashed on the prepared response. We serialize
/// it into the SDK cache KV keyed by swap id, keeping `CrossChainPrepared`
/// unchanged so the send dispatcher stays generic.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct PreparedContext {
    swap_id: String,
    invoice: String,
    invoice_amount_sats: u64,
    ln_fee_sats: u64,
    max_slippage_bps: u32,
    destination_chain: String,
    destination_address: String,
    estimated_out: u128,
    fee_amount: u128,
}

pub(crate) struct BoltzService {
    client: Arc<BoltzClient>,
    spark_wallet: Arc<SparkWallet>,
    storage: Arc<dyn Storage>,
    #[allow(dead_code)] // surfaced for future multi-network support
    network: Network,
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
        network: Network,
        lightning_sender: Arc<LightningSender>,
    ) -> Self {
        info!("Boltz service initialized");
        Self {
            client,
            spark_wallet,
            storage,
            network,
            lightning_sender,
        }
    }

    /// Best-effort helper to build a boltz-client [`BoltzClientConfig`] for
    /// the given network + referral id. Returns `None` on non-mainnet
    /// networks since Boltz only supports mainnet today.
    pub(crate) fn default_client_config(
        network: Network,
        referral_id: String,
    ) -> Option<BoltzClientConfig> {
        match network {
            Network::Mainnet => Some(BoltzClientConfig::mainnet(referral_id)),
            Network::Regtest => None,
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
            .map(|spec| CrossChainRoutePair {
                provider: CrossChainProvider::Boltz,
                chain: spec.id.as_str().to_string(),
                asset: "USDT".to_string(),
                contract_address: spec.token_address.clone(),
                decimals: 6,
                exact_out_eligible: false,
            })
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
    ) -> Result<CrossChainPrepared, SdkError> {
        // v1 Boltz is BTC-only. Tokens must be rejected before we commit any
        // state on Boltz's side.
        if token_identifier.is_some() {
            return Err(SdkError::InvalidInput(
                "Boltz does not support token sends in v1".to_string(),
            ));
        }

        let invoice_amount_sats = u64::try_from(amount).map_err(|_| {
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

        debug!(
            "Boltz: preparing reverse swap to {recipient_address} on {}, amount {invoice_amount_sats} sats",
            route.chain
        );

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

        let ln_fee_sats = self
            .spark_wallet
            .fetch_lightning_send_fee_estimate(&created.invoice, None)
            .await
            .map_err(|e| {
                SdkError::Generic(format!(
                    "Failed to fetch lightning send fee estimate for Boltz invoice: {e}"
                ))
            })?;

        // Fee denominated in sats: Boltz spread (invoice sats - on-chain sats
        // paid out) + lightning routing fee budget.
        let boltz_spread_sats = created
            .invoice_amount_sats
            .saturating_sub(prepared.estimated_onchain_amount);
        let fee_amount = u128::from(boltz_spread_sats).saturating_add(u128::from(ln_fee_sats));
        let estimated_out = u128::from(prepared.usdt_amount);
        let invoice_amount_sats = created.invoice_amount_sats;
        let resolved_slippage = max_slippage_bps.unwrap_or(prepared.slippage_bps);

        let context = PreparedContext {
            swap_id: created.swap_id.clone(),
            invoice: created.invoice.clone(),
            invoice_amount_sats,
            ln_fee_sats,
            max_slippage_bps: resolved_slippage,
            destination_chain: route.chain.clone(),
            destination_address: recipient_address.to_string(),
            estimated_out,
            fee_amount,
        };
        let ctx_json = serde_json::to_string(&context)
            .map_err(|e| SdkError::Generic(format!("Failed to serialize Boltz context: {e}")))?;
        self.storage
            .set_cached_item(prepared_context_key(&created.swap_id), ctx_json)
            .await
            .map_err(|e| SdkError::Generic(format!("Failed to cache Boltz context: {e}")))?;

        Ok(CrossChainPrepared {
            quote_id: created.swap_id.clone(),
            deposit_request: created.invoice,
            amount_in: u128::from(invoice_amount_sats),
            estimated_out,
            fee_amount,
            fee_asset: None,
            expires_at: prepared.expires_at.to_string(),
            pair: route.clone(),
            recipient_address: recipient_address.to_string(),
            token_identifier: None,
        })
    }

    #[allow(clippy::large_futures)]
    async fn send(&self, prepared: &CrossChainPrepared) -> Result<CrossChainSendResult, SdkError> {
        let ctx_raw = self
            .storage
            .get_cached_item(prepared_context_key(&prepared.quote_id))
            .await
            .map_err(|e| SdkError::Generic(format!("Failed to read Boltz context: {e}")))?
            .ok_or_else(|| {
                SdkError::Generic(format!(
                    "Boltz prepare context missing for swap {}",
                    prepared.quote_id
                ))
            })?;
        let context: PreparedContext = serde_json::from_str(&ctx_raw)
            .map_err(|e| SdkError::Generic(format!("Failed to deserialize Boltz context: {e}")))?;

        // Delegate the LN leg to the shared helper. It pays the hold
        // invoice, builds the Payment row, persists it, and spawns SSP-side
        // polling — the same path `send_bolt11_invoice` takes. On failure
        // the hold invoice eventually times out on Boltz's side; no payment
        // row is written, so there is nothing to reconcile on ours.
        let sdk_payment = self
            .lightning_sender
            .pay_and_persist_lightning_invoice(
                &context.invoice,
                None,
                context.ln_fee_sats,
                false,
                u128::from(context.invoice_amount_sats),
                None,
            )
            .await
            .map_err(|e| SdkError::Generic(format!("Boltz lightning payment failed: {e}")))?;
        let spark_payment_id = sdk_payment.id.clone();

        debug!(
            "Boltz: lightning payment {spark_payment_id} sent for swap {}",
            context.swap_id
        );

        let metadata = PaymentMetadata {
            conversion_info: Some(ConversionInfo::Boltz {
                swap_id: context.swap_id.clone(),
                destination_chain: context.destination_chain.clone(),
                destination_address: context.destination_address.clone(),
                invoice: context.invoice.clone(),
                invoice_amount_sats: context.invoice_amount_sats,
                estimated_out: context.estimated_out,
                status: ConversionStatus::Pending,
                fee: Some(context.fee_amount),
                max_slippage_bps: context.max_slippage_bps,
                quote_degraded: false,
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

        // Handle the event listener uses to map swap → payment row for
        // in-place metadata updates.
        self.storage
            .set_cached_item(swap_payment_map_key(&context.swap_id), payment_id.clone())
            .await
            .map_err(|e| {
                SdkError::Generic(format!("Failed to cache Boltz swap → payment map: {e}"))
            })?;

        Ok(CrossChainSendResult {
            order_id: context.swap_id,
            payment_id,
        })
    }
}

fn boltz_err_to_sdk(err: &BoltzError) -> SdkError {
    SdkError::Generic(format!("Boltz: {err}"))
}
