//! Flashnet Orchestra cross-chain provider.
//!
//! Implements [`CrossChainProvider`] for the Orchestra bridge/swap API.
//! Handles quoting, sending (deposit + submit), and background monitoring
//! of in-flight orders.

#![allow(dead_code)]

use std::collections::HashSet;
use std::sync::Arc;

use breez_sdk_common::input::CrossChainAddressFamily;
use flashnet::OrchestraClient;
use flashnet::orchestra::{
    AmountMode, OrderStatus, QuoteRequest, QuoteResponse, Route, RouteAsset, StatusResponse,
    SubmitResponse,
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
use crate::{ConversionInfo, ConversionStatus, Network, PaymentDetails, Storage};

use super::{
    CrossChainPrepared, CrossChainProvider, CrossChainRouteFilter, CrossChainRoutePair,
    CrossChainSendResult, CrossChainService,
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
        network: Network,
        spark_wallet: Arc<SparkWallet>,
        storage: Arc<dyn Storage>,
        shutdown_receiver: watch::Receiver<()>,
    ) -> Self {
        let client = Arc::new(OrchestraClient::new(
            config,
            network.into(),
            Arc::clone(&spark_wallet),
        ));
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
                continue;
            };

            let ConversionInfo::Orchestra {
                order_id,
                quote_id,
                destination_chain,
                destination_asset,
                destination_address,
                estimated_out,
                fee,
                ..
            } = conversion_info.clone()
            else {
                continue;
            };

            // Prefer order_id, fall back to quote_id if order_id is empty
            // (can happen if /submit failed but we still want to track).
            let status_response = if order_id.is_empty() {
                client.status_by_quote_id(&quote_id).await
            } else {
                client.status_by_id(&order_id).await
            };

            let status_response = match status_response {
                Ok(r) => r,
                Err(e) => {
                    debug!("Orchestra status query failed for order {order_id}: {e}");
                    continue;
                }
            };

            let order_status = status_response.order.status;
            if !order_status.is_terminal() {
                continue;
            }

            let new_status = match order_status {
                OrderStatus::Completed => ConversionStatus::Completed,
                OrderStatus::Refunded => ConversionStatus::Refunded,
                _ => ConversionStatus::Failed,
            };

            // Use the real amount_out from Orchestra if available, otherwise
            // keep the original estimate.
            let final_out = status_response
                .order
                .amount_out
                .as_deref()
                .and_then(|s| s.parse::<u128>().ok())
                .unwrap_or(estimated_out);

            let updated_metadata = crate::PaymentMetadata {
                conversion_info: Some(ConversionInfo::Orchestra {
                    order_id,
                    quote_id,
                    destination_chain,
                    destination_asset,
                    destination_address,
                    estimated_out: final_out,
                    status: new_status.clone(),
                    fee,
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
        // contract_address) so the caller only sees one route per external endpoint.
        let mut seen = HashSet::new();
        let mut pairs: Vec<CrossChainRoutePair> = Vec::new();

        for r in routes.iter().filter(|r| {
            let side = non_spark_side(r, is_send);
            let ca = side.contract_address.as_deref();
            family_filter.is_none_or(|f| f.matches_chain(&side.chain, ca))
                && contract_filter.is_none_or(|filter_ca| ca.is_some_and(|c| c == filter_ca))
        }) {
            let side = non_spark_side(r, is_send);
            let key = (
                side.chain.clone(),
                side.asset.clone(),
                side.contract_address.clone(),
            );
            if seen.insert(key) {
                pairs.push(CrossChainRoutePair {
                    provider: CrossChainProvider::Orchestra,
                    chain: side.chain.clone(),
                    asset: side.asset.clone(),
                    contract_address: side.contract_address.clone(),
                    decimals: side.decimals,
                    exact_out_eligible: r.exact_out_eligible,
                });
            }
        }

        Ok(pairs)
    }

    async fn prepare(
        &self,
        recipient_address: &str,
        route: &CrossChainRoutePair,
        amount: u128,
        token_identifier: Option<String>,
        max_slippage_bps: Option<u32>,
    ) -> Result<CrossChainPrepared, SdkError> {
        let dest_chain = &route.chain;
        let dest_asset = &route.asset;
        let source_asset = match token_identifier.as_deref() {
            None => "BTC".to_string(),
            Some(_) => "USDB".to_string(),
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
            .map_err(|e| SdkError::Generic(format!("Orchestra quote failed: {e}")))?;
        debug!("Orchestra: quote response: {:?}", quote);

        let amount_in = parse_amount(&quote.amount_in, "amountIn")?;
        let estimated_out = parse_amount(&quote.estimated_out, "estimatedOut")?;
        let fee_amount = parse_amount(&quote.fee_amount, "feeAmount")?;

        Ok(CrossChainPrepared {
            quote_id: quote.quote_id,
            deposit_request: quote.deposit_address,
            amount_in,
            estimated_out,
            fee_amount,
            fee_asset: if quote.fee_asset.eq_ignore_ascii_case("BTC") {
                None
            } else {
                Some(quote.fee_asset)
            },
            expires_at: quote.expires_at,
            pair: route.clone(),
            recipient_address: recipient_address.to_string(),
            token_identifier,
        })
    }

    async fn send(&self, prepared: &CrossChainPrepared) -> Result<CrossChainSendResult, SdkError> {
        // Step 1: Spark transfer to the Orchestra deposit address.
        let spark_tx_hash = self
            .client
            .transfer_to_deposit(
                &prepared.deposit_request,
                prepared.amount_in,
                prepared.token_identifier.as_deref(),
            )
            .await
            .map_err(|e| SdkError::Generic(format!("Orchestra deposit transfer failed: {e}")))?;

        debug!(
            "Orchestra: deposit transfer {spark_tx_hash} sent for quote {}",
            prepared.quote_id
        );

        // Step 2: Submit the deposit to Orchestra.
        let submit_res: Result<SubmitResponse, _> = self
            .client
            .submit_spark(flashnet::orchestra::SubmitRequestSpark {
                quote_id: prepared.quote_id.clone(),
                spark_tx_hash: spark_tx_hash.clone(),
                source_spark_address: None,
            })
            .await;
        debug!("Orchestra: submit response: {:?}", submit_res);

        // Step 3: Persist ConversionInfo::Orchestra metadata.
        let (status, order_id) = match &submit_res {
            Ok(response) => (ConversionStatus::Pending, response.order_id.clone()),
            Err(e) => {
                error!("Orchestra /submit failed after deposit transfer {spark_tx_hash}: {e}");
                (ConversionStatus::RefundNeeded, String::new())
            }
        };

        let metadata = crate::PaymentMetadata {
            conversion_info: Some(ConversionInfo::Orchestra {
                order_id: order_id.clone(),
                quote_id: prepared.quote_id.clone(),
                destination_chain: prepared.pair.chain.clone(),
                destination_asset: prepared.pair.asset.clone(),
                destination_address: prepared.recipient_address.clone(),
                estimated_out: prepared.estimated_out,
                status,
                fee: Some(prepared.fee_amount),
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

#[allow(dead_code)]
fn _keep_status_response(_: StatusResponse) {}
