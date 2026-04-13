//! Flashnet Orchestra cross-chain provider.
//!
//! Implements [`CrossChainProvider`] for the Orchestra bridge/swap API.
//! Handles quoting, sending (deposit + submit), and background monitoring
//! of in-flight orders.

#![allow(dead_code)]

use std::sync::Arc;

use flashnet::OrchestraClient;
use flashnet::orchestra::{
    AmountMode, QuoteRequest, QuoteResponse, StatusResponse, SubmitResponse,
};
use platform_utils::time::Duration;
use platform_utils::tokio;
use spark_wallet::SparkWallet;
use tokio::{
    select,
    sync::{broadcast, watch},
};
use tracing::{Instrument, debug, error, info};

use breez_sdk_common::input::{CrossChainAddressFamily, CrossChainRoutePair};

use crate::error::SdkError;
use crate::{Network, Storage};

use super::{CrossChainPrepared, CrossChainSendResult, CrossChainService};

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

    #[allow(clippy::unused_async)]
    async fn poll_in_flight_orders(
        storage: &Arc<dyn Storage>,
        client: &Arc<OrchestraClient>,
    ) -> Result<(), SdkError> {
        // No-op until the persistence layer exposes an "orchestra order pending" filter.
        let _ = (storage, client);
        Ok(())
    }
}

#[macros::async_trait]
impl CrossChainService for OrchestraService {
    async fn get_routes(
        &self,
        family: CrossChainAddressFamily,
        asset: Option<&str>,
    ) -> Result<Vec<CrossChainRoutePair>, SdkError> {
        // Fetch all Spark-sourced routes and filter to those matching the address family.
        let routes =
            self.client.routes_for_chains(&[]).await.map_err(|e| {
                SdkError::Generic(format!("Failed to fetch cross-chain routes: {e}"))
            })?;

        let pairs: Vec<CrossChainRoutePair> = routes
            .iter()
            .filter(|r| family.matches_chain(&r.destination.chain))
            .filter(|r| {
                if let Some(af) = asset {
                    r.destination.asset.eq_ignore_ascii_case(af)
                } else {
                    true
                }
            })
            .map(|r| CrossChainRoutePair {
                chain: r.destination.chain.clone(),
                asset: r.destination.asset.clone(),
                contract_address: r.destination.contract_address.clone(),
                decimals: r.destination.decimals,
                exact_out_eligible: r.exact_out_eligible,
            })
            .collect();

        Ok(pairs)
    }

    async fn prepare(
        &self,
        recipient_address: &str,
        dest_chain: &str,
        dest_asset: &str,
        amount: u128,
        token_identifier: Option<String>,
        max_slippage_bps: Option<u32>,
    ) -> Result<CrossChainPrepared, SdkError> {
        let source_asset = match token_identifier.as_deref() {
            None => "BTC".to_string(),
            Some(_) => "USDB".to_string(),
        };

        let request = QuoteRequest {
            source_chain: SPARK_SOURCE_CHAIN.to_string(),
            source_asset: source_asset.clone(),
            destination_chain: dest_chain.to_string(),
            destination_asset: dest_asset.to_string(),
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
            "Orchestra: requesting quote {}/{} -> {}/{} amount={}",
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

        let amount_in = parse_amount(&quote.amount_in, "amountIn")?;
        let estimated_out = parse_amount(&quote.estimated_out, "estimatedOut")?;
        let fee_amount = parse_amount(&quote.fee_amount, "feeAmount")?;

        // Look up route details (contract_address, decimals, etc.) from cached routes.
        let route_pair = match self.client.find_route(dest_chain, dest_asset).await {
            Ok(Some(route)) => CrossChainRoutePair {
                chain: route.destination.chain.clone(),
                asset: route.destination.asset.clone(),
                contract_address: route.destination.contract_address.clone(),
                decimals: route.destination.decimals,
                exact_out_eligible: route.exact_out_eligible,
            },
            _ => CrossChainRoutePair {
                chain: dest_chain.to_string(),
                asset: dest_asset.to_string(),
                contract_address: None,
                decimals: 0,
                exact_out_eligible: false,
            },
        };

        Ok(CrossChainPrepared {
            quote_id: quote.quote_id,
            deposit_address: quote.deposit_address,
            amount_in,
            estimated_out,
            fee_amount,
            fee_bps: quote.fee_bps,
            expires_at: quote.expires_at,
            pair: route_pair,
            recipient_address: recipient_address.to_string(),
            token_identifier,
        })
    }

    async fn send(&self, prepared: &CrossChainPrepared) -> Result<CrossChainSendResult, SdkError> {
        // Step 1: Spark transfer to the Orchestra deposit address.
        let spark_tx_hash = self
            .client
            .transfer_to_deposit(
                &prepared.deposit_address,
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
        let submit_result: Result<SubmitResponse, _> = self
            .client
            .submit_spark(flashnet::orchestra::SubmitRequestSpark {
                quote_id: prepared.quote_id.clone(),
                spark_tx_hash: spark_tx_hash.clone(),
                source_spark_address: None,
            })
            .await;

        // Step 3: Persist ConversionInfo::Orchestra metadata.
        let (status, order_id) = match &submit_result {
            Ok(response) => (crate::ConversionStatus::Pending, response.order_id.clone()),
            Err(e) => {
                error!("Orchestra /submit failed after deposit transfer {spark_tx_hash}: {e}");
                (crate::ConversionStatus::RefundNeeded, String::new())
            }
        };

        let metadata = crate::PaymentMetadata {
            conversion_info: Some(crate::ConversionInfo::Orchestra {
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

        submit_result
            .map(|r| CrossChainSendResult {
                order_id: r.order_id,
                payment_id: payment_id.clone(),
            })
            .map_err(|e| SdkError::Generic(format!("Orchestra submit failed: {e}")))
    }
}

#[allow(dead_code)]
fn _keep_status_response(_: StatusResponse) {}
