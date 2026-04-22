//! Flashnet Orchestra cross-chain provider.
//!
//! Implements [`CrossChainProvider`] for the Orchestra bridge/swap API.
//! Handles quoting, sending (deposit + submit), and background monitoring
//! of in-flight orders.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Arc;

use breez_sdk_common::input::CrossChainAddressFamily;
use flashnet::OrchestraClient;
use flashnet::orchestra::{
    AmountMode, OrderStatus, QuoteRequest, QuoteResponse, Route, RouteAsset, SubmitResponse,
};
use platform_utils::time::Duration;
use platform_utils::tokio;
use spark_wallet::SparkWallet;
use tokio::{
    select,
    sync::{broadcast, watch},
};
use tracing::{Instrument, debug, error, info};

use crate::error::SdkError;
use crate::persist::{ConversionFilter, StorageListPaymentsRequest, StoragePaymentDetailsFilter};
use crate::{ConversionInfo, ConversionStatus, PaymentDetails, Storage};

use super::{
    CrossChainFeeMode, CrossChainPrepared, CrossChainProvider, CrossChainProviderContext,
    CrossChainRouteFilter, CrossChainRoutePair, CrossChainSendResult, CrossChainService,
    SourceAsset,
};

/// The canonical Spark source chain string used by Orchestra.
const SPARK_SOURCE_CHAIN: &str = "spark";

fn parse_amount(value: &str, field: &str) -> Result<u128, SdkError> {
    value
        .parse::<u128>()
        .map_err(|e| SdkError::Generic(format!("Orchestra returned invalid {field}: {e}")))
}

const DEFAULT_SLIPPAGE_BPS: u32 = 50;

/// How often the background monitor polls in-flight orders.
const MONITOR_INTERVAL: Duration = Duration::from_secs(30);

/// Flashnet Orchestra cross-chain provider.
pub(crate) struct OrchestraService {
    client: Arc<OrchestraClient>,
    spark_wallet: Arc<SparkWallet>,
    storage: Arc<dyn Storage>,
    monitor_trigger: broadcast::Sender<()>,
}

impl OrchestraService {
    pub(crate) fn new(
        config: flashnet::OrchestraConfig,
        spark_wallet: Arc<SparkWallet>,
        storage: Arc<dyn Storage>,
        shutdown_receiver: watch::Receiver<()>,
    ) -> Self {
        let client = Arc::new(OrchestraClient::new(config, Arc::clone(&spark_wallet)));
        let (monitor_trigger, _) = broadcast::channel(10);

        let service = Self {
            client,
            spark_wallet,
            storage,
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
        let client = Arc::clone(&self.client);
        let mut trigger_receiver = monitor_trigger.subscribe();
        let span = tracing::Span::current();

        tokio::spawn(
            async move {
                loop {
                    if let Err(e) = Self::poll_in_flight_orders(&storage, &client).await {
                        error!("Orchestra monitor poll failed: {e:?}");
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

    /// Polls Orchestra for status updates on in-flight cross-chain orders.
    ///
    /// Queries storage for payments with `ConversionFilter::OrchestraPending`,
    /// calls the Orchestra `/status` endpoint for each, and updates the
    /// `ConversionInfo::Orchestra` metadata when the order reaches a terminal
    /// state (replacing the estimated output with the real `amount_out`).
    #[allow(clippy::too_many_lines)]
    async fn poll_in_flight_orders(
        storage: &Arc<dyn Storage>,
        client: &Arc<OrchestraClient>,
    ) -> Result<(), SdkError> {
        debug!("Orchestra monitor: polling for in-flight orders");
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
            .await
            .map_err(|e| {
                SdkError::Generic(format!("Failed to list pending Orchestra orders: {e}"))
            })?;

        debug!("Orchestra monitor: found {} pending orders", pending.len());
        for payment in &pending {
            // Extract all Orchestra metadata fields in one destructure.
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
                    "Orchestra monitor: payment {} has no Orchestra conversion_info, skipping",
                    payment.id
                );
                continue;
            };

            let ConversionInfo::Orchestra {
                order_id,
                quote_id,
                chain,
                chain_id,
                asset,
                recipient_address,
                estimated_out,
                fee,
                read_token,
                asset_decimals,
                asset_contract,
                ..
            } = conversion_info.clone()
            else {
                debug!(
                    "Orchestra monitor: payment {} conversion_info is not Orchestra variant, skipping",
                    payment.id
                );
                continue;
            };

            let lookup_id = if order_id.is_empty() {
                &quote_id
            } else {
                &order_id
            };
            debug!(
                "Orchestra monitor: checking payment {} (order={order_id}, quote={quote_id}, dest={chain}/{asset})",
                payment.id
            );

            // Prefer order_id, fall back to quote_id if order_id is empty
            // (can happen if /submit failed but we still want to track).
            let rt = read_token.as_deref();
            let status_response = if order_id.is_empty() {
                client.status_by_quote_id(&quote_id, rt).await
            } else {
                client.status_by_id(&order_id, rt).await
            };

            let status_response = match status_response {
                Ok(r) => r,
                Err(e) => {
                    debug!("Orchestra monitor: status query failed for {lookup_id}: {e}");
                    continue;
                }
            };

            let order_status = status_response.order.status;
            debug!(
                "Orchestra monitor: payment {} order status: {order_status:?} (amount_out={:?})",
                payment.id, status_response.order.amount_out,
            );

            if !order_status.is_terminal() {
                debug!(
                    "Orchestra monitor: payment {} still in progress",
                    payment.id
                );
                continue;
            }

            let new_status = match order_status {
                OrderStatus::Completed => ConversionStatus::Completed,
                OrderStatus::Refunded => ConversionStatus::Refunded,
                _ => ConversionStatus::Failed,
            };

            // Use the real amounts from Orchestra status if available.
            // Keep estimated_out frozen; set delivered_amount with the actual.
            let delivered_amount = status_response
                .order
                .amount_out
                .as_deref()
                .and_then(|s| s.parse::<u128>().ok());

            debug!(
                "Orchestra monitor: payment {} terminal → {new_status:?}, delivered={delivered_amount:?} (estimated was {estimated_out})",
                payment.id
            );

            let updated_metadata = crate::PaymentMetadata {
                conversion_info: Some(ConversionInfo::Orchestra {
                    order_id,
                    quote_id,
                    chain,
                    chain_id,
                    asset,
                    recipient_address,
                    estimated_out,
                    delivered_amount,
                    status: new_status.clone(),
                    fee,
                    read_token,
                    asset_decimals,
                    asset_contract,
                }),
                ..Default::default()
            };

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
                    "Orchestra order for payment {} reached terminal state: {new_status:?}",
                    payment.id
                );
            }
        }

        Ok(())
    }
}

#[macros::async_trait]
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

        let routes =
            self.client.spark_routes(is_send).await.map_err(|e| {
                SdkError::Generic(format!("Failed to fetch cross-chain routes: {e}"))
            })?;

        // The non-Spark side of the route: for send it's the destination,
        // for receive it's the source (the chain the user sends from).
        fn non_spark_side(r: &Route, is_send: bool) -> &RouteAsset {
            if is_send { &r.destination } else { &r.source }
        }

        // Multiple raw routes may exist for the same external chain (e.g.
        // BTC→USDT-on-tron and USDB→USDT-on-tron). Dedup by (chain, asset,
        // contract_address) so the caller only sees one route per external
        // endpoint, but accumulate the Spark-side source variants into
        // `supported_sources`.
        type Key = (String, String, Option<String>);
        let mut order: Vec<Key> = Vec::new();
        let mut grouped: HashMap<Key, CrossChainRoutePair> = HashMap::new();

        for r in routes.iter().filter(|r| {
            let side = non_spark_side(r, is_send);
            let ca = side.contract_address.as_deref();
            family_filter.is_none_or(|f| f.matches_chain(&side.chain, ca))
                && contract_filter.is_none_or(|filter_ca| ca.is_some_and(|c| c == filter_ca))
        }) {
            let side = non_spark_side(r, is_send);
            let key: Key = (
                side.chain.clone(),
                side.asset.clone(),
                side.contract_address.clone(),
            );

            // On send, the Spark side is `source`; on receive, it's `destination`.
            let spark_side = if is_send { &r.source } else { &r.destination };
            let source_variant = if spark_side.asset.eq_ignore_ascii_case("BTC") {
                Some(SourceAsset::Bitcoin)
            } else {
                // Non-BTC Spark source without contract_address: defensive
                // skip. Shouldn't happen per current Orchestra behavior.
                spark_side
                    .contract_address
                    .as_ref()
                    .map(|ca| SourceAsset::Token(ca.clone()))
            };

            let entry = grouped.entry(key.clone()).or_insert_with(|| {
                order.push(key.clone());
                side_to_route_pair(side, r.exact_out_eligible)
            });

            if let Some(variant) = source_variant
                && !entry.supported_sources.contains(&variant)
            {
                entry.supported_sources.push(variant);
            }
        }

        let pairs: Vec<CrossChainRoutePair> = order
            .into_iter()
            .filter_map(|k| grouped.remove(&k))
            .collect();
        Ok(pairs)
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
        let dest_chain = &route.chain;
        let dest_asset = &route.asset;
        // Resolve the Orchestra-side source_asset string (e.g. "BTC", "USDB")
        // from the cached spark_routes. Match the route with the correct
        // destination triplet AND the desired source (contract_address when
        // a token_identifier is requested, asset == "BTC" otherwise).
        let raw_routes =
            self.client.spark_routes(true).await.map_err(|e| {
                SdkError::Generic(format!("Failed to fetch cross-chain routes: {e}"))
            })?;
        let matched = raw_routes.iter().find(|r| {
            let dest_matches = r.destination.chain == *dest_chain
                && r.destination.asset == *dest_asset
                && r.destination.contract_address == route.contract_address;
            let source_matches = match token_identifier.as_deref() {
                None => r.source.asset.eq_ignore_ascii_case("BTC"),
                Some(tid) => r.source.contract_address.as_deref() == Some(tid),
            };
            dest_matches && source_matches
        });
        let source_asset = match matched {
            Some(r) => r.source.asset.clone(),
            None => {
                return Err(SdkError::InvalidInput(format!(
                    "Orchestra does not offer a route for source {} → {}/{}",
                    token_identifier.as_deref().unwrap_or("BTC"),
                    dest_chain,
                    dest_asset
                )));
            }
        };

        let request = QuoteRequest {
            source_chain: SPARK_SOURCE_CHAIN.to_string(),
            source_asset: source_asset.clone(),
            destination_chain: dest_chain.clone(),
            destination_asset: dest_asset.clone(),
            amount: amount.to_string(),
            recipient_address: recipient_address.to_string(),
            amount_mode: Some(AmountMode::ExactIn),
            refund_address: None,
            slippage_bps: Some(max_slippage_bps.unwrap_or(DEFAULT_SLIPPAGE_BPS)),
            zeroconf_enabled: None,
            app_fees: Vec::new(),
            affiliate_id: None,
        };

        debug!(
            "Orchestra: requesting quote: {}/{} -> {}/{} amount={}",
            request.source_chain,
            request.source_asset,
            request.destination_chain,
            request.destination_asset,
            request.amount
        );
        let quote: QuoteResponse = self
            .client
            .quote(request)
            .await
            .map_err(|e| SdkError::Generic(format!("Orchestra: {e}")))?;
        debug!("Orchestra: quote response: {:?}", quote);

        let amount_in = parse_amount(&quote.amount_in, "amountIn")?;
        let estimated_out = parse_amount(&quote.estimated_out, "estimatedOut")?;
        let fee_amount = parse_amount(&quote.total_fee_amount, "totalFeeAmount")?;

        let provider_context = CrossChainProviderContext::Orchestra {
            quote_id: quote.quote_id,
            deposit_address: quote.deposit_address,
        };

        Ok(CrossChainPrepared {
            amount_in,
            estimated_out,
            fee_amount,
            fee_asset: if quote.fee_asset.eq_ignore_ascii_case("BTC") {
                None
            } else {
                Some(quote.fee_asset)
            },
            // Spark transfer fee is 0 today; the field is wired for a future
            // non-zero case. Both FeesIncluded/FeesExcluded pass through
            // identically since `amount_in = amount`.
            source_transfer_fee_sats: 0,
            fee_mode,
            expires_at: quote.expires_at,
            pair: route.clone(),
            recipient_address: recipient_address.to_string(),
            token_identifier,
            provider_context,
        })
    }

    async fn send(&self, prepared: &CrossChainPrepared) -> Result<CrossChainSendResult, SdkError> {
        let CrossChainProviderContext::Orchestra {
            quote_id,
            deposit_address,
        } = &prepared.provider_context
        else {
            return Err(SdkError::Generic(
                "Orchestra send called with non-Orchestra provider context".to_string(),
            ));
        };

        // Step 1: Spark transfer to the Orchestra deposit address.
        let spark_tx_hash = self
            .client
            .transfer_to_deposit(
                deposit_address,
                prepared.amount_in,
                prepared.token_identifier.as_deref(),
            )
            .await
            .map_err(|e| SdkError::Generic(format!("Orchestra deposit transfer failed: {e}")))?;
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
        let submit_res: Result<SubmitResponse, _> = self
            .client
            .submit_spark(flashnet::orchestra::SubmitRequestSpark {
                quote_id: quote_id.clone(),
                spark_tx_hash: spark_tx_hash.clone(),
                source_spark_address,
            })
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

        let metadata = crate::PaymentMetadata {
            conversion_info: Some(ConversionInfo::Orchestra {
                order_id: order_id.clone(),
                quote_id: quote_id.clone(),
                chain: prepared.pair.chain.clone(),
                chain_id: prepared.pair.chain_id.clone(),
                asset: prepared.pair.asset.clone(),
                recipient_address: prepared.recipient_address.clone(),
                estimated_out: prepared.estimated_out,
                delivered_amount: None,
                status,
                fee: Some(prepared.fee_amount),
                read_token,
                asset_decimals: u32::from(prepared.pair.decimals),
                asset_contract: prepared.pair.contract_address.clone(),
            }),
            ..Default::default()
        };

        let payment_id = crate::utils::payments::insert_or_cache_payment_metadata(
            &spark_tx_hash,
            metadata,
            &self.spark_wallet,
            &self.storage,
            true,
        )
        .await
        .unwrap_or_else(|e| {
            error!("Failed to persist Orchestra metadata for payment {spark_tx_hash}: {e:?}");
            spark_tx_hash
        });

        self.trigger_monitor();

        submit_res
            .map(|r| CrossChainSendResult {
                order_id: r.order_id,
                payment_id: payment_id.clone(),
            })
            .map_err(|e| SdkError::Generic(format!("Orchestra submit failed: {e}")))
    }
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
        supported_sources: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_route_asset(chain: &str, chain_id: Option<&str>) -> RouteAsset {
        RouteAsset {
            chain: chain.to_string(),
            asset: "USDC".to_string(),
            contract_address: Some("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string()),
            decimals: 6,
            chain_id: chain_id.map(str::to_string),
        }
    }

    #[test]
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

    #[test]
    fn side_to_pair_preserves_missing_chain_id() {
        let side = test_route_asset("solana", None);
        let pair = side_to_route_pair(&side, false);

        assert_eq!(
            pair.chain_id, None,
            "chain_id stays None when the route asset doesn't expose one"
        );
        assert!(!pair.exact_out_eligible);
    }
}
