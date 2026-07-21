//! Flashnet Orchestra cross-chain provider.
//!
//! Implements [`CrossChainProvider`] for the Orchestra bridge/swap API.
//! Handles quoting, sending (deposit + submit), and background monitoring
//! of in-flight orders.

use std::collections::HashMap;
use std::sync::Arc;

use breez_sdk_common::breez_server::BreezServer;
use breez_sdk_common::fiat::FiatService;
use breez_sdk_common::input::CrossChainAddressFamily;
use chrono::DateTime;
use flashnet::orchestra::{
    AmountMode, EstimateRequest, EstimateResponse, Order, OrderStatus, QuoteRequest, QuoteResponse,
    Route, RouteAsset, StatusResponse, SubmitResponse,
};
use flashnet::{FlashnetError, OrchestraClient, OrchestraConfig, OrchestraConfigResolver};
use platform_utils::time::{Duration, SystemTime, UNIX_EPOCH};
use platform_utils::tokio;
use spark_wallet::SparkWallet;
use tokio::{
    select,
    sync::{broadcast, watch},
};
use tracing::{Instrument, debug, error, info, warn};

use crate::error::SdkError;
use crate::persist::{ConversionFilter, StorageListPaymentsRequest, StoragePaymentDetailsFilter};
use crate::{ConversionInfo, ConversionStatus, PaymentDetails, Storage};

use super::{
    CrossChainFeeMode, CrossChainProvider, CrossChainProviderContext, CrossChainReceiveInfo,
    CrossChainReceivePrepared, CrossChainRouteFilter, CrossChainRoutePair, CrossChainSendPrepared,
    CrossChainService, SparkAsset, derive_btc_leg_transfer_id,
    orchestra_storage_adapter::{OrchestraStorageAdapter, OrchestraSwapData},
};

use crate::utils::{
    payments::{fetch_and_process_payment, resolve_and_insert_payment_metadata},
    polling::{PollSchedule, poll_until},
};

const DEFAULT_AFFILIATE_ID: &str = "breez_sdk";
// Polling cadence for the outbound Spark transfer leg.
const SEND_POLL_INITIAL_DELAY_MS: u64 = 500;
const SEND_POLL_MAX_DELAY_MS: u64 = 2000;
const SEND_POLL_TIMEOUT_SECS: u64 = 30;
/// The canonical Spark chain string used by Orchestra.
const SPARK_CHAIN: &str = "spark";
/// How often the background monitor polls in-flight orders.
const MONITOR_INTERVAL: Duration = Duration::from_secs(30);
/// Grace period to keep probing a receive quote before giving up.
const RECEIVE_GRACE_SECS: u64 = 24 * 60 * 60;

/// Approximate fixed external cost per USD-stable receive that Orchestra
/// passes through but doesn't itemize in `total_fee_amount` (bridge /
/// aggregator / gas / rounding).
///
/// Empirically observed at ~$0.037-$0.038 across order sizes on USDC → USDB;
/// sized to $0.04 with a small margin for observed external drift up to
/// $0.045. Expressed in destination base units under the assumption of
/// 6-decimal USD stables. Applied only when the receive destination is in
/// `USD_STABLE_ASSETS`; call-site gate lives in `prepare_receive`. USDB cent
/// flooring is itemized inside Orchestra's own `total_fee_amount` as
/// `rounding_fee_amount`, so no separate grid buffer is needed here.
const STABLE_RECEIVE_BUFFER_BASE_UNITS: u128 = 40_000;

/// Probe-ratio threshold (bps of the inflated target) above which Orchestra's
/// estimate is treated as optimistic and shaved by the external-fee buffer.
/// Empirically CCTP-direct USDC probes report ~9885 bps; multi-hop and
/// non-CCTP routes report ~9385 bps. Below this gate Orchestra has already
/// priced external overhead into the estimate and the buffer would just
/// inflate the deposit without helping delivery.
const STABLE_RECEIVE_BUFFER_ACTIVATION_BPS: u128 = 9_700;

/// Resolves the Orchestra config from Breez server.
///
/// Fetched lazily on first cross-chain use (not at connect) so a slow or down
/// server never delays startup for what is an optional provider. A missing or
/// failed config returns an error that is not cached, so the next cross-chain
/// action retries: there is no bundled fallback key.
pub(crate) struct BreezServerOrchestraConfigResolver {
    breez_server: Arc<BreezServer>,
}

impl BreezServerOrchestraConfigResolver {
    pub(crate) fn new(breez_server: Arc<BreezServer>) -> Self {
        Self { breez_server }
    }
}

#[macros::async_trait]
impl OrchestraConfigResolver for BreezServerOrchestraConfigResolver {
    async fn resolve(&self) -> Result<OrchestraConfig, FlashnetError> {
        match self.breez_server.fetch_orchestra_config().await {
            Ok(Some(cfg)) => Ok(OrchestraConfig {
                base_url: cfg.base_url,
                api_key: cfg.api_key,
            }),
            Ok(None) => Err(FlashnetError::Generic(
                "Breez server has no Orchestra config".to_string(),
            )),
            Err(e) => Err(FlashnetError::Generic(format!(
                "Failed to fetch Orchestra config from Breez server: {e}"
            ))),
        }
    }
}

/// Source-side identity of an Orchestra route after `(dest, source)` matching.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ResolvedSparkAsset {
    /// Wire symbol (e.g. `"BTC"`, `"USDB"`).
    asset: String,
    /// Source-asset decimals.
    decimals: u8,
}

/// Flashnet Orchestra cross-chain provider.
pub(crate) struct OrchestraService {
    client: Arc<OrchestraClient>,
    spark_wallet: Arc<SparkWallet>,
    storage: Arc<dyn Storage>,
    fiat_service: Arc<dyn FiatService>,
    monitor_trigger: broadcast::Sender<()>,
}

impl OrchestraService {
    pub(crate) fn new(
        config_resolver: Arc<dyn OrchestraConfigResolver>,
        spark_wallet: Arc<SparkWallet>,
        storage: Arc<dyn Storage>,
        fiat_service: Arc<dyn FiatService>,
        shutdown_receiver: watch::Receiver<()>,
    ) -> Self {
        let client = Arc::new(OrchestraClient::new(
            config_resolver,
            Arc::clone(&spark_wallet),
        ));
        let (monitor_trigger, _) = broadcast::channel(10);

        let service = Self {
            client,
            spark_wallet,
            storage,
            fiat_service,
            monitor_trigger: monitor_trigger.clone(),
        };
        info!("Orchestra service initialized");
        service.spawn_monitor(shutdown_receiver, &monitor_trigger);
        service
    }

    fn trigger_monitor(&self) {
        let _ = self.monitor_trigger.send(());
    }

    fn spawn_monitor(
        &self,
        mut shutdown_receiver: watch::Receiver<()>,
        monitor_trigger: &broadcast::Sender<()>,
    ) {
        let storage = Arc::clone(&self.storage);
        let swap_storage = OrchestraStorageAdapter::new(Arc::clone(&storage));
        let client = Arc::clone(&self.client);
        let spark_wallet = Arc::clone(&self.spark_wallet);
        let fiat_service = Arc::clone(&self.fiat_service);
        let mut trigger_receiver = monitor_trigger.subscribe();
        let span = tracing::Span::current();

        tokio::spawn(
            async move {
                loop {
                    if let Err(e) = Self::poll_in_flight_sends(&storage, &client).await {
                        error!("Orchestra send-monitor poll failed: {e:?}");
                    }
                    if let Err(e) = Self::poll_in_flight_receives(
                        &storage,
                        &swap_storage,
                        &client,
                        &spark_wallet,
                        fiat_service.as_ref(),
                    )
                    .await
                    {
                        error!("Orchestra receive-monitor poll failed: {e:?}");
                    }

                    select! {
                        _ = shutdown_receiver.changed() => {
                            info!("Orchestra monitor shutdown signal received");
                            return;
                        }
                        _ = trigger_receiver.recv() => {
                            debug!("Orchestra monitor triggered");
                        }
                        () = tokio::time::sleep(MONITOR_INTERVAL) => {}
                    }
                }
            }
            .instrument(span),
        );
    }

    /// Polls Orchestra for status updates on in-flight cross-chain send orders.
    ///
    /// Queries storage for payments with `ConversionFilter::OrchestraPending`,
    /// calls the Orchestra `/status` endpoint for each, and updates the
    /// `ConversionInfo::Orchestra` metadata when the order reaches a terminal
    /// state (replacing the estimated output with the real `amount_out`).
    #[allow(clippy::too_many_lines)]
    async fn poll_in_flight_sends(
        storage: &Arc<dyn Storage>,
        client: &Arc<OrchestraClient>,
    ) -> Result<(), SdkError> {
        let pending = storage
            .list_payments(StorageListPaymentsRequest {
                payment_details_filter: Some(vec![
                    StoragePaymentDetailsFilter::Spark {
                        htlc_status: None,
                        conversion_filter: Some(ConversionFilter::OrchestraPending),
                    },
                    StoragePaymentDetailsFilter::Token {
                        conversion_filter: Some(ConversionFilter::OrchestraPending),
                        tx_hash: None,
                        tx_type: None,
                    },
                ]),
                ..Default::default()
            })
            .await?;

        debug!(
            "Orchestra monitor: found {} pending send orders",
            pending.len()
        );
        for payment in &pending {
            let Some(
                PaymentDetails::Spark {
                    conversion_info: Some(conversion_info),
                    ..
                }
                | PaymentDetails::Token {
                    conversion_info: Some(conversion_info),
                    ..
                },
            ) = &payment.details
            else {
                debug!(
                    "Orchestra monitor: payment {} has no conversion_info, skipping",
                    payment.id
                );
                continue;
            };

            let ConversionInfo::Orchestra {
                order_id,
                quote_id,
                read_token,
                chain,
                asset,
                ..
            } = conversion_info
            else {
                debug!(
                    "Orchestra monitor: payment {} conversion_info is not Orchestra variant, skipping",
                    payment.id
                );
                continue;
            };

            let lookup_id = if order_id.is_empty() {
                quote_id
            } else {
                order_id
            };
            debug!(
                "Orchestra monitor: checking payment {} (order={order_id}, quote={quote_id}, dest={chain}/{asset})",
                payment.id
            );

            // Prefer order_id, fall back to quote_id if order_id is empty
            // (can happen if /submit failed but we still want to track).
            let rt = read_token.as_deref();
            let status_response = if order_id.is_empty() {
                client.status_by_quote_id(quote_id, rt).await
            } else {
                client.status_by_id(order_id, rt).await
            };

            let status_response = match status_response {
                Ok(r) => r,
                Err(e) => {
                    debug!("Orchestra monitor: status query failed for {lookup_id}: {e}");
                    continue;
                }
            };

            debug!(
                "Orchestra monitor: payment {} order status: {:?} (amount_out={:?})",
                payment.id, status_response.order.status, status_response.order.amount_out,
            );

            let Some(updated_metadata) = apply_terminal_status(conversion_info, &status_response)
            else {
                debug!(
                    "Orchestra monitor: payment {} still in progress",
                    payment.id
                );
                continue;
            };

            debug!(
                "Orchestra monitor: payment {} terminal update built",
                payment.id
            );

            if let Err(e) = storage
                .insert_payment_metadata(payment.id.clone(), updated_metadata)
                .await
            {
                error!(
                    "Failed to update Orchestra status for payment {}: {e}",
                    payment.id
                );
            } else {
                info!(
                    "Orchestra order for payment {} reached terminal state",
                    payment.id
                );
            }
        }

        Ok(())
    }

    /// Polls Orchestra for status updates on active cross-chain receive orders.
    ///
    /// Dispatch to one of two helpers based on whether Orchestra has created an
    /// order handle yet:
    /// * No order yet → [`Self::check_receive_deposit`]: probe `/submit`
    ///   with a fresh idempotency key; a 200 means the deposit was
    ///   detected and the order handle is now stored on the row.
    /// * Order in flight → [`Self::poll_receive_order_status`]: poll
    ///   `/status` to terminal and either attach metadata (Completed) or
    ///   close (Failed / Refunded).
    async fn poll_in_flight_receives(
        storage: &Arc<dyn Storage>,
        swap_storage: &OrchestraStorageAdapter,
        client: &Arc<OrchestraClient>,
        spark_wallet: &Arc<SparkWallet>,
        fiat_service: &dyn FiatService,
    ) -> Result<(), SdkError> {
        let active = swap_storage.list_active().await?;
        debug!(
            "Orchestra monitor: found {} active receive rows",
            active.len()
        );

        for (row, data) in active {
            let quote_id = data.quote_id.clone();
            // Long-stop for unfunded quotes
            if data.order_id.is_none() && is_past_receive_grace(&data) {
                info!(
                    "Orchestra receive {quote_id}: unfunded {RECEIVE_GRACE_SECS}s past expiry, closing row"
                );
                if let Err(e) = swap_storage.mark_terminal(row).await {
                    error!(
                        "Orchestra receive {quote_id}: failed to mark unfunded row terminal: {e:?}"
                    );
                }
                continue;
            }

            let (row, data, order_id) = match data.order_id.clone() {
                Some(order_id) => (row, data, order_id),
                None => {
                    match Self::check_for_receive_deposit(swap_storage, client, row, data).await {
                        // Deposit detected
                        Ok(Some((row, data, order_id))) => (row, data, order_id),
                        Ok(None) => continue,
                        Err(e) => {
                            error!("Orchestra receive {quote_id}: deposit check failed: {e:?}");
                            continue;
                        }
                    }
                }
            };
            if let Err(e) = Self::poll_receive_order_status(
                storage,
                swap_storage,
                client,
                spark_wallet,
                fiat_service,
                row,
                data,
                &order_id,
            )
            .await
            {
                error!("Orchestra receive {quote_id}: status poll failed: {e:?}");
            }
        }

        Ok(())
    }

    /// Probe `/submit` with a fresh idempotency key to see whether
    /// Orchestra has detected the inbound deposit on the external chain.
    /// Any error (including the `invalid_tx_hash` 400 Orchestra returns
    /// before the deposit arrives) leaves the row non-terminal so the next
    /// tick retries. A 200 means the deposit was detected and Orchestra
    /// has created the order handle, which the adapter persists for the
    /// status-polling phase. Returns the updated `(row, data, order_id)`
    /// so the caller can chain straight into a status poll.
    async fn check_for_receive_deposit(
        swap_storage: &OrchestraStorageAdapter,
        client: &Arc<OrchestraClient>,
        row: crate::StoredCrossChainSwap,
        data: OrchestraSwapData,
    ) -> Result<Option<(crate::StoredCrossChainSwap, OrchestraSwapData, String)>, SdkError> {
        let quote_id = data.quote_id.clone();
        let request = flashnet::orchestra::SubmitRequest {
            quote_id: quote_id.clone(),
            spark_tx_hash: None,
            source_spark_address: None,
        };
        let idempotency_key = uuid::Uuid::new_v4().to_string();
        match client.submit(request, idempotency_key).await {
            Ok(resp) => {
                info!(
                    "Orchestra receive {quote_id}: detected deposit, orderId={}",
                    resp.order_id
                );
                let order_id = resp.order_id.clone();
                let (row, data) = swap_storage
                    .attach_order_handle(row, data, resp.order_id, resp.read_token)
                    .await?;
                Ok(Some((row, data, order_id)))
            }
            Err(e) => {
                debug!("Orchestra receive {quote_id}: no deposit yet: {e}");
                Ok(None)
            }
        }
    }

    /// Poll `/status` for an order whose handle has already been created by
    /// `check_receive_deposit`. On terminal `Completed` attach metadata to
    /// the inbound Spark Payment (or cache it if the Payment row is not
    /// visible yet); on terminal `Failed` / `Refunded` close the row with
    /// no metadata.
    #[allow(clippy::too_many_arguments)]
    async fn poll_receive_order_status(
        storage: &Arc<dyn Storage>,
        swap_storage: &OrchestraStorageAdapter,
        client: &Arc<OrchestraClient>,
        spark_wallet: &Arc<SparkWallet>,
        fiat_service: &dyn FiatService,
        row: crate::StoredCrossChainSwap,
        data: OrchestraSwapData,
        order_id: &str,
    ) -> Result<(), SdkError> {
        let quote_id = data.quote_id.clone();
        let resp = match client
            .status_by_id(order_id, data.read_token.as_deref())
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                debug!(
                    "Orchestra receive {quote_id}: status request failed for orderId={order_id}: {e}"
                );
                return Ok(());
            }
        };
        let order = resp.order;
        debug!("Orchestra receive {quote_id}: order response: {order:?}");
        match order.status {
            OrderStatus::Completed => {
                match attach_receive_metadata(storage, spark_wallet, fiat_service, &data, &order)
                    .await
                {
                    Ok(true) => {
                        info!("Orchestra receive {quote_id} → Completed, metadata attached");
                        swap_storage.mark_terminal(row).await?;
                    }
                    Ok(false) => {
                        // Order Completed but no sparkTxHash yet — no key to
                        // link or cache against. Leave row non-terminal and
                        // retry on the next tick once Orchestra populates it.
                        debug!(
                            "Orchestra receive {quote_id} Completed without sparkTxHash; will retry"
                        );
                    }
                    Err(e) => {
                        error!("Orchestra receive {quote_id} metadata attach failed: {e:?}");
                    }
                }
            }
            OrderStatus::Failed | OrderStatus::Refunded => {
                info!(
                    "Orchestra receive {quote_id} → {:?}, closing row without metadata",
                    order.status
                );
                swap_storage.mark_terminal(row).await?;
            }
            // Non-terminal status.
            _ => {}
        }
        Ok(())
    }

    /// Resolves the Orchestra-side `source_asset` wire symbol (e.g. `"BTC"`,
    /// `"USDB"`) for the given destination route + Spark source.
    ///
    /// Orchestra's `/quote` API identifies the source asset by
    /// `(sourceChain, sourceAsset)` symbols rather than contract addresses,
    /// so we look up the matching raw route and read its `source.asset`.
    /// This doubles as validation that Orchestra actually offers a route for
    /// the requested source-to-destination combination.
    ///
    /// `spark_routes()` is cache-backed (TTL'd) so this call is effectively
    /// free in the hot path.
    async fn resolve_source_asset(
        &self,
        dest: &CrossChainRoutePair,
        token_identifier: Option<&str>,
    ) -> Result<ResolvedSparkAsset, SdkError> {
        let raw_routes = self.client.spark_routes(true).await?;
        find_source_asset(&raw_routes, dest, token_identifier).ok_or_else(|| {
            SdkError::InvalidInput(format!(
                "Orchestra does not offer a route for source {} → {}/{}",
                token_identifier.unwrap_or("BTC"),
                dest.chain,
                dest.asset
            ))
        })
    }

    /// Source-units `amount` → destination-units target. BTC source uses the
    /// fiat rate; USD-stable token source rescales between decimals.
    async fn compute_target_destination_amount(
        &self,
        source_asset: &ResolvedSparkAsset,
        route: &CrossChainRoutePair,
        amount: u128,
    ) -> Result<u128, SdkError> {
        if source_asset.asset.eq_ignore_ascii_case("BTC") {
            let btc_usd = super::fetch_btc_usd_rate(self.fiat_service.as_ref()).await?;
            super::convert_sats_to_destination_amount(amount, btc_usd, route.decimals.into())
        } else if super::is_usd_stable_asset(&source_asset.asset) {
            super::rescale_decimals(amount, source_asset.decimals.into(), route.decimals.into())
        } else {
            Err(SdkError::InvalidInput(format!(
                "Cross-chain source asset not supported for inflation: {}",
                source_asset.asset
            )))
        }
    }

    /// Destination-units `target` → source-units rough deposit, used as the
    /// probe seed for Orchestra's `/estimate` on `FeesExcluded` receives.
    /// Symmetric inverse of [`Self::compute_target_destination_amount`]:
    /// BTC destination fetches the fiat rate; USD-stable destination rescales
    /// decimals at par.
    async fn compute_target_source_amount(
        &self,
        destination: &SparkAsset,
        destination_decimals: u32,
        route: &CrossChainRoutePair,
        target: u128,
    ) -> Result<u128, SdkError> {
        match destination {
            SparkAsset::Bitcoin => {
                let btc_usd = super::fetch_btc_usd_rate(self.fiat_service.as_ref()).await?;
                super::convert_sats_to_destination_amount(target, btc_usd, route.decimals.into())
            }
            SparkAsset::Token { .. } => {
                super::rescale_decimals(target, destination_decimals, route.decimals.into())
            }
        }
    }

    /// Probes the live delivery ratio via an `ExactIn` estimate, then scales
    /// `source_amount` up to deliver `destination_amount`. Floored at
    /// `source_amount`. Sends the affiliate id so the probe sees the same
    /// fee schedule the real quote will.
    ///
    /// Direction-agnostic: callers supply `source_chain` / `source_asset` and
    /// `destination_chain` / `destination_asset` explicitly. Send passes
    /// `source_chain = "spark"`; receive passes `destination_chain = "spark"`.
    ///
    /// `apply_stable_receive_buffers` shaves the raw estimate by the
    /// external-fee and grid-hop buffers before scaling. Only set on receives
    /// that land a 6-decimal USD stable on Spark: the constants assume
    /// 6-decimal units, and the cent-floor motivation for the grid-hop buffer
    /// only applies to USDB.
    #[allow(clippy::too_many_arguments)]
    async fn estimate_required_source_amount(
        &self,
        source_chain: &str,
        source_asset: &str,
        destination_chain: &str,
        destination_asset: &str,
        source_amount: u128,
        destination_amount: u128,
        apply_stable_receive_buffers: bool,
    ) -> Result<u128, SdkError> {
        let request = EstimateRequest {
            source_chain: source_chain.to_string(),
            source_asset: source_asset.to_string(),
            destination_chain: destination_chain.to_string(),
            destination_asset: destination_asset.to_string(),
            amount: source_amount.to_string(),
            amount_mode: Some(AmountMode::ExactIn),
            affiliate_id: Some(DEFAULT_AFFILIATE_ID.to_string()),
        };
        debug!(
            "Orchestra: estimating delivery ratio: {}/{} -> {}/{} source={}",
            request.source_chain,
            request.source_asset,
            request.destination_chain,
            request.destination_asset,
            request.amount
        );
        let estimate: EstimateResponse = self.client.estimate(request).await?;
        debug!("Orchestra: estimate response: {:?}", estimate);
        let delivered = parse_amount(&estimate.estimated_out, "estimatedOut")?;
        let probe_ratio_bps = delivered
            .saturating_mul(10_000)
            .checked_div(destination_amount)
            .unwrap_or(0);
        let buffer_applied =
            apply_stable_receive_buffers && probe_ratio_bps >= STABLE_RECEIVE_BUFFER_ACTIVATION_BPS;
        let adjusted_delivered = if buffer_applied {
            adjust_estimate_for_receive_buffers(delivered)?
        } else {
            delivered
        };
        let required_in =
            proportional_inflation(source_amount, destination_amount, adjusted_delivered)?;
        debug!(
            "Orchestra: estimate scaling: source_probe={source_amount} \
             estimated_delivered={delivered} adjusted_delivered={adjusted_delivered} \
             probe_ratio_bps={probe_ratio_bps} buffer_applied={buffer_applied} \
             target={destination_amount} → required_in={required_in}",
        );
        Ok(required_in)
    }
}

fn parse_amount(value: &str, field: &str) -> Result<u128, SdkError> {
    value
        .parse::<u128>()
        .map_err(|e| SdkError::Generic(format!("Orchestra returned invalid {field}: {e}")))
}

/// Shaves Orchestra's `estimated_out` by the external-fee buffer before it's
/// used to scale `required_in`. Orchestra's estimate covers their own platform
/// fees (including cent-floor rounding) but not the pass-through
/// bridge/aggregator/gas costs. See
/// [`STABLE_RECEIVE_BUFFER_BASE_UNITS`] for provenance.
fn adjust_estimate_for_receive_buffers(delivered: u128) -> Result<u128, SdkError> {
    let adjusted = delivered.saturating_sub(STABLE_RECEIVE_BUFFER_BASE_UNITS);
    if adjusted == 0 {
        return Err(SdkError::InvalidInput(
            "Cross-chain receive amount too small: Orchestra's estimate is at or below \
             the external fee floor. Increase the target."
                .to_string(),
        ));
    }
    Ok(adjusted)
}

/// Returns `source_amount * destination_amount / estimated_delivered`, floored
/// at `source_amount`. Errors on zero `estimated_delivered` or overflow.
fn proportional_inflation(
    source_amount: u128,
    destination_amount: u128,
    estimated_delivered: u128,
) -> Result<u128, SdkError> {
    if estimated_delivered == 0 {
        return Err(SdkError::Generic(
            "Cross-chain: ExactIn estimate returned zero delivered amount".to_string(),
        ));
    }
    let inflated = source_amount
        .checked_mul(destination_amount)
        .and_then(|p| p.checked_div(estimated_delivered))
        .ok_or_else(|| SdkError::Generic("Cross-chain: inflation scaling overflow".to_string()))?;
    Ok(inflated.max(source_amount))
}

/// Errors if `quoted_estimated_out` falls below `destination_amount * (1 −
/// max_slippage_bps / 10000)`.
fn verify_quote_not_drifted(
    destination_amount: u128,
    quoted_estimated_out: u128,
    max_slippage_bps: u32,
) -> Result<(), SdkError> {
    let min_acceptable = destination_amount
        .saturating_mul(u128::from(10_000u32.saturating_sub(max_slippage_bps)))
        / 10_000u128;
    if quoted_estimated_out < min_acceptable {
        // drift_bps = (target − delivered) / target × 10000. Capped at
        // 10000 so an upstream anomaly (e.g. delivered = 0) doesn't
        // overflow the log line.
        let drift_bps = destination_amount
            .saturating_sub(quoted_estimated_out)
            .saturating_mul(10_000)
            .checked_div(destination_amount)
            .unwrap_or(0);
        warn!(
            "Cross-chain quote drift: target={destination_amount} \
             delivered={quoted_estimated_out} min_acceptable={min_acceptable} \
             drift_bps={drift_bps} slippage_budget_bps={max_slippage_bps}"
        );
        return Err(SdkError::InvalidInput(format!(
            "Cross-chain quote rate drift: expected destination amount {destination_amount}, got {quoted_estimated_out}. Please re-prepare."
        )));
    }
    Ok(())
}

/// Finds the Spark-side asset of a raw route matching `(external_pair,
/// spark_asset)`. Direction picks which side of the [`Route`] is Spark:
/// `is_send = true` → Spark is `source` (and the external pair is the
/// destination); `is_send = false` → Spark is `destination`.
///
/// Match semantics:
/// - external side: exact match on `(chain, asset, contract_address)`.
/// - Spark side: BTC matches by case-insensitive asset symbol; Token
///   matches by `contract_address` equalling the token identifier (which
///   on the Spark side is the bech32m token id).
///
/// Returns the matched Spark side's wire symbol + decimals; `None` if no
/// route matches.
fn find_spark_side(
    routes: &[Route],
    external_pair: &CrossChainRoutePair,
    spark_asset: &SparkAsset,
    is_send: bool,
) -> Option<ResolvedSparkAsset> {
    routes
        .iter()
        .find(|r| {
            let (external, spark) = if is_send {
                (&r.destination, &r.source)
            } else {
                (&r.source, &r.destination)
            };
            external.chain == external_pair.chain
                && external.asset == external_pair.asset
                && external.contract_address == external_pair.contract_address
                && match spark_asset {
                    SparkAsset::Bitcoin => spark.asset.eq_ignore_ascii_case("BTC"),
                    SparkAsset::Token { token_identifier } => {
                        spark.contract_address.as_deref() == Some(token_identifier.as_str())
                    }
                }
        })
        .map(|r| {
            let spark = if is_send { &r.source } else { &r.destination };
            ResolvedSparkAsset {
                asset: spark.asset.clone(),
                decimals: spark.decimals,
            }
        })
}

/// Finds the Orchestra source asset for an outbound (Spark → external)
/// route. Thin wrapper over [`find_spark_side`] in the send direction.
/// `token_identifier == None` means BTC source; otherwise the Spark token
/// id (bech32m).
fn find_source_asset(
    routes: &[Route],
    dest: &CrossChainRoutePair,
    token_identifier: Option<&str>,
) -> Option<ResolvedSparkAsset> {
    let spark = match token_identifier {
        None => SparkAsset::Bitcoin,
        Some(tid) => SparkAsset::Token {
            token_identifier: tid.to_string(),
        },
    };
    find_spark_side(routes, dest, &spark, true)
}

/// Finds the Orchestra destination asset symbol for an inbound (external
/// → Spark) route. Thin wrapper over [`find_spark_side`] in the receive
/// direction, projecting away the decimals. Test-only — production now
/// calls `find_spark_side` directly because the receive flow needs the
/// destination's decimals too.
#[cfg(test)]
fn find_destination_asset_symbol(
    routes: &[Route],
    pair: &CrossChainRoutePair,
    destination: &SparkAsset,
) -> Option<String> {
    find_spark_side(routes, pair, destination, false).map(|r| r.asset)
}

#[macros::async_trait]
#[allow(clippy::too_many_lines)]
impl CrossChainService for OrchestraService {
    async fn get_routes(
        &self,
        filter: &CrossChainRouteFilter,
    ) -> Result<Vec<CrossChainRoutePair>, SdkError> {
        let (is_send, contract_filter, family_filter) = match filter {
            CrossChainRouteFilter::Send { address_details } => {
                let family: CrossChainAddressFamily = address_details.address_family.into();
                (
                    true,
                    address_details.contract_address.as_deref(),
                    Some(family),
                )
            }
            CrossChainRouteFilter::Receive { contract_address } => {
                (false, contract_address.as_deref(), None)
            }
        };

        let routes = self.client.spark_routes(is_send).await?;

        Ok(dedupe_routes(
            &routes,
            is_send,
            family_filter,
            contract_filter,
        ))
    }

    async fn prepare_send(
        &self,
        recipient_address: &str,
        route: &CrossChainRoutePair,
        amount: u128,
        token_identifier: Option<String>,
        max_slippage_bps: u32,
        fee_mode: CrossChainFeeMode,
    ) -> Result<CrossChainSendPrepared, SdkError> {
        let source_asset = self
            .resolve_source_asset(route, token_identifier.as_deref())
            .await?;

        // FeesExcluded inflates the source to deliver the cross-chain
        // conversion of `amount`; FeesIncluded passes `amount` through (send
        // all, recipient gets `amount − fees`).
        let (source_amount, destination_amount) = match fee_mode {
            CrossChainFeeMode::FeesIncluded => (amount, None),
            CrossChainFeeMode::FeesExcluded => {
                let destination_amount = self
                    .compute_target_destination_amount(&source_asset, route, amount)
                    .await?;
                let required_in = self
                    .estimate_required_source_amount(
                        SPARK_CHAIN,
                        &source_asset.asset,
                        &route.chain,
                        &route.asset,
                        amount,
                        destination_amount,
                        false,
                    )
                    .await?;
                (required_in, Some(destination_amount))
            }
        };

        let request = QuoteRequest {
            source_chain: SPARK_CHAIN.to_string(),
            source_asset: source_asset.asset.clone(),
            destination_chain: route.chain.clone(),
            destination_asset: route.asset.clone(),
            amount: source_amount.to_string(),
            recipient_address: recipient_address.to_string(),
            amount_mode: Some(AmountMode::ExactIn),
            refund_address: None,
            slippage_bps: Some(max_slippage_bps),
            zeroconf_enabled: None,
            app_fees: Vec::new(),
            affiliate_id: Some(DEFAULT_AFFILIATE_ID.to_string()),
        };

        debug!(
            "Orchestra: requesting quote: {}/{} -> {}/{} amount={}",
            request.source_chain,
            request.source_asset,
            request.destination_chain,
            request.destination_asset,
            request.amount
        );
        let quote: QuoteResponse = self.client.quote(request).await?;
        debug!("Orchestra: quote response: {:?}", quote);

        let amount_in = parse_amount(&quote.amount_in, "amountIn")?;
        let estimated_out = parse_amount(&quote.estimated_out, "estimatedOut")?;
        let service_fee_amount = parse_amount(&quote.total_fee_amount, "totalFeeAmount")?;

        if let Some(target) = destination_amount {
            verify_quote_not_drifted(target, estimated_out, max_slippage_bps)?;
        }

        // `amount_in` expressed in destination-asset units, via the same
        // path as `target_dest`. `fee_amount` is the gap to `estimated_out`.
        let asset_amount_in = self
            .compute_target_destination_amount(&source_asset, route, amount_in)
            .await?;
        let fee_amount = asset_amount_in.saturating_sub(estimated_out);

        let provider_context = CrossChainProviderContext::Orchestra {
            quote_id: quote.quote_id,
            deposit_address: quote.deposit_address,
            deposit_amount: amount_in,
        };

        Ok(CrossChainSendPrepared {
            amount_in,
            asset_amount_in,
            estimated_out,
            fee_amount,
            service_fee_amount,
            service_fee_asset: if quote.fee_asset.eq_ignore_ascii_case("BTC") {
                None
            } else {
                Some(quote.fee_asset)
            },
            // Source-side Spark transfer fee is 0 today.
            source_transfer_fee_sats: 0,
            fee_mode,
            expires_at: quote.expires_at,
            pair: route.clone(),
            recipient_address: recipient_address.to_string(),
            token_identifier,
            provider_context,
        })
    }

    async fn prepare_receive(
        &self,
        route: &CrossChainRoutePair,
        recipient_address: &str,
        amount: u128,
        max_slippage_bps: u32,
        destination: &SparkAsset,
        fee_mode: CrossChainFeeMode,
        target_overpay_bps: u32,
    ) -> Result<CrossChainReceivePrepared, SdkError> {
        // Resolve the destination's Spark-side wire symbol (e.g. "BTC",
        // "USDB") and decimals by reading them off the matching raw route.
        // The caller has already validated `destination` against
        // `route.spark_assets`.
        let raw_routes = self.client.spark_routes(false).await?;
        let resolved_destination = find_spark_side(&raw_routes, route, destination, false)
            .ok_or_else(|| {
                SdkError::Generic(format!(
                    "Orchestra route {}/{} has no entry matching destination {:?}",
                    route.chain, route.asset, destination
                ))
            })?;
        let destination_asset_symbol = resolved_destination.asset;
        let destination_decimals = u32::from(resolved_destination.decimals);
        let destination_token_identifier = match destination {
            SparkAsset::Bitcoin => None,
            SparkAsset::Token { token_identifier } => Some(token_identifier.clone()),
        };
        let apply_stable_buffers = super::USD_STABLE_ASSETS
            .iter()
            .any(|s| s.eq_ignore_ascii_case(&destination_asset_symbol));

        // FeesExcluded inflates the source deposit so Orchestra delivers
        // `amount` (already overpay-padded by the dispatcher) on the Spark
        // side; FeesIncluded passes `amount` through verbatim as the
        // deposit (the original receive behaviour).
        let (source_amount, target_destination_amount) = match fee_mode {
            CrossChainFeeMode::FeesIncluded => (amount, None),
            CrossChainFeeMode::FeesExcluded => {
                // Apply `target_overpay_bps` for SIZING only: Orchestra gets
                // a wider target to work with so the deposit is padded, but
                // the drift check (below) still uses `amount` — the user's
                // actual ask. Applying the pad to both would raise the
                // drift-accept threshold in lockstep with the deposit,
                // negating the overpay's purpose.
                //
                // `route.asset` is the cross-chain SOURCE on receive, and
                // the SDK's `get_cross_chain_routes` aggregator already
                // restricts it to USD-stable tickers (USDC / USDT / …), so
                // the par rescale / fiat anchor used inside the probe is
                // well-defined.
                let inflated_target = super::inflate_target_amount(amount, target_overpay_bps);
                let probe_source = self
                    .compute_target_source_amount(
                        destination,
                        destination_decimals,
                        route,
                        inflated_target,
                    )
                    .await?;
                debug!(
                    "Orchestra receive probe: destination_target={amount} \
                     inflated_target={inflated_target} → probe_source={probe_source} \
                     (destination_decimals={destination_decimals}, source_decimals={})",
                    route.decimals,
                );
                let required_in = self
                    .estimate_required_source_amount(
                        &route.chain,
                        &route.asset,
                        SPARK_CHAIN,
                        &destination_asset_symbol,
                        probe_source,
                        inflated_target,
                        apply_stable_buffers,
                    )
                    .await?;
                // Drift check uses the ORIGINAL `amount`, not `inflated_target`.
                (required_in, Some(amount))
            }
        };

        let request = QuoteRequest {
            // Routes returned from `get_routes(Receive)` describe the
            // non-Spark side, so on receive the route is the SOURCE and
            // the DESTINATION is Spark.
            source_chain: route.chain.clone(),
            source_asset: route.asset.clone(),
            destination_chain: SPARK_CHAIN.to_string(),
            destination_asset: destination_asset_symbol.clone(),
            amount: source_amount.to_string(),
            recipient_address: recipient_address.to_string(),
            // ExactIn: deposit amount is fixed (caller-picked on FeesIncluded,
            // SDK-computed on FeesExcluded); Orchestra forward-computes what
            // the receiver gets net of fees.
            amount_mode: Some(AmountMode::ExactIn),
            refund_address: None,
            slippage_bps: Some(max_slippage_bps),
            zeroconf_enabled: None,
            app_fees: Vec::new(),
            affiliate_id: Some(DEFAULT_AFFILIATE_ID.to_string()),
        };

        debug!(
            "Orchestra: requesting receive quote: {}/{} -> {}/{} amount={}",
            request.source_chain,
            request.source_asset,
            request.destination_chain,
            request.destination_asset,
            request.amount
        );
        let quote: QuoteResponse = self.client.quote(request).await?;
        debug!("Orchestra: receive quote response: {:?}", quote);

        let deposit_amount = parse_amount(&quote.amount_in, "amountIn")?;
        let quote_estimated_out = parse_amount(&quote.estimated_out, "estimatedOut")?;
        let expires_at_secs = parse_rfc3339_to_unix_seconds(&quote.expires_at)?;

        // When FeesExcluded reject the quote if Orchestra's own delivery
        // estimate drifts outside the slippage tolerance.
        if let Some(target) = target_destination_amount {
            verify_quote_not_drifted(target, quote_estimated_out, max_slippage_bps)?;
        }
        // Shave Orchestra's own delivery estimate by the external overhead
        // we model on the input side. Orchestea don't estimate the external
        // delivery costs of routing the payment.
        let shaved_expected = if apply_stable_buffers {
            quote_estimated_out.saturating_sub(STABLE_RECEIVE_BUFFER_BASE_UNITS)
        } else {
            quote_estimated_out
        };
        // Buffer sizing guarantees delivered ≥ target, so target is a floor
        // on what the receiver will actually see. Reporting the shaved value
        // when it dips below target would under-promise something the buffer
        // was designed to make impossible.
        let expected_received_amount =
            target_destination_amount.map_or(shaved_expected, |t| shaved_expected.max(t));

        let data = OrchestraSwapData {
            quote_id: quote.quote_id.clone(),
            order_id: None,
            read_token: None,
            recipient_address: recipient_address.to_string(),
            source_chain: route.chain.clone(),
            source_asset: route.asset.clone(),
            source_chain_id: route.chain_id.clone(),
            source_contract_address: route.contract_address.clone(),
            source_decimals: u32::from(route.decimals),
            destination_chain: SPARK_CHAIN.to_string(),
            destination_asset: destination_asset_symbol,
            destination_decimals,
            token_identifier: destination_token_identifier.clone(),
            amount_in: quote.amount_in.clone(),
            expected_amount_out: expected_received_amount.to_string(),
            fee_amount: Some(quote.total_fee_amount.clone()),
            expires_at: expires_at_secs,
        };

        let adapter = OrchestraStorageAdapter::new(Arc::clone(&self.storage));
        adapter.upsert(&data).await?;

        let payment_request = super::build_receive_payment_request(
            &quote.deposit_address,
            route.chain_id.as_deref(),
            route.contract_address.as_deref(),
            deposit_amount,
        )?;

        Ok(CrossChainReceivePrepared {
            payment_request,
            info: CrossChainReceiveInfo {
                deposit_address: quote.deposit_address,
                deposit_amount,
                expected_received_amount,
                token_identifier: destination_token_identifier,
                expires_at: expires_at_secs,
            },
        })
    }

    async fn send(
        &self,
        prepared: &CrossChainSendPrepared,
        idempotency_key: Option<String>,
    ) -> Result<crate::Payment, SdkError> {
        let CrossChainProviderContext::Orchestra {
            quote_id,
            deposit_address,
            deposit_amount,
        } = &prepared.provider_context
        else {
            return Err(SdkError::Generic(
                "Orchestra send called with non-Orchestra provider context".to_string(),
            ));
        };
        // Read from the context — `prepared.amount_in` may carry a user-facing
        // display value (token base units on the conversion path) instead.
        let deposit_amount = *deposit_amount;

        validate_quote_expiry(&prepared.expires_at)?;

        let transfer_id = Some(derive_btc_leg_transfer_id(
            idempotency_key.as_deref(),
            &format!("cross_chain:orchestra:{quote_id}"),
        )?);

        // Step 1: Spark transfer to the Orchestra deposit address.
        let asset_transfer = self
            .client
            .transfer_to_deposit(
                deposit_address,
                deposit_amount,
                prepared.token_identifier.as_deref(),
                transfer_id,
            )
            .await?;
        let spark_tx_hash = asset_transfer.id();
        debug!("Orchestra: deposit transfer {spark_tx_hash} sent for quote {quote_id}");

        // Step 2: Submit the deposit to Orchestra.
        // Include the source spark address for BTC transfers so Orchestra
        // can verify the deposit sender.
        let source_spark_address = if prepared.token_identifier.is_none() {
            let addr = self
                .spark_wallet
                .get_spark_address()?
                .to_address_string()
                .map_err(|e| {
                    SdkError::Generic(format!("Failed to convert Spark address to string: {e}"))
                })?;
            Some(addr)
        } else {
            None
        };
        let idempotency_key = flashnet::orchestra::derive_idempotency_key("submit", quote_id);
        let submit_res: Result<SubmitResponse, _> = self
            .client
            .submit(
                flashnet::orchestra::SubmitRequest {
                    quote_id: quote_id.clone(),
                    spark_tx_hash: Some(spark_tx_hash.clone()),
                    source_spark_address,
                },
                idempotency_key,
            )
            .await;
        debug!("Orchestra: submit response: {:?}", submit_res);

        // Step 3: Persist ConversionInfo::Orchestra metadata.
        let (status, order_id, read_token) = match &submit_res {
            Ok(response) => (
                ConversionStatus::Pending,
                response.order_id.clone(),
                response.read_token.clone(),
            ),
            Err(e) => {
                error!("Orchestra /submit failed after deposit transfer {spark_tx_hash}: {e}");
                (ConversionStatus::RefundNeeded, String::new(), None)
            }
        };

        let conversion_info = ConversionInfo::Orchestra {
            order_id: order_id.clone(),
            quote_id: quote_id.clone(),
            chain: prepared.pair.chain.clone(),
            chain_id: prepared.pair.chain_id.clone(),
            asset: prepared.pair.asset.clone(),
            recipient_address: prepared.recipient_address.clone(),
            asset_amount_in: Some(prepared.asset_amount_in),
            estimated_out: prepared.estimated_out,
            delivered_amount: None,
            status,
            fee_amount: Some(prepared.fee_amount),
            service_fee_amount: Some(prepared.service_fee_amount),
            service_fee_asset: prepared.service_fee_asset.clone(),
            read_token,
            asset_decimals: u32::from(prepared.pair.decimals),
            asset_contract: prepared.pair.contract_address.clone(),
        };
        let metadata = crate::PaymentMetadata {
            conversion_info: Some(conversion_info.clone()),
            ..Default::default()
        };

        let payment_id = crate::utils::conversions::resolve_and_insert_payment_metadata_for_transfer(
            &asset_transfer,
            metadata,
            &self.spark_wallet,
            &self.storage,
            true,
        )
        .await
        .unwrap_or_else(|e| {
            // Reached only when both the row insert and the cache fallback
            // inside the helper failed, so the ConversionInfo is unrecoverable.
            error!(
                "Failed to persist or cache Orchestra metadata for payment {spark_tx_hash}: {e:?}"
            );
            spark_tx_hash
        });

        self.trigger_monitor();

        // Surface a submit error before kicking off polling.
        let submit_response = submit_res?;
        let order_id = submit_response.order_id;

        // Poll the outbound Spark transfer until it settles to terminal status.
        let schedule = PollSchedule {
            initial_delay: Duration::from_millis(SEND_POLL_INITIAL_DELAY_MS),
            max_delay: Duration::from_millis(SEND_POLL_MAX_DELAY_MS),
            timeout: Duration::from_secs(SEND_POLL_TIMEOUT_SECS),
        };
        let storage = Arc::clone(&self.storage);
        let spark_wallet = self.spark_wallet.clone();
        let payment_id_for_poll = payment_id.clone();
        let polled = poll_until(schedule, None, || {
            fetch_and_process_payment(
                spark_wallet.as_ref(),
                Arc::clone(&storage),
                &payment_id_for_poll,
                false,
            )
        })
        .await;

        match polled {
            Ok(payment) => Ok(payment),
            Err(e) => {
                // Operator sync still in flight — the metadata is already
                // cached, and `poll_in_flight_sends` will reconcile the
                // payment row as soon as it lands. Surface a payment built
                // from the deposit transfer (with the Orchestra
                // `ConversionInfo` attached) so callers see the send as
                // submitted rather than failed.
                debug!(
                    "Orchestra: payment row not yet visible (order {order_id}): {e}; returning fallback payment built from the deposit transfer"
                );
                let payment = crate::utils::conversions::payment_from_asset_transfer(
                    asset_transfer,
                    &self.spark_wallet,
                    &self.storage,
                    &payment_id,
                )
                .await?
                .ok_or_else(|| {
                    SdkError::Generic(format!(
                        "Orchestra transfer produced no outgoing payment for {payment_id}"
                    ))
                })?;
                Ok(payment_with_orchestra_info(payment, Some(conversion_info)))
            }
        }
    }
}

/// Returns the route side opposite the Spark wallet — destination for sends,
/// source for receives.
fn non_spark_side(r: &Route, is_send: bool) -> &RouteAsset {
    if is_send { &r.destination } else { &r.source }
}

/// Attaches the Orchestra [`ConversionInfo`] to a freshly-converted
/// [`Payment`]. The payment's top-level `status` is left as-is — it reflects
/// the local Spark/Token transfer settlement, while the cross-chain pending
/// state lives inside `conversion_info.status`. Lightning / Withdraw /
/// Deposit details pass through unchanged (they shouldn't occur on the
/// Orchestra send path; this is defensive).
fn payment_with_orchestra_info(
    mut payment: crate::Payment,
    conversion_info: Option<ConversionInfo>,
) -> crate::Payment {
    payment.details = match payment.details {
        Some(PaymentDetails::Spark {
            invoice_details,
            htlc_details,
            ..
        }) => Some(PaymentDetails::Spark {
            invoice_details,
            htlc_details,
            conversion_info,
        }),
        Some(PaymentDetails::Token {
            metadata,
            tx_hash,
            tx_type,
            invoice_details,
            ..
        }) => Some(PaymentDetails::Token {
            metadata,
            tx_hash,
            tx_type,
            invoice_details,
            conversion_info,
        }),
        other => other,
    };
    payment
}

/// Whether a raw Orchestra route should appear in the deduplicated list,
/// given the caller's address-family and contract-address filters.
///
/// Both filters operate on the non-Spark side of the route:
/// - `family_filter` restricts to routes whose chain/contract matches the
///   address family (e.g. EVM, Solana).
/// - `contract_filter` restricts to routes whose contract address equals
///   the supplied value.
///
/// `None` for either filter is a pass-through.
fn route_passes_filters(
    r: &Route,
    is_send: bool,
    family_filter: Option<CrossChainAddressFamily>,
    contract_filter: Option<&str>,
) -> bool {
    let side = non_spark_side(r, is_send);
    let contract = side.contract_address.as_deref();
    let family_ok = family_filter.is_none_or(|f| f.matches_chain(&side.chain, contract));
    let contract_ok = contract_filter.is_none_or(|wanted| contract == Some(wanted));
    family_ok && contract_ok
}

/// Returns the updated [`PaymentMetadata`] for an Orchestra order that has
/// reached terminal state, or `None` if it hasn't. `Completed` → Completed,
/// `Refunded` → Refunded, anything else terminal → Failed. `delivered_amount`
/// comes from `status_response.order.amount_out` when present.
fn apply_terminal_status(
    info: &ConversionInfo,
    status_response: &StatusResponse,
) -> Option<crate::PaymentMetadata> {
    let ConversionInfo::Orchestra {
        order_id,
        quote_id,
        chain,
        chain_id,
        asset,
        recipient_address,
        asset_amount_in,
        estimated_out,
        fee_amount,
        service_fee_amount,
        service_fee_asset,
        read_token,
        asset_decimals,
        asset_contract,
        ..
    } = info
    else {
        return None;
    };

    let order_status = status_response.order.status;
    if !order_status.is_terminal() {
        return None;
    }
    let new_status = match order_status {
        OrderStatus::Completed => ConversionStatus::Completed,
        OrderStatus::Refunded => ConversionStatus::Refunded,
        _ => ConversionStatus::Failed,
    };

    let delivered_amount = status_response
        .order
        .amount_out
        .as_deref()
        .and_then(|s| s.parse::<u128>().ok());

    let updated_fee_amount = super::compute_terminal_fee_amount(
        &new_status,
        *asset_amount_in,
        delivered_amount,
        *fee_amount,
    );

    Some(crate::PaymentMetadata {
        conversion_info: Some(ConversionInfo::Orchestra {
            order_id: order_id.clone(),
            quote_id: quote_id.clone(),
            chain: chain.clone(),
            chain_id: chain_id.clone(),
            asset: asset.clone(),
            recipient_address: recipient_address.clone(),
            asset_amount_in: *asset_amount_in,
            estimated_out: *estimated_out,
            delivered_amount,
            status: new_status,
            fee_amount: updated_fee_amount,
            service_fee_amount: *service_fee_amount,
            service_fee_asset: service_fee_asset.clone(),
            read_token: read_token.clone(),
            asset_decimals: *asset_decimals,
            asset_contract: asset_contract.clone(),
        }),
        ..Default::default()
    })
}

/// Reads `order.sparkTxHash` (the linking key on receive), resolves it to the
/// inbound Spark `Payment` id, and upserts `ConversionInfo::Orchestra` onto it.
/// `spark_tx_hash` may be a Spark transfer id (BTC receive) or a token tx hash
/// (USDB receive — token payment ids carry a `:vout` suffix that `spark_tx_hash`
/// alone doesn't), so the resolution is delegated to `resolve_and_insert_payment_metadata`,
/// which also caches the metadata if the Payment row hasn't been synced yet.
///
/// Returns `Ok(false)` when the order is `Completed` but `spark_tx_hash` is
/// not yet populated — there's nothing to link or even cache against, so the
/// caller leaves the row non-terminal for the next tick. `Ok(true)` means
/// metadata was either attached or cached for sync to reapply.
async fn attach_receive_metadata(
    storage: &Arc<dyn Storage>,
    spark_wallet: &SparkWallet,
    fiat_service: &dyn FiatService,
    data: &OrchestraSwapData,
    order: &Order,
) -> Result<bool, SdkError> {
    let Some(spark_tx_hash) = order.spark_tx_hash.as_deref() else {
        debug!(
            "Orchestra receive {}: order Completed but no sparkTxHash yet",
            data.quote_id
        );
        return Ok(false);
    };
    let conversion_info = build_orchestra_receive_conversion_info(data, order, fiat_service).await;
    let metadata = crate::PaymentMetadata {
        conversion_info: Some(conversion_info),
        ..Default::default()
    };
    // tx_inputs_are_ours = false: on receive, the inbound token tx is funded
    // by Orchestra's counterparty, not us.
    resolve_and_insert_payment_metadata(spark_tx_hash, metadata, spark_wallet, storage, false)
        .await?;
    Ok(true)
}

/// Mirrors the send-side helper [`apply_terminal_status`] for receive: pulls
/// the live-ish bits from the `Order` and the quote-time bits from the
/// stashed `OrchestraSwapData`. `chain/asset` describes the non-Spark side
/// (the source on receive) so the SDK's existing UI surface renders symmetric
/// to send.
///
/// Prefers `order.amount_in` (what the sender actually deposited) over the
/// quote-time `data.amount_in` (the requested deposit). They differ when a
/// late deposit gets repriced or the sender rounds.
///
/// Fee is computed in source-asset base units:
/// - **USDB destination**: `amount_in − rescale_decimals(amount_out, dst, src)`.
///   USDC/USDT and USDB are all USD-stable ~1:1, so rescaling the destination
///   amount into source-asset decimals before subtracting is the realized
///   fee. The rescale is the identity when both sides share decimals.
/// - **BTC destination**: `amount_in − sats_to_source(amount_out)`, using the
///   live BTC/USD rate to convert delivered sats into source-asset units.
/// - **Fallback**: the quote-time `data.fee_amount` if either input is
///   missing or the fiat lookup fails.
async fn build_orchestra_receive_conversion_info(
    data: &OrchestraSwapData,
    order: &Order,
    fiat_service: &dyn FiatService,
) -> ConversionInfo {
    let asset_amount_in = order
        .amount_in
        .as_deref()
        .and_then(|s| s.parse::<u128>().ok())
        .or_else(|| data.amount_in.parse::<u128>().ok());
    let estimated_out = data.expected_amount_out.parse::<u128>().unwrap_or(0);
    let delivered_amount = order
        .amount_out
        .as_deref()
        .and_then(|s| s.parse::<u128>().ok());
    let quote_fee_amount = data
        .fee_amount
        .as_deref()
        .and_then(|s| s.parse::<u128>().ok());

    let fee_amount = compute_receive_fee(data, asset_amount_in, delivered_amount, fiat_service)
        .await
        .or(quote_fee_amount);

    ConversionInfo::Orchestra {
        order_id: order.id.clone(),
        quote_id: data.quote_id.clone(),
        read_token: None,
        chain: data.source_chain.clone(),
        chain_id: data.source_chain_id.clone(),
        asset: data.source_asset.clone(),
        recipient_address: data.recipient_address.clone(),
        asset_amount_in,
        estimated_out,
        delivered_amount,
        status: ConversionStatus::Completed,
        // Realized total fee in source-asset units.
        fee_amount,
        service_fee_amount: quote_fee_amount,
        service_fee_asset: Some(data.source_asset.clone()),
        asset_decimals: data.source_decimals,
        asset_contract: data.source_contract_address.clone(),
    }
}

/// Whether a pre-order receive row is past the local grace window past
/// `expires_at`. Used by the long-stop in `poll_in_flight_receives` so
/// quotes that never get funded eventually stop generating /submit traffic.
fn is_past_receive_grace(data: &OrchestraSwapData) -> bool {
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    now_secs >= data.expires_at.saturating_add(RECEIVE_GRACE_SECS)
}

/// Computes the realized cross-chain receive fee in source-asset base units.
/// Returns `None` if any input is missing, the fiat lookup fails, or the
/// converted delivered amount exceeds the deposit (numerical drift in
/// sats→USD conversion, repricing oddities, a stale fiat rate). The caller
/// falls back to the quote-time estimate in any of these cases.
async fn compute_receive_fee(
    data: &OrchestraSwapData,
    asset_amount_in: Option<u128>,
    delivered_amount: Option<u128>,
    fiat_service: &dyn FiatService,
) -> Option<u128> {
    let amount_in = asset_amount_in?;
    let amount_out = delivered_amount?;
    let delivered_in_source = if data.token_identifier.is_some() {
        // Token destination (USDB): source and destination are both USD-stable.
        // Rescale destination units to source-asset decimals and subtract.
        super::rescale_decimals(amount_out, data.destination_decimals, data.source_decimals).ok()?
    } else {
        // BTC destination (sats): convert sats to source-asset units via the
        // BTC/USD rate, then subtract.
        let btc_usd = super::fetch_btc_usd_rate(fiat_service).await.ok()?;
        super::convert_sats_to_destination_amount(amount_out, btc_usd, data.source_decimals).ok()?
    };
    amount_in.checked_sub(delivered_in_source)
}

/// Parses Orchestra's RFC3339 `expires_at` into unix seconds.
fn parse_rfc3339_to_unix_seconds(expires_at: &str) -> Result<u64, SdkError> {
    let exp = DateTime::parse_from_rfc3339(expires_at).map_err(|e| {
        SdkError::Generic(format!("Orchestra: invalid expires_at {expires_at:?}: {e}"))
    })?;
    Ok(u64::try_from(exp.timestamp()).unwrap_or(0))
}

/// Rejects an expired quote at send time so the caller can re-prepare
/// instead of getting a less helpful error from `/submit`.
fn validate_quote_expiry(expires_at: &str) -> Result<(), SdkError> {
    let exp_secs = parse_rfc3339_to_unix_seconds(expires_at)?;
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| SdkError::Generic("Failed to read current time".to_string()))?
        .as_secs();
    if now_secs >= exp_secs {
        return Err(SdkError::InvalidInput(
            "Cross-chain quote has expired. Please re-prepare.".to_string(),
        ));
    }
    Ok(())
}

/// Dedupes Orchestra's raw `Route` list into the SDK's [`CrossChainRoutePair`]
/// shape — one pair per `(chain, asset, contract_address)` endpoint with the
/// supported Spark-side source variants accumulated into `spark_assets`.
///
/// Multiple raw routes can exist for the same external chain (e.g.
/// `BTC→USDT-on-tron` and `USDB→USDT-on-tron`); the caller wants to see one
/// `USDT-on-tron` route advertising both source variants.
fn dedupe_routes(
    routes: &[Route],
    is_send: bool,
    family_filter: Option<CrossChainAddressFamily>,
    contract_filter: Option<&str>,
) -> Vec<CrossChainRoutePair> {
    type Key = (String, String, Option<String>);
    let mut order: Vec<Key> = Vec::new();
    let mut grouped: HashMap<Key, CrossChainRoutePair> = HashMap::new();

    for r in routes
        .iter()
        .filter(|r| route_passes_filters(r, is_send, family_filter, contract_filter))
    {
        let side = non_spark_side(r, is_send);
        let key: Key = (
            side.chain.clone(),
            side.asset.clone(),
            side.contract_address.clone(),
        );

        // On send, the Spark side is `source`; on receive, it's `destination`.
        // Orchestra's `contract_address` on the Spark side is the bech32m
        // token identifier (`btkn1...`).
        let spark_side = if is_send { &r.source } else { &r.destination };
        let source_variant = if spark_side.asset.eq_ignore_ascii_case("BTC") {
            Some(SparkAsset::Bitcoin)
        } else {
            // Non-BTC Spark source without a token identifier: defensive skip.
            // Shouldn't happen per current Orchestra behavior.
            spark_side
                .contract_address
                .as_ref()
                .map(|tid| SparkAsset::Token {
                    token_identifier: tid.clone(),
                })
        };

        let entry = grouped.entry(key.clone()).or_insert_with(|| {
            order.push(key.clone());
            side_to_route_pair(side, r.exact_out_eligible)
        });

        if let Some(variant) = source_variant
            && !entry.spark_assets.contains(&variant)
        {
            entry.spark_assets.push(variant);
        }
    }

    order
        .into_iter()
        .filter_map(|k| grouped.remove(&k))
        .collect()
}

/// Build a [`CrossChainRoutePair`] from one side of an Orchestra [`Route`].
///
/// Chain/asset/identifier/decimals pass through verbatim from the route's
/// non-Spark side — `chain_id` is surfaced so downstream consumers can
/// disambiguate same-name chains.
fn side_to_route_pair(side: &RouteAsset, exact_out_eligible: bool) -> CrossChainRoutePair {
    CrossChainRoutePair {
        provider: CrossChainProvider::Orchestra,
        chain: side.chain.clone(),
        chain_id: side.chain_id.clone(),
        asset: side.asset.clone(),
        contract_address: side.contract_address.clone(),
        decimals: side.decimals,
        exact_out_eligible,
        spark_assets: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use breez_sdk_common::error::ServiceConnectivityError;
    use breez_sdk_common::fiat::{FiatCurrency, Rate};

    use super::*;
    use macros::{async_test_all, test_all};

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    /// A `FiatService` that fails every call. The receive-fee builder uses
    /// it to exercise the quote-time fallback path.
    struct FailingFiat;

    #[macros::async_trait]
    impl FiatService for FailingFiat {
        async fn fetch_fiat_currencies(
            &self,
        ) -> Result<Vec<FiatCurrency>, ServiceConnectivityError> {
            Err(ServiceConnectivityError::Other("not used".to_string()))
        }
        async fn fetch_fiat_rates(&self) -> Result<Vec<Rate>, ServiceConnectivityError> {
            Err(ServiceConnectivityError::Other("upstream down".to_string()))
        }
    }

    #[async_test_all]
    async fn build_receive_conversion_info_pulls_quote_time_and_live_fields() {
        let data = OrchestraSwapData {
            quote_id: "q_xyz".to_string(),
            order_id: Some("ord_xyz".to_string()),
            read_token: Some("rt_xyz".to_string()),
            recipient_address: "sp1rcv".to_string(),
            source_chain: "ethereum".to_string(),
            source_asset: "USDC".to_string(),
            source_chain_id: Some("1".to_string()),
            source_contract_address: Some("0xUSDC".to_string()),
            source_decimals: 6,
            destination_chain: "spark".to_string(),
            destination_asset: "BTC".to_string(),
            destination_decimals: 8,
            token_identifier: None,
            amount_in: "100".to_string(),             // quote-time
            expected_amount_out: "50000".to_string(), // quote-time
            fee_amount: Some("250".to_string()),      // quote-time
            expires_at: 1_700_000_120,
        };
        let order = Order {
            id: "ord_xyz".to_string(),
            status: OrderStatus::Completed,
            kind: Some("order".to_string()),
            quote_id: Some("q_xyz".to_string()),
            source_chain: None,
            source_asset: None,
            source_address: None,
            source_tx_hash: Some("0xeth-tx".to_string()),
            source_tx_vout: None,
            sweep_tx_hash: None,
            destination_chain: None,
            destination_asset: None,
            destination_address: None,
            destination_tx_hash: None,
            deposit_address: None,
            recipient_address: None,
            amount_in: None,
            amount_out: Some("49500".to_string()), // live
            amount_fiat_usd: None,
            amount_fiat_currency: None,
            spot_usd_per_btc: None,
            fee_bps: None,
            fee_amount: None,
            fee_asset: None,
            rounding_fee_amount: None,
            slippage_bps: None,
            flashnet_request_id: None,
            spark_tx_hash: Some("spark-tx-hash".to_string()),
            refund_asset: None,
            refund_amount: None,
            refund_tx_hash: None,
            error_code: None,
            error_message: None,
            total_fee_bps: None,
            total_fee_amount: None,
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
            completed_at: None,
        };
        let info = build_orchestra_receive_conversion_info(&data, &order, &FailingFiat).await;
        match info {
            ConversionInfo::Orchestra {
                order_id,
                quote_id,
                chain,
                asset,
                recipient_address,
                asset_amount_in,
                estimated_out,
                delivered_amount,
                status,
                ..
            } => {
                assert_eq!(order_id, "ord_xyz");
                assert_eq!(quote_id, "q_xyz");
                // chain/asset describe the NON-Spark side (source on receive).
                assert_eq!(chain, "ethereum");
                assert_eq!(asset, "USDC");
                assert_eq!(recipient_address, "sp1rcv");
                assert_eq!(asset_amount_in, Some(100));
                assert_eq!(estimated_out, 50_000);
                assert_eq!(delivered_amount, Some(49_500));
                assert_eq!(status, ConversionStatus::Completed);
            }
            _ => panic!("expected Orchestra variant"),
        }
    }

    /// Future RFC3339 round-trips to the exact unix timestamp. The fixed
    /// value pins the conversion semantics so a silent drift (e.g. a TZ
    /// regression) surfaces here, not as UI bugs downstream.
    #[test_all]
    fn parse_rfc3339_to_unix_seconds_accepts_future_timestamps() {
        let ts = parse_rfc3339_to_unix_seconds("2099-01-01T00:00:00Z").unwrap();
        assert_eq!(ts, 4_070_908_800);
    }

    /// Malformed input surfaces as a descriptive error rather than a panic
    /// or a silent zero — the receive flow surfaces this to the integrator.
    #[test_all]
    fn parse_rfc3339_to_unix_seconds_rejects_malformed_input() {
        let err =
            parse_rfc3339_to_unix_seconds("not-a-date").expect_err("malformed input must fail");
        match err {
            SdkError::Generic(msg) => assert!(msg.contains("invalid expires_at"), "{msg}"),
            other => panic!("expected Generic, got {other:?}"),
        }
    }

    fn test_route_asset(chain: &str, chain_id: Option<&str>) -> RouteAsset {
        RouteAsset {
            chain: chain.to_string(),
            asset: "USDC".to_string(),
            contract_address: Some("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string()),
            decimals: 6,
            chain_id: chain_id.map(str::to_string),
        }
    }

    #[test_all]
    fn side_to_pair_passes_through_chain_id() {
        let side = test_route_asset("base", Some("8453"));
        let pair = side_to_route_pair(&side, true);

        assert_eq!(pair.provider, CrossChainProvider::Orchestra);
        assert_eq!(pair.chain, "base");
        assert_eq!(
            pair.chain_id,
            Some("8453".to_string()),
            "chain_id on the route asset should propagate to the pair"
        );
        assert_eq!(pair.asset, "USDC");
        assert_eq!(
            pair.contract_address.as_deref(),
            Some("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48")
        );
        assert_eq!(pair.decimals, 6);
        assert!(pair.exact_out_eligible);
    }

    #[test_all]
    fn side_to_pair_preserves_missing_chain_id() {
        let side = test_route_asset("solana", None);
        let pair = side_to_route_pair(&side, false);

        assert_eq!(
            pair.chain_id, None,
            "chain_id stays None when the route asset doesn't expose one"
        );
        assert!(!pair.exact_out_eligible);
    }

    // ---- dedupe_routes ----

    fn ra(chain: &str, asset: &str, contract: Option<&str>) -> RouteAsset {
        RouteAsset {
            chain: chain.to_string(),
            asset: asset.to_string(),
            contract_address: contract.map(str::to_string),
            decimals: 6,
            chain_id: None,
        }
    }

    fn route(source: RouteAsset, destination: RouteAsset) -> Route {
        Route {
            source_chain: source.chain.clone(),
            source_asset: source.asset.clone(),
            destination_chain: destination.chain.clone(),
            destination_asset: destination.asset.clone(),
            exact_out_eligible: false,
            source,
            destination,
        }
    }

    #[test_all]
    fn dedupe_routes_accumulates_source_variants() {
        // Same external endpoint (tron/USDT) fronted by two Spark sources
        // (BTC and a USDB token). Caller should see one pair with both
        // variants in `spark_assets`.
        let usdb_contract = "btkn1usdb_contract";
        let routes = vec![
            route(
                ra("spark", "BTC", None),
                ra("tron", "USDT", Some("TXYZtronUsdt")),
            ),
            route(
                ra("spark", "USDB", Some(usdb_contract)),
                ra("tron", "USDT", Some("TXYZtronUsdt")),
            ),
        ];

        let pairs = dedupe_routes(&routes, true, None, None);

        assert_eq!(
            pairs.len(),
            1,
            "same external endpoint must dedup to one pair"
        );
        let p = &pairs[0];
        assert_eq!(p.chain, "tron");
        assert_eq!(p.asset, "USDT");
        assert!(p.spark_assets.contains(&SparkAsset::Bitcoin));
        assert!(p.spark_assets.contains(&SparkAsset::Token {
            token_identifier: usdb_contract.to_string(),
        }));
    }

    #[test_all]
    fn dedupe_routes_separates_different_endpoints() {
        let routes = vec![
            route(ra("spark", "BTC", None), ra("tron", "USDT", Some("TXYZ1"))),
            route(ra("spark", "BTC", None), ra("base", "USDC", Some("0xABC"))),
        ];

        let pairs = dedupe_routes(&routes, true, None, None);

        assert_eq!(pairs.len(), 2);
        // Insertion order preserved.
        assert_eq!(pairs[0].chain, "tron");
        assert_eq!(pairs[0].asset, "USDT");
        assert_eq!(pairs[1].chain, "base");
        assert_eq!(pairs[1].asset, "USDC");
    }

    #[test_all]
    fn dedupe_routes_applies_contract_filter() {
        let routes = vec![
            route(ra("spark", "BTC", None), ra("base", "USDC", Some("0xAAA"))),
            route(ra("spark", "BTC", None), ra("base", "USDC", Some("0xBBB"))),
        ];

        let pairs = dedupe_routes(&routes, true, None, Some("0xBBB"));

        assert_eq!(pairs.len(), 1, "contract filter narrows the result set");
        assert_eq!(pairs[0].contract_address.as_deref(), Some("0xBBB"));
    }

    #[test_all]
    fn dedupe_routes_receive_swaps_spark_side() {
        // For receives, the non-Spark side is the *source* and the Spark
        // side is the *destination*. The same dedup logic should group
        // by the source side.
        let routes = vec![
            route(ra("base", "USDC", Some("0xABC")), ra("spark", "BTC", None)),
            route(
                ra("base", "USDC", Some("0xABC")),
                ra("spark", "USDB", Some("btkn1usdb")),
            ),
        ];

        let pairs = dedupe_routes(&routes, false, None, None);

        assert_eq!(pairs.len(), 1, "receive dedup groups by source side");
        assert_eq!(pairs[0].chain, "base");
        assert!(pairs[0].spark_assets.contains(&SparkAsset::Bitcoin));
        assert!(pairs[0].spark_assets.contains(&SparkAsset::Token {
            token_identifier: "btkn1usdb".to_string(),
        }));
    }

    // ---- route_passes_filters ----

    #[test_all]
    fn route_passes_filters_accepts_when_both_filters_none() {
        let r = route(ra("spark", "BTC", None), ra("base", "USDC", Some("0xAAA")));
        assert!(route_passes_filters(&r, true, None, None));
    }

    #[test_all]
    fn route_passes_filters_contract_filter_rejects_mismatch() {
        let r = route(ra("spark", "BTC", None), ra("base", "USDC", Some("0xAAA")));
        assert!(!route_passes_filters(&r, true, None, Some("0xBBB")));
        assert!(route_passes_filters(&r, true, None, Some("0xAAA")));
    }

    #[test_all]
    fn route_passes_filters_family_filter_evm_matches_via_contract_address() {
        // EVM family matches when the contract_address parses as EVM hex.
        let r = route(
            ra("spark", "BTC", None),
            ra(
                "arbitrum",
                "USDT",
                Some("0x1234567890123456789012345678901234567890"),
            ),
        );
        assert!(route_passes_filters(
            &r,
            true,
            Some(CrossChainAddressFamily::Evm),
            None
        ));
    }

    #[test_all]
    fn route_passes_filters_family_filter_rejects_wrong_family() {
        // Tron chain shouldn't match Solana family.
        let r = route(
            ra("spark", "BTC", None),
            ra("tron", "USDT", Some("TXYZtronUsdt")),
        );
        assert!(!route_passes_filters(
            &r,
            true,
            Some(CrossChainAddressFamily::Solana),
            None
        ));
    }

    #[test_all]
    fn route_passes_filters_both_filters_must_match() {
        let r = route(
            ra("spark", "BTC", None),
            ra(
                "arbitrum",
                "USDT",
                Some("0x1234567890123456789012345678901234567890"),
            ),
        );
        // Family matches but contract doesn't → reject.
        assert!(!route_passes_filters(
            &r,
            true,
            Some(CrossChainAddressFamily::Evm),
            Some("0xdeadbeef")
        ));
        // Both match → accept.
        assert!(route_passes_filters(
            &r,
            true,
            Some(CrossChainAddressFamily::Evm),
            Some("0x1234567890123456789012345678901234567890")
        ));
    }

    // ---- with_orchestra_info ----

    fn dummy_payment(method: crate::PaymentMethod, details: PaymentDetails) -> crate::Payment {
        crate::Payment {
            id: "p1".to_string(),
            payment_type: crate::PaymentType::Send,
            status: crate::PaymentStatus::Completed,
            amount: 1_000,
            fees: 0,
            timestamp: 100,
            method,
            details: Some(details),
            conversion_details: None,
        }
    }

    #[test_all]
    fn with_orchestra_info_injects_into_spark_details_and_preserves_status() {
        let original_details = PaymentDetails::Spark {
            invoice_details: None,
            htlc_details: None,
            conversion_info: None,
        };
        let payment = dummy_payment(crate::PaymentMethod::Spark, original_details);
        let info = orchestra_info("ord1", "q1");

        let out = payment_with_orchestra_info(payment, Some(info));

        // Status reflects the local Spark transfer (already settled by the
        // time we reach the fallback); cross-chain pending lives in
        // conversion_info.status.
        assert_eq!(out.status, crate::PaymentStatus::Completed);
        assert!(matches!(
            out.details,
            Some(PaymentDetails::Spark {
                conversion_info: Some(ConversionInfo::Orchestra { .. }),
                ..
            })
        ));
    }

    #[test_all]
    fn with_orchestra_info_preserves_spark_invoice_and_htlc_details() {
        // Defensive: invoice_details / htlc_details on Spark payments must
        // not be wiped by the override.
        let original_details = PaymentDetails::Spark {
            invoice_details: Some(crate::SparkInvoicePaymentDetails {
                description: Some("preserved".to_string()),
                invoice: "inv".to_string(),
            }),
            htlc_details: None,
            conversion_info: None,
        };
        let payment = dummy_payment(crate::PaymentMethod::Spark, original_details);

        let out = payment_with_orchestra_info(payment, None);

        if let Some(PaymentDetails::Spark {
            invoice_details, ..
        }) = out.details
        {
            assert_eq!(
                invoice_details.and_then(|d| d.description).as_deref(),
                Some("preserved")
            );
        } else {
            panic!("expected Spark details");
        }
    }

    #[test_all]
    fn with_orchestra_info_injects_into_token_details_and_preserves_metadata() {
        let original_details = PaymentDetails::Token {
            metadata: crate::TokenMetadata {
                identifier: "btkn1usdb".to_string(),
                issuer_public_key: "issuer".to_string(),
                name: "Bitcoin USD".to_string(),
                ticker: "USDB".to_string(),
                decimals: 6,
                max_supply: 0,
                is_freezable: true,
            },
            tx_hash: "hash".to_string(),
            tx_type: crate::TokenTransactionType::Transfer,
            invoice_details: None,
            conversion_info: None,
        };
        let payment = dummy_payment(crate::PaymentMethod::Token, original_details);
        let info = orchestra_info("ord1", "q1");

        let out = payment_with_orchestra_info(payment, Some(info));

        // Top-level status reflects the local Token transfer.
        assert_eq!(out.status, crate::PaymentStatus::Completed);
        if let Some(PaymentDetails::Token {
            metadata,
            conversion_info,
            ..
        }) = out.details
        {
            // Real metadata fetched via the shared helper is preserved.
            assert_eq!(metadata.ticker, "USDB");
            assert_eq!(metadata.decimals, 6);
            assert!(matches!(
                conversion_info,
                Some(ConversionInfo::Orchestra { .. })
            ));
        } else {
            panic!("expected Token details");
        }
    }

    // ---- apply_terminal_status ----

    fn orchestra_info(order_id: &str, quote_id: &str) -> ConversionInfo {
        ConversionInfo::Orchestra {
            order_id: order_id.to_string(),
            quote_id: quote_id.to_string(),
            chain: "base".to_string(),
            chain_id: Some("8453".to_string()),
            asset: "USDC".to_string(),
            recipient_address: "0xabc".to_string(),
            asset_amount_in: Some(1_010_000),
            estimated_out: 1_000_000,
            delivered_amount: None,
            status: ConversionStatus::Pending,
            fee_amount: Some(10_000),
            service_fee_amount: Some(50),
            service_fee_asset: Some("USDC".to_string()),
            read_token: Some("rt_token".to_string()),
            asset_decimals: 6,
            asset_contract: Some("0xUSDC".to_string()),
        }
    }

    fn status_response(status: OrderStatus, amount_out: Option<&str>) -> StatusResponse {
        StatusResponse {
            order: flashnet::orchestra::Order {
                id: "ord1".to_string(),
                status,
                kind: Some("order".to_string()),
                quote_id: Some("q1".to_string()),
                source_chain: Some("spark".to_string()),
                source_asset: Some("BTC".to_string()),
                source_address: None,
                source_tx_hash: Some("txh".to_string()),
                source_tx_vout: None,
                sweep_tx_hash: None,
                deposit_address: Some("dep".to_string()),
                destination_chain: Some("base".to_string()),
                destination_asset: Some("USDC".to_string()),
                destination_address: None,
                destination_tx_hash: None,
                recipient_address: Some("0xabc".to_string()),
                amount_in: Some("1000".to_string()),
                amount_out: amount_out.map(str::to_string),
                amount_fiat_usd: None,
                amount_fiat_currency: None,
                spot_usd_per_btc: None,
                fee_bps: Some(50),
                fee_amount: Some("50".to_string()),
                fee_asset: None,
                rounding_fee_amount: None,
                slippage_bps: Some(100),
                flashnet_request_id: None,
                spark_tx_hash: None,
                refund_asset: None,
                refund_amount: None,
                refund_tx_hash: None,
                error_code: None,
                error_message: None,
                total_fee_bps: None,
                total_fee_amount: None,
                created_at: "0".to_string(),
                updated_at: "0".to_string(),
                completed_at: None,
            },
            stages: Vec::new(),
        }
    }

    fn assert_orchestra_status(metadata: &crate::PaymentMetadata, expected: &ConversionStatus) {
        let info = metadata
            .conversion_info
            .as_ref()
            .expect("metadata should have conversion_info");
        match info {
            ConversionInfo::Orchestra { status, .. } => assert_eq!(status, expected),
            other => panic!("expected Orchestra variant, got {other:?}"),
        }
    }

    #[test_all]
    fn apply_terminal_status_skips_pending() {
        let info = orchestra_info("ord1", "q1");
        let resp = status_response(OrderStatus::Processing, Some("999000"));
        assert!(apply_terminal_status(&info, &resp).is_none());
    }

    #[test_all]
    fn apply_terminal_status_skips_non_orchestra_variant() {
        let amm_info = ConversionInfo::Amm {
            pool_id: "pool".to_string(),
            conversion_id: "cid".to_string(),
            status: ConversionStatus::Pending,
            fee: None,
            purpose: None,
            amount_adjustment: None,
        };
        let resp = status_response(OrderStatus::Completed, Some("999000"));
        assert!(apply_terminal_status(&amm_info, &resp).is_none());
    }

    #[test_all]
    fn apply_terminal_status_maps_completed() {
        let info = orchestra_info("ord1", "q1");
        let resp = status_response(OrderStatus::Completed, Some("999000"));
        let updated = apply_terminal_status(&info, &resp).expect("terminal");
        assert_orchestra_status(&updated, &ConversionStatus::Completed);
        if let Some(ConversionInfo::Orchestra {
            delivered_amount,
            estimated_out,
            fee_amount,
            ..
        }) = &updated.conversion_info
        {
            assert_eq!(*delivered_amount, Some(999_000));
            assert_eq!(*estimated_out, 1_000_000, "estimated_out stays frozen");
            // Realized fee = asset_amount_in (1_010_000) − delivered_amount (999_000)
            // = 11_000, overriding the prepare-time estimate of 10_000.
            assert_eq!(*fee_amount, Some(11_000));
        }
    }

    #[test_all]
    fn apply_terminal_status_maps_refunded() {
        let info = orchestra_info("ord1", "q1");
        let resp = status_response(OrderStatus::Refunded, None);
        let updated = apply_terminal_status(&info, &resp).expect("terminal");
        assert_orchestra_status(&updated, &ConversionStatus::Refunded);
        if let Some(ConversionInfo::Orchestra {
            delivered_amount,
            fee_amount,
            ..
        }) = &updated.conversion_info
        {
            assert_eq!(*delivered_amount, None, "no amount_out → None");
            // Refunds keep the prepare-time estimate; the realized fee
            // formula (`asset_amount_in − 0`) would be misleading.
            assert_eq!(
                *fee_amount,
                Some(10_000),
                "refund retains the prepare-time estimate"
            );
        }
    }

    #[test_all]
    fn apply_terminal_status_completed_without_asset_amount_in_keeps_estimate() {
        // Pre-upgrade row: `asset_amount_in` is None so the realized fee
        // cannot be computed. Stored estimate stays as-is.
        let info = match orchestra_info("ord1", "q1") {
            ConversionInfo::Orchestra {
                order_id,
                quote_id,
                chain,
                chain_id,
                asset,
                recipient_address,
                estimated_out,
                delivered_amount,
                status,
                service_fee_amount,
                service_fee_asset,
                read_token,
                asset_decimals,
                asset_contract,
                ..
            } => ConversionInfo::Orchestra {
                order_id,
                quote_id,
                chain,
                chain_id,
                asset,
                recipient_address,
                asset_amount_in: None,
                estimated_out,
                delivered_amount,
                status,
                fee_amount: Some(10_000),
                service_fee_amount,
                service_fee_asset,
                read_token,
                asset_decimals,
                asset_contract,
            },
            _ => unreachable!(),
        };
        let resp = status_response(OrderStatus::Completed, Some("999000"));
        let updated = apply_terminal_status(&info, &resp).expect("terminal");
        if let Some(ConversionInfo::Orchestra { fee_amount, .. }) = &updated.conversion_info {
            assert_eq!(
                *fee_amount,
                Some(10_000),
                "missing `asset_amount_in` falls back to the stored estimate"
            );
        }
    }

    #[test_all]
    fn apply_terminal_status_maps_failed() {
        let info = orchestra_info("ord1", "q1");
        let resp = status_response(OrderStatus::Failed, None);
        let updated = apply_terminal_status(&info, &resp).expect("terminal");
        assert_orchestra_status(&updated, &ConversionStatus::Failed);
    }

    #[test_all]
    fn apply_terminal_status_ignores_unparseable_amount_out() {
        let info = orchestra_info("ord1", "q1");
        let resp = status_response(OrderStatus::Completed, Some("not-a-number"));
        let updated = apply_terminal_status(&info, &resp).expect("terminal");
        if let Some(ConversionInfo::Orchestra {
            delivered_amount, ..
        }) = &updated.conversion_info
        {
            assert_eq!(*delivered_amount, None, "unparseable amount_out → None");
        }
    }

    // ---- find_source_asset ----

    fn dest_pair(chain: &str, asset: &str, contract: Option<&str>) -> CrossChainRoutePair {
        CrossChainRoutePair {
            provider: CrossChainProvider::Orchestra,
            chain: chain.to_string(),
            chain_id: None,
            asset: asset.to_string(),
            contract_address: contract.map(str::to_string),
            decimals: 6,
            exact_out_eligible: false,
            spark_assets: Vec::new(),
        }
    }

    #[test_all]
    fn find_source_asset_matches_btc_source_case_insensitively() {
        // Source side asset is "btc" lowercase; lookup should still match.
        let routes = vec![route(
            ra("spark", "btc", None),
            ra("base", "USDC", Some("0xUSDC")),
        )];
        let dest = dest_pair("base", "USDC", Some("0xUSDC"));
        let found = find_source_asset(&routes, &dest, None).expect("route should match");
        assert_eq!(found.asset, "btc");
    }

    #[test_all]
    fn find_source_asset_matches_token_source_by_contract_address() {
        let routes = vec![
            route(ra("spark", "BTC", None), ra("base", "USDC", Some("0xUSDC"))),
            route(
                ra("spark", "USDB", Some("btkn1usdb_contract")),
                ra("base", "USDC", Some("0xUSDC")),
            ),
        ];
        let dest = dest_pair("base", "USDC", Some("0xUSDC"));
        let found = find_source_asset(&routes, &dest, Some("btkn1usdb_contract"))
            .expect("route should match");
        assert_eq!(found.asset, "USDB");
    }

    #[test_all]
    fn find_source_asset_returns_none_when_destination_mismatch() {
        let routes = vec![route(
            ra("spark", "BTC", None),
            ra("base", "USDC", Some("0xUSDC")),
        )];
        // Different destination chain.
        let dest = dest_pair("tron", "USDC", Some("0xUSDC"));
        assert!(find_source_asset(&routes, &dest, None).is_none());
    }

    #[test_all]
    fn find_source_asset_returns_none_when_token_identifier_mismatch() {
        let routes = vec![route(
            ra("spark", "USDB", Some("btkn1usdb")),
            ra("base", "USDC", Some("0xUSDC")),
        )];
        let dest = dest_pair("base", "USDC", Some("0xUSDC"));
        assert!(find_source_asset(&routes, &dest, Some("btkn1other")).is_none());
    }

    #[test_all]
    fn find_source_asset_distinguishes_contract_address_when_chain_repeats() {
        // Two routes to the same chain/asset but different destination contracts.
        let routes = vec![
            route(ra("spark", "BTC", None), ra("base", "USDC", Some("0xAAA"))),
            route(ra("spark", "BTC", None), ra("base", "USDC", Some("0xBBB"))),
        ];
        let dest = dest_pair("base", "USDC", Some("0xBBB"));
        // The match logic uses contract_address as part of the destination
        // identity, so this picks the second route.
        let found = find_source_asset(&routes, &dest, None).expect("route should match");
        assert_eq!(found.asset, "BTC");
    }

    // ---- find_destination_asset_symbol ----

    /// Build a [`CrossChainRoutePair`] describing the external (source)
    /// side of a receive-direction route. Mirrors `dest_pair` in shape but
    /// reads naturally from receive call sites.
    fn external_pair(chain: &str, asset: &str, contract: Option<&str>) -> CrossChainRoutePair {
        CrossChainRoutePair {
            provider: CrossChainProvider::Orchestra,
            chain: chain.to_string(),
            chain_id: None,
            asset: asset.to_string(),
            contract_address: contract.map(str::to_string),
            decimals: 6,
            exact_out_eligible: false,
            spark_assets: Vec::new(),
        }
    }

    /// Bitcoin destination resolves to the wire symbol "BTC" from the
    /// matching raw route's Spark side.
    #[test_all]
    fn find_destination_asset_symbol_resolves_bitcoin() {
        let routes = vec![
            // On receive routes, source = external, destination = Spark.
            route(ra("base", "USDC", Some("0xUSDC")), ra("spark", "BTC", None)),
            route(
                ra("base", "USDC", Some("0xUSDC")),
                ra("spark", "USDB", Some("btkn1usdb")),
            ),
        ];
        let pair = external_pair("base", "USDC", Some("0xUSDC"));
        let sym = find_destination_asset_symbol(&routes, &pair, &SparkAsset::Bitcoin);
        assert_eq!(sym.as_deref(), Some("BTC"));
    }

    /// Token destination picks the raw route whose Spark-side
    /// `contract_address` matches the requested token id, and surfaces
    /// that route's asset symbol.
    #[test_all]
    fn find_destination_asset_symbol_resolves_token_by_identifier() {
        let routes = vec![
            route(ra("base", "USDC", Some("0xUSDC")), ra("spark", "BTC", None)),
            route(
                ra("base", "USDC", Some("0xUSDC")),
                ra("spark", "USDB", Some("btkn1usdb")),
            ),
        ];
        let pair = external_pair("base", "USDC", Some("0xUSDC"));
        let sym = find_destination_asset_symbol(
            &routes,
            &pair,
            &SparkAsset::Token {
                token_identifier: "btkn1usdb".to_string(),
            },
        );
        assert_eq!(sym.as_deref(), Some("USDB"));
    }

    /// A token id that no raw route exposes returns `None` — the SDK
    /// validation upstream should have caught this already; the helper
    /// just reports its truthful answer.
    #[test_all]
    fn find_destination_asset_symbol_returns_none_for_unknown_token() {
        let routes = vec![route(
            ra("base", "USDC", Some("0xUSDC")),
            ra("spark", "BTC", None),
        )];
        let pair = external_pair("base", "USDC", Some("0xUSDC"));
        let sym = find_destination_asset_symbol(
            &routes,
            &pair,
            &SparkAsset::Token {
                token_identifier: "btkn1nothing".to_string(),
            },
        );
        assert!(sym.is_none());
    }

    /// An external pair the route catalogue doesn't carry returns `None`.
    #[test_all]
    fn find_destination_asset_symbol_returns_none_when_external_pair_unknown() {
        let routes = vec![route(
            ra("base", "USDC", Some("0xUSDC")),
            ra("spark", "BTC", None),
        )];
        let pair = external_pair("solana", "USDC", Some("USDCsol"));
        let sym = find_destination_asset_symbol(&routes, &pair, &SparkAsset::Bitcoin);
        assert!(sym.is_none());
    }

    // `rescale_decimals` and `is_usd_stable_asset` live in cross_chain/mod.rs;
    // tests for them are colocated there.

    #[test_all]
    fn proportional_inflation_scales_source_to_hit_target() {
        // 10_000 sats delivered 5_980_000 → to deliver 6_000_000 we need
        // 10_000 * 6_000_000 / 5_980_000 = 10_033 sats.
        let inflated = proportional_inflation(10_000, 6_000_000, 5_980_000).unwrap();
        assert_eq!(inflated, 10_033);
    }

    #[test_all]
    fn proportional_inflation_floors_at_source_amount() {
        // Estimate over-delivers (probe rate temporarily favourable). The
        // formula would suggest a smaller source, but we never inflate to less
        // than `source_amount` — fees-on-top means sender pays at least amount.
        let inflated = proportional_inflation(10_000, 6_000_000, 6_010_000).unwrap();
        assert_eq!(inflated, 10_000);
    }

    #[test_all]
    fn proportional_inflation_exact_target_returns_source() {
        // Estimate delivers exactly the target → no inflation, just pass through.
        let inflated = proportional_inflation(10_000, 6_000_000, 6_000_000).unwrap();
        assert_eq!(inflated, 10_000);
    }

    #[test_all]
    fn proportional_inflation_rejects_zero_delivered() {
        let err = proportional_inflation(10_000, 6_000_000, 0).unwrap_err();
        assert!(matches!(err, SdkError::Generic(ref m) if m.contains("zero delivered")));
    }

    // ---- adjust_estimate_for_receive_buffers ----

    /// Estimate at or below the buffer floor is rejected with `InvalidInput`
    /// so the caller can surface a "too small" error at prepare time.
    #[test_all]
    fn adjust_estimate_rejects_below_buffer_floor() {
        let floor = super::STABLE_RECEIVE_BUFFER_BASE_UNITS;
        let err = adjust_estimate_for_receive_buffers(floor).unwrap_err();
        assert!(matches!(err, SdkError::InvalidInput(_)));
        let err = adjust_estimate_for_receive_buffers(floor - 1).unwrap_err();
        assert!(matches!(err, SdkError::InvalidInput(_)));
    }

    /// Estimate above the floor is shaved by exactly the external buffer.
    /// Feeding the adjusted value into `proportional_inflation` produces a
    /// `required_in` larger than the pre-fix version by roughly
    /// `(buffer × source / delivered)`.
    #[test_all]
    fn adjust_estimate_shaves_by_external_buffer_and_scales_required_in_up() {
        let external = super::STABLE_RECEIVE_BUFFER_BASE_UNITS;
        let raw_delivered = 990_000u128;
        let adjusted = adjust_estimate_for_receive_buffers(raw_delivered).unwrap();
        assert_eq!(adjusted, raw_delivered - external);

        let source = 1_000_000u128;
        let target = 1_000_000u128;
        let pre_fix = proportional_inflation(source, target, raw_delivered).unwrap();
        let post_fix = proportional_inflation(source, target, adjusted).unwrap();
        assert!(post_fix > pre_fix);
    }

    #[test_all]
    fn verify_quote_not_drifted_accepts_exact_target() {
        assert!(verify_quote_not_drifted(1_000_000, 1_000_000, 100).is_ok());
    }

    #[test_all]
    fn verify_quote_not_drifted_accepts_within_slippage() {
        // 1% slippage on 1_000_000 = 10_000 → minimum acceptable 990_000.
        assert!(verify_quote_not_drifted(1_000_000, 990_000, 100).is_ok());
        assert!(verify_quote_not_drifted(1_000_000, 995_000, 100).is_ok());
    }

    #[test_all]
    fn verify_quote_not_drifted_rejects_below_buffer() {
        // 1% slippage tolerates down to 990_000; 989_999 must fail.
        let err = verify_quote_not_drifted(1_000_000, 989_999, 100).unwrap_err();
        match err {
            SdkError::InvalidInput(ref msg) => {
                assert!(
                    msg.contains("rate drift") && msg.contains("1000000") && msg.contains("989999"),
                    "unexpected message: {msg}"
                );
            }
            other => panic!("expected InvalidInput rate-drift error, got {other:?}"),
        }
    }

    #[test_all]
    fn verify_quote_not_drifted_extreme_slippage_accepts_anything() {
        // 100% slippage = no floor.
        assert!(verify_quote_not_drifted(1_000_000, 0, 10_000).is_ok());
    }

    // ---- validate_quote_expiry ----

    #[test_all]
    fn validate_quote_expiry_accepts_future_rfc3339() {
        use platform_utils::time::{SystemTime, UNIX_EPOCH};
        let future_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .saturating_add(600);
        let dt =
            chrono::DateTime::<chrono::Utc>::from_timestamp(future_secs.cast_signed(), 0).unwrap();
        let s = dt.to_rfc3339();
        assert!(validate_quote_expiry(&s).is_ok());
    }

    #[test_all]
    fn validate_quote_expiry_rejects_past_rfc3339() {
        // 2001-09-09 — well in the past.
        let err = validate_quote_expiry("2001-09-09T01:46:40Z").unwrap_err();
        assert!(matches!(err, SdkError::InvalidInput(ref m) if m.contains("expired")));
    }

    #[test_all]
    fn validate_quote_expiry_rejects_malformed() {
        let err = validate_quote_expiry("not-a-timestamp").unwrap_err();
        assert!(matches!(err, SdkError::Generic(ref m) if m.contains("invalid expires_at")));
    }

    #[test_all]
    fn dedupe_routes_skips_non_btc_spark_source_without_contract() {
        // Defensive: a non-BTC Spark side missing `contract_address` would
        // be silently dropped as a source variant. This shouldn't happen
        // in practice but the path is exercised here.
        let routes = vec![route(
            ra("spark", "MYSTERY", None),
            ra("base", "USDC", Some("0xABC")),
        )];

        let pairs = dedupe_routes(&routes, true, None, None);

        // The route still produces a pair (the destination still matters),
        // but `spark_assets` is empty.
        assert_eq!(pairs.len(), 1);
        assert!(pairs[0].spark_assets.is_empty());
    }
}
