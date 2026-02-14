use std::{collections::HashMap, str::FromStr, sync::Arc};

use bitcoin::secp256k1::PublicKey;
use flashnet::{
    CacheStore, ClawbackRequest, ClawbackResponse, ExecuteSwapRequest, FlashnetClient,
    FlashnetConfig, FlashnetError, GetMinAmountsRequest, ListPoolsRequest, PoolSortOrder,
    SimulateSwapRequest,
};
use spark_wallet::{ListTransfersRequest, SparkWallet, TransferId};
use tokio::{
    select,
    sync::{broadcast, watch},
};
use tokio_with_wasm::alias as tokio;
use tracing::{debug, error, info, warn};
use web_time::Duration;

use crate::{
    ListPaymentsRequest, Network, Payment, PaymentDetails, PaymentDetailsFilter, PaymentMetadata,
    Storage,
    persist::ObjectCacheRepository,
    token_conversion::{ConversionAmount, DEFAULT_CONVERSION_MAX_SLIPPAGE_BPS},
    utils::token::token_transaction_to_payments,
};

use super::{
    ConversionError, ConversionEstimate, ConversionInfo, ConversionOptions, ConversionPurpose,
    ConversionStatus, FetchConversionLimitsRequest, FetchConversionLimitsResponse,
    TokenConversionPool, TokenConversionResponse, TokenConverter,
};

/// Flashnet-based implementation of the `TokenConverter` trait.
///
/// This implementation handles the mechanics of executing conversions via Flashnet,
/// including pool selection, swap execution, and refund handling.
pub(crate) struct FlashnetTokenConverter {
    flashnet_client: Arc<FlashnetClient>,
    storage: Arc<dyn Storage>,
    spark_wallet: Arc<SparkWallet>,
    network: Network,
    refund_trigger: broadcast::Sender<()>,
    integrator_fee_bps: u32,
}

impl FlashnetTokenConverter {
    /// Creates a new `FlashnetTokenConverter` instance.
    ///
    /// Spawns a background task to periodically process failed conversion refunds.
    ///
    /// # Arguments
    /// * `storage` - Storage for payment lookups and metadata updates
    /// * `spark_wallet` - Spark wallet for transfer/transaction lookups
    /// * `network` - The network configuration
    /// * `shutdown_receiver` - Watch receiver to signal shutdown of the refunder task
    pub fn new(
        flashnet_config: FlashnetConfig,
        storage: Arc<dyn Storage>,
        spark_wallet: Arc<SparkWallet>,
        network: Network,
        shutdown_receiver: watch::Receiver<()>,
    ) -> Self {
        let integrator_fee_bps = flashnet_config
            .integrator_config
            .as_ref()
            .map_or(0, |c| c.fee_bps);

        let flashnet_client = Arc::new(FlashnetClient::new(
            flashnet_config,
            spark_wallet.clone(),
            Arc::new(CacheStore::default()),
        ));

        let (refund_trigger, _) = broadcast::channel(10);

        let converter = Self {
            flashnet_client,
            storage,
            spark_wallet,
            network,
            refund_trigger: refund_trigger.clone(),
            integrator_fee_bps,
        };

        // Spawn the background refunder task
        converter.spawn_refunder(shutdown_receiver, &refund_trigger);

        converter
    }

    /// Spawns a background task that periodically checks for failed conversions
    /// and initiates refunds. Triggered on startup, by the refund trigger, and every 150 seconds.
    fn spawn_refunder(
        &self,
        mut shutdown_receiver: watch::Receiver<()>,
        refund_trigger: &broadcast::Sender<()>,
    ) {
        let storage = Arc::clone(&self.storage);
        let flashnet_client = Arc::clone(&self.flashnet_client);
        let mut trigger_receiver = refund_trigger.subscribe();

        tokio::spawn(async move {
            loop {
                if let Err(e) = Self::refund_failed_conversions(&storage, &flashnet_client).await {
                    error!("Failed to refund failed conversions: {e:?}");
                }

                select! {
                    _ = shutdown_receiver.changed() => {
                        info!("Conversion refunder shutdown signal received");
                        return;
                    }
                    _ = trigger_receiver.recv() => {
                        debug!("Conversion refunder triggered");
                    }
                    () = tokio::time::sleep(Duration::from_secs(150)) => {}
                }
            }
        });
    }

    /// Process all failed conversions needing refunds.
    async fn refund_failed_conversions(
        storage: &Arc<dyn Storage>,
        flashnet_client: &Arc<FlashnetClient>,
    ) -> Result<(), ConversionError> {
        debug!("Checking for failed conversions needing refunds");
        let payments = storage
            .list_payments(ListPaymentsRequest {
                payment_details_filter: Some(vec![
                    PaymentDetailsFilter::Spark {
                        htlc_status: None,
                        conversion_refund_needed: Some(true),
                    },
                    PaymentDetailsFilter::Token {
                        conversion_refund_needed: Some(true),
                        tx_hash: None,
                        tx_type: None,
                    },
                ]),
                ..Default::default()
            })
            .await?;

        debug!(
            "Found {} payments needing conversion refunds",
            payments.len()
        );

        for payment in payments {
            if let Err(e) = Self::refund_payment(storage, flashnet_client, &payment).await {
                error!(
                    "Failed to refund conversion for payment {}: {e:?}",
                    payment.id
                );
            }
        }
        Ok(())
    }

    /// Refund a single failed conversion payment.
    async fn refund_payment(
        storage: &Arc<dyn Storage>,
        flashnet_client: &Arc<FlashnetClient>,
        payment: &Payment,
    ) -> Result<(), ConversionError> {
        let (clawback_id, conversion_info) = match &payment.details {
            Some(PaymentDetails::Spark {
                conversion_info, ..
            }) => (payment.id.clone(), conversion_info),
            Some(PaymentDetails::Token {
                tx_hash,
                conversion_info,
                ..
            }) => (tx_hash.clone(), conversion_info),
            _ => {
                return Err(ConversionError::RefundFailed(
                    "Payment is not a Spark or Conversion".into(),
                ));
            }
        };

        let Some(ConversionInfo {
            pool_id,
            conversion_id,
            status: ConversionStatus::RefundNeeded,
            fee,
            purpose,
        }) = conversion_info
        else {
            return Err(ConversionError::RefundFailed(
                "Conversion does not have a refund pending status".into(),
            ));
        };

        debug!(
            "Conversion refund needed for payment {}: pool_id {pool_id}",
            payment.id
        );

        let Ok(pool_id) = PublicKey::from_str(pool_id) else {
            return Err(ConversionError::RefundFailed(format!(
                "Invalid pool_id: {pool_id}"
            )));
        };

        match flashnet_client
            .clawback(ClawbackRequest {
                pool_id,
                transfer_id: clawback_id,
            })
            .await
        {
            Ok(ClawbackResponse {
                accepted: true,
                spark_status_tracking_id,
                ..
            }) => {
                debug!(
                    "Clawback initiated for payment {}: tracking_id: {}",
                    payment.id, spark_status_tracking_id
                );
                // Update the payment metadata to reflect the refund status
                storage
                    .insert_payment_metadata(
                        payment.id.clone(),
                        PaymentMetadata {
                            conversion_info: Some(ConversionInfo {
                                pool_id: pool_id.to_string(),
                                conversion_id: conversion_id.clone(),
                                status: ConversionStatus::Refunded,
                                fee: *fee,
                                purpose: purpose.clone(),
                            }),
                            ..Default::default()
                        },
                    )
                    .await?;
                Ok(())
            }
            Ok(ClawbackResponse {
                accepted: false,
                request_id,
                error,
                ..
            }) => Err(ConversionError::RefundFailed(format!(
                "Clawback not accepted: request_id: {request_id:?}, error: {error:?}"
            ))),
            Err(e) => Err(ConversionError::RefundFailed(format!(
                "Failed to initiate clawback: {e}"
            ))),
        }
    }

    /// Gets the best conversion pool for the given conversion options and amount.
    async fn get_conversion_pool(
        &self,
        conversion_options: &ConversionOptions,
        token_identifier: Option<&String>,
        amount_out: u128,
    ) -> Result<TokenConversionPool, ConversionError> {
        let conversion_type = &conversion_options.conversion_type;
        let (asset_in_address, asset_out_address) =
            conversion_type.as_asset_addresses(token_identifier)?;

        // List available pools for the asset pair in both directions
        let a_in_pools_fut = self.flashnet_client.list_pools(ListPoolsRequest {
            asset_a_address: Some(asset_in_address.clone()),
            asset_b_address: Some(asset_out_address.clone()),
            sort: Some(PoolSortOrder::Volume24hDesc),
            ..Default::default()
        });
        let b_in_pools_fut = self.flashnet_client.list_pools(ListPoolsRequest {
            asset_a_address: Some(asset_out_address.clone()),
            asset_b_address: Some(asset_in_address.clone()),
            sort: Some(PoolSortOrder::Volume24hDesc),
            ..Default::default()
        });
        let (a_in_pools_res, b_in_pools_res) = tokio::join!(a_in_pools_fut, b_in_pools_fut);

        // Merge pools by pool_id to avoid duplicates
        let mut pools = a_in_pools_res.map_or(HashMap::new(), |res| {
            res.pools
                .into_iter()
                .map(|pool| (pool.lp_public_key, pool))
                .collect::<HashMap<_, _>>()
        });
        if let Ok(res) = b_in_pools_res {
            pools.extend(res.pools.into_iter().map(|pool| (pool.lp_public_key, pool)));
        }
        let pools = pools.into_values().collect::<Vec<_>>();

        if pools.is_empty() {
            warn!(
                "No conversion pools available: in address {asset_in_address}, out address {asset_out_address}",
            );
            return Err(ConversionError::NoPoolsAvailable);
        }

        // Extract max_slippage_bps with default fallback
        let max_slippage_bps = conversion_options
            .max_slippage_bps
            .unwrap_or(DEFAULT_CONVERSION_MAX_SLIPPAGE_BPS);

        // Select the best pool using multi-factor scoring
        let pool = flashnet::select_best_pool(
            &pools,
            &asset_in_address,
            amount_out,
            max_slippage_bps,
            self.integrator_fee_bps,
            self.network.into(),
        )?;

        Ok(TokenConversionPool {
            asset_in_address,
            asset_out_address,
            pool,
        })
    }

    /// Validates a conversion internally and returns the estimate.
    async fn estimate_internal(
        &self,
        conversion_pool: &TokenConversionPool,
        conversion_options: &ConversionOptions,
        amount_out: u128,
    ) -> Result<Option<ConversionEstimate>, ConversionError> {
        let TokenConversionPool {
            asset_in_address,
            asset_out_address,
            pool,
        } = conversion_pool;

        // Calculate the required amount in for the desired amount out
        let amount_in = pool.calculate_amount_in(
            asset_in_address,
            amount_out,
            conversion_options
                .max_slippage_bps
                .unwrap_or(DEFAULT_CONVERSION_MAX_SLIPPAGE_BPS),
            self.integrator_fee_bps,
            self.network.into(),
        )?;

        // Simulate the swap to validate the conversion
        let response = self
            .flashnet_client
            .simulate_swap(SimulateSwapRequest {
                asset_in_address: asset_in_address.clone(),
                asset_out_address: asset_out_address.clone(),
                pool_id: pool.lp_public_key,
                amount_in,
                integrator_bps: if self.integrator_fee_bps > 0 {
                    Some(self.integrator_fee_bps)
                } else {
                    None
                },
            })
            .await?;

        if response.amount_out < amount_out {
            return Err(ConversionError::ValidationFailed(format!(
                "Validation returned {} but expected at least {amount_out}",
                response.amount_out
            )));
        }

        Ok(response.fee_paid_asset_in.map(|fee| ConversionEstimate {
            options: conversion_options.clone(),
            amount: amount_in,
            fee,
        }))
    }

    /// Fetches a payment by its conversion identifier.
    /// The identifier can be either a spark transfer id or a token transaction hash.
    async fn fetch_payment_by_identifier(
        &self,
        identifier: &str,
        tx_inputs_are_ours: bool,
    ) -> Result<Payment, ConversionError> {
        debug!("Fetching conversion payment for identifier: {}", identifier);

        let payment: Result<Payment, ConversionError> = if let Ok(transfer_id) =
            TransferId::from_str(identifier)
        {
            // It's a spark transfer id
            let transfers = self
                .spark_wallet
                .list_transfers(ListTransfersRequest {
                    transfer_ids: vec![transfer_id],
                    ..Default::default()
                })
                .await?;
            let transfer =
                transfers.items.first().cloned().ok_or_else(|| {
                    ConversionError::ConversionFailed("Transfer not found".into())
                })?;
            transfer
                .try_into()
                .map_err(|e: crate::SdkError| ConversionError::Sdk(e))
        } else {
            // It's a token transaction hash
            let token_transactions = self
                .spark_wallet
                .get_token_transactions_by_hashes(vec![identifier.to_string()])
                .await?;
            let token_transaction = token_transactions.first().ok_or_else(|| {
                ConversionError::ConversionFailed("Token transaction not found".into())
            })?;
            let object_repository = ObjectCacheRepository::new(self.storage.clone());
            let payments = token_transaction_to_payments(
                &self.spark_wallet,
                &object_repository,
                token_transaction,
                tx_inputs_are_ours,
            )
            .await
            .map_err(ConversionError::Sdk)?;
            payments.first().cloned().ok_or_else(|| {
                ConversionError::ConversionFailed("Payment not found for token transaction".into())
            })
        };

        payment
            .inspect(|p| debug!("Found payment: {p:?}"))
            .inspect_err(|e| debug!("No payment found: {e}"))
    }

    /// Updates the payment with the conversion info.
    ///
    /// Arguments:
    /// * `pool_id` - The pool id used for the conversion.
    /// * `outbound_identifier` - The outbound spark transfer id or token transaction hash.
    /// * `inbound_identifier` - The inbound spark transfer id or token transaction hash if the conversion was successful.
    /// * `refund_identifier` - The inbound refund spark transfer id or token transaction hash if the conversion was refunded.
    /// * `fee` - The fee paid for the conversion.
    /// * `purpose` - The purpose of the conversion.
    ///
    /// Returns:
    /// * The sent payment id of the conversion.
    /// * The received payment id of the conversion.
    async fn update_payment_conversion_info(
        &self,
        pool_id: &PublicKey,
        outbound_identifier: String,
        inbound_identifier: Option<String>,
        refund_identifier: Option<String>,
        fee: Option<u128>,
        purpose: &ConversionPurpose,
    ) -> Result<(String, Option<String>), ConversionError> {
        debug!(
            "Updating payment conversion info for pool_id: {pool_id}, outbound_identifier: {outbound_identifier}, inbound_identifier: {inbound_identifier:?}, refund_identifier: {refund_identifier:?}"
        );

        let cache = ObjectCacheRepository::new(self.storage.clone());
        let status = match (&inbound_identifier, &refund_identifier) {
            (Some(_), _) => ConversionStatus::Completed,
            (None, Some(_)) => ConversionStatus::Refunded,
            _ => ConversionStatus::RefundNeeded,
        };
        let pool_id_str = pool_id.to_string();
        let conversion_id = uuid::Uuid::now_v7().to_string();

        // Save the sent payment metadata to cache so it's picked up during sync.
        // We don't insert the payment directly to storage here - sync will do that
        // and emit the appropriate PaymentSucceeded event.
        let sent_payment_id = self
            .fetch_payment_by_identifier(&outbound_identifier, true)
            .await?
            .id;
        cache
            .save_payment_metadata(
                &sent_payment_id,
                &PaymentMetadata {
                    conversion_info: Some(ConversionInfo {
                        pool_id: pool_id_str.clone(),
                        conversion_id: conversion_id.clone(),
                        status: status.clone(),
                        fee,
                        purpose: None,
                    }),
                    ..Default::default()
                },
            )
            .await?;

        // Update the received payment metadata if available
        let received_payment_id = if let Some(identifier) = &inbound_identifier {
            let metadata = PaymentMetadata {
                conversion_info: Some(ConversionInfo {
                    pool_id: pool_id_str.clone(),
                    conversion_id: conversion_id.clone(),
                    status: status.clone(),
                    fee: None,
                    purpose: Some(purpose.clone()),
                }),
                ..Default::default()
            };
            if let Ok(payment) = self.fetch_payment_by_identifier(identifier, false).await {
                self.storage
                    .insert_payment_metadata(payment.id.clone(), metadata)
                    .await?;
                Some(payment.id)
            } else {
                cache.save_payment_metadata(identifier, &metadata).await?;
                Some(identifier.clone())
            }
        } else {
            None
        };

        // Update the refund payment metadata if available
        if let Some(identifier) = &refund_identifier {
            let metadata = PaymentMetadata {
                conversion_info: Some(ConversionInfo {
                    pool_id: pool_id_str,
                    conversion_id,
                    status,
                    fee: None,
                    purpose: None,
                }),
                ..Default::default()
            };
            if let Ok(payment) = self.fetch_payment_by_identifier(identifier, false).await {
                self.storage
                    .insert_payment_metadata(payment.id.clone(), metadata)
                    .await?;
            } else {
                cache.save_payment_metadata(identifier, &metadata).await?;
            }
        }

        Ok((sent_payment_id, received_payment_id))
    }
}

#[macros::async_trait]
impl TokenConverter for FlashnetTokenConverter {
    #[allow(clippy::too_many_lines)]
    async fn convert(
        &self,
        options: &ConversionOptions,
        purpose: &ConversionPurpose,
        token_identifier: Option<&String>,
        amount: ConversionAmount,
    ) -> Result<TokenConversionResponse, ConversionError> {
        // Determine amount_in and min_amount_out based on ConversionAmount variant
        let (amount_in, min_amount_out) = match amount {
            ConversionAmount::MinAmountOut(min_out) => {
                // Calculate amount_in from min_amount_out
                let conversion_pool = self
                    .get_conversion_pool(options, token_identifier, min_out)
                    .await?;
                let estimate = self
                    .estimate_internal(&conversion_pool, options, min_out)
                    .await?
                    .ok_or(ConversionError::ConversionFailed(
                        "No conversion estimate available".to_string(),
                    ))?;
                (estimate.amount, min_out)
            }
            ConversionAmount::AmountIn(amount_in) => {
                // We have the input, simulate to get expected output
                let conversion_pool = self
                    .get_conversion_pool(options, token_identifier, 0)
                    .await?;

                let simulate_response = self
                    .flashnet_client
                    .simulate_swap(SimulateSwapRequest {
                        asset_in_address: conversion_pool.asset_in_address.clone(),
                        asset_out_address: conversion_pool.asset_out_address.clone(),
                        pool_id: conversion_pool.pool.lp_public_key,
                        amount_in,
                        integrator_bps: None,
                    })
                    .await?;

                // Apply slippage tolerance to get minimum acceptable output
                let max_slippage = options
                    .max_slippage_bps
                    .unwrap_or(DEFAULT_CONVERSION_MAX_SLIPPAGE_BPS);
                let min_out = simulate_response
                    .amount_out
                    .saturating_mul(10_000u128.saturating_sub(u128::from(max_slippage)))
                    .saturating_div(10_000);

                (amount_in, min_out)
            }
        };

        // Get the conversion pool for execution
        let conversion_pool = self
            .get_conversion_pool(options, token_identifier, min_amount_out)
            .await?;
        let pool_id = conversion_pool.pool.lp_public_key;

        // Execute the conversion
        let response_res = self
            .flashnet_client
            .execute_swap(ExecuteSwapRequest {
                asset_in_address: conversion_pool.asset_in_address.clone(),
                asset_out_address: conversion_pool.asset_out_address.clone(),
                pool_id,
                amount_in,
                max_slippage_bps: options
                    .max_slippage_bps
                    .unwrap_or(DEFAULT_CONVERSION_MAX_SLIPPAGE_BPS),
                min_amount_out,
                integrator_fee_rate_bps: None,
                integrator_public_key: None,
            })
            .await;

        match response_res {
            Ok(response) => {
                info!(
                    "Conversion executed: accepted {}, error {:?}",
                    response.accepted, response.error
                );
                let (sent_payment_id, received_payment_id) = self
                    .update_payment_conversion_info(
                        &pool_id,
                        response.transfer_id,
                        response.outbound_transfer_id,
                        response.refund_transfer_id,
                        response.fee_amount,
                        purpose,
                    )
                    .await?;

                if let Some(received_payment_id) = received_payment_id
                    && response.accepted
                {
                    Ok(TokenConversionResponse {
                        sent_payment_id,
                        received_payment_id,
                    })
                } else {
                    let error_message = response
                        .error
                        .unwrap_or("Conversion not accepted".to_string());
                    Err(ConversionError::ConversionFailed(format!(
                        "Convert token failed, refund in progress: {error_message}",
                    )))
                }
            }
            Err(e) => {
                error!("Convert token failed: {e:?}");
                if let FlashnetError::Execution {
                    transaction_identifier: Some(transaction_identifier),
                    source,
                } = &e
                {
                    let _ = self
                        .update_payment_conversion_info(
                            &pool_id,
                            transaction_identifier.clone(),
                            None,
                            None,
                            None,
                            purpose,
                        )
                        .await;
                    let _ = self.refund_trigger.send(());
                    Err(ConversionError::ConversionFailed(format!(
                        "Convert token failed, refund pending: {}",
                        *source.clone()
                    )))
                } else {
                    Err(e.into())
                }
            }
        }
    }

    async fn validate(
        &self,
        options: Option<&ConversionOptions>,
        token_identifier: Option<&String>,
        amount_out: u128,
    ) -> Result<Option<ConversionEstimate>, ConversionError> {
        let Some(options) = options else {
            return Ok(None);
        };

        let conversion_pool = self
            .get_conversion_pool(options, token_identifier, amount_out)
            .await?;

        self.estimate_internal(&conversion_pool, options, amount_out)
            .await
    }

    async fn fetch_limits(
        &self,
        request: &FetchConversionLimitsRequest,
    ) -> Result<FetchConversionLimitsResponse, ConversionError> {
        let (asset_in_address, asset_out_address) = request
            .conversion_type
            .as_asset_addresses(request.token_identifier.as_ref())?;

        let min_amounts = self
            .flashnet_client
            .get_min_amounts(GetMinAmountsRequest {
                asset_in_address,
                asset_out_address,
            })
            .await?;

        Ok(FetchConversionLimitsResponse {
            min_from_amount: min_amounts.asset_in_min,
            min_to_amount: min_amounts.asset_out_min,
        })
    }
}
