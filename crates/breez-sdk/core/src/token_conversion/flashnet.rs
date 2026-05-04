use std::{collections::HashMap, str::FromStr, sync::Arc};

use bitcoin::secp256k1::PublicKey;
use flashnet::{
    BTC_ASSET_ADDRESS, CacheStore, ClawbackRequest, ClawbackResponse, ExecuteSwapRequest,
    FlashnetClient, FlashnetConfig, FlashnetError, GetMinAmountsRequest, ListPoolsRequest,
    PoolSortOrder, SimulateSwapRequest,
};
use platform_utils::time::Duration;
use platform_utils::tokio;
use spark_wallet::{SparkWallet, TransferId};
use tokio::{
    select,
    sync::{broadcast, watch},
};
use tracing::{Instrument, debug, error, info, warn};

use crate::{
    AmountAdjustmentReason, Network, Payment, PaymentDetails, PaymentMetadata, Storage,
    persist::{StorageListPaymentsRequest, StoragePaymentDetailsFilter},
    token_conversion::{ConversionAmount, DEFAULT_CONVERSION_MAX_SLIPPAGE_BPS},
};

use super::{
    ConversionError, ConversionEstimate, ConversionInfo, ConversionOptions, ConversionPurpose,
    ConversionStatus, ConversionType, FeeSplit, FetchConversionLimitsRequest,
    FetchConversionLimitsResponse, TokenConversionPool, TokenConversionResponse, TokenConverter,
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
        let span = tracing::Span::current();

        tokio::spawn(
            async move {
                loop {
                    if let Err(e) =
                        Self::refund_failed_conversions(&storage, &flashnet_client).await
                    {
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
            }
            .instrument(span),
        );
    }

    /// Process all failed conversions needing refunds.
    async fn refund_failed_conversions(
        storage: &Arc<dyn Storage>,
        flashnet_client: &Arc<FlashnetClient>,
    ) -> Result<(), ConversionError> {
        debug!("Checking for failed conversions needing refunds");
        let payments = storage
            .list_payments(StorageListPaymentsRequest {
                payment_details_filter: Some(vec![
                    StoragePaymentDetailsFilter::Spark {
                        htlc_status: None,
                        conversion_filter: Some(crate::persist::ConversionFilter::AmmRefundNeeded),
                    },
                    StoragePaymentDetailsFilter::Token {
                        conversion_filter: Some(crate::persist::ConversionFilter::AmmRefundNeeded),
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

        let Some(ConversionInfo::Amm {
            pool_id,
            conversion_id,
            status: ConversionStatus::RefundNeeded,
            fee,
            purpose,
            amount_adjustment,
        }) = conversion_info
        else {
            return Err(ConversionError::RefundFailed(
                "Conversion is not an AMM conversion with refund pending status".into(),
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
                            conversion_info: Some(ConversionInfo::Amm {
                                pool_id: pool_id.to_string(),
                                conversion_id: conversion_id.clone(),
                                status: ConversionStatus::Refunded,
                                fee: *fee,
                                purpose: purpose.clone(),
                                amount_adjustment: amount_adjustment.clone(),
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
        token_identifier: Option<&String>,
        amount_out: u128,
    ) -> Result<ConversionEstimate, ConversionError> {
        let TokenConversionPool {
            asset_in_address,
            asset_out_address,
            pool,
        } = conversion_pool;

        // Calculate the required amount in for the desired amount out
        let calculated_amount_in = pool.calculate_amount_in(
            asset_in_address,
            amount_out,
            conversion_options
                .max_slippage_bps
                .unwrap_or(DEFAULT_CONVERSION_MAX_SLIPPAGE_BPS),
            self.integrator_fee_bps,
            self.network.into(),
        )?;

        // Apply min conversion limit floor and token dust check
        let (amount_in, amount_adjustment) = self
            .maybe_adjust_to_min_limit(conversion_options, token_identifier, calculated_amount_in)
            .await?;

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

        Ok(ConversionEstimate {
            options: conversion_options.clone(),
            amount_in,
            amount_out: response.amount_out,
            fee: response.fee_paid_asset_in.unwrap_or(0),
            amount_adjustment,
        })
    }

    /// Updates the payment with the conversion info.
    ///
    /// Arguments:
    /// * `pool_id` - The pool id used for the conversion.
    /// * `outbound_identifier` - The outbound spark transfer id or token transaction hash.
    /// * `inbound_identifier` - The inbound spark transfer id or token transaction hash if the conversion was successful.
    /// * `refund_identifier` - The inbound refund spark transfer id or token transaction hash if the conversion was refunded.
    /// * `fee_split` - The fee split between sent and received sides of the conversion.
    /// * `purpose` - The purpose of the conversion.
    ///
    /// Returns:
    /// * The sent payment id of the conversion.
    /// * The received payment id of the conversion.
    #[allow(clippy::too_many_arguments)]
    async fn update_payment_conversion_info(
        &self,
        pool_id: &PublicKey,
        outbound_identifier: String,
        inbound_identifier: Option<String>,
        refund_identifier: Option<String>,
        fee_split: Option<FeeSplit>,
        purpose: &ConversionPurpose,
        amount_adjustment: Option<AmountAdjustmentReason>,
    ) -> Result<(String, Option<String>), ConversionError> {
        let (sent_fee, received_fee) = match &fee_split {
            Some(FeeSplit::Sent(fee)) => (Some(*fee), None),
            Some(FeeSplit::Received(fee)) => (None, Some(*fee)),
            None => (None, None),
        };
        debug!(
            "Updating payment conversion info for pool_id: {pool_id}, outbound_identifier: {outbound_identifier}, inbound_identifier: {inbound_identifier:?}, refund_identifier: {refund_identifier:?}, sent_fee: {sent_fee:?}, received_fee: {received_fee:?}",
        );

        let status = match (&inbound_identifier, &refund_identifier) {
            (Some(_), _) => ConversionStatus::Completed,
            (None, Some(_)) => ConversionStatus::Refunded,
            _ => ConversionStatus::RefundNeeded,
        };
        let pool_id_str = pool_id.to_string();
        let conversion_id = uuid::Uuid::now_v7().to_string();

        // Insert sent, received, and refund payment metadata in parallel.
        let sent_fut = async {
            crate::utils::payments::insert_or_cache_payment_metadata(
                &outbound_identifier,
                PaymentMetadata {
                    conversion_info: Some(ConversionInfo::Amm {
                        pool_id: pool_id_str.clone(),
                        conversion_id: conversion_id.clone(),
                        status: status.clone(),
                        fee: sent_fee,
                        purpose: Some(purpose.clone()),
                        amount_adjustment: amount_adjustment.clone(),
                    }),
                    ..Default::default()
                },
                &self.spark_wallet,
                &self.storage,
                true,
            )
            .await
            .map_err(ConversionError::Sdk)
        };

        let received_fut = async {
            if let Some(identifier) = &inbound_identifier {
                let payment_id = crate::utils::payments::insert_or_cache_payment_metadata(
                    identifier,
                    PaymentMetadata {
                        conversion_info: Some(ConversionInfo::Amm {
                            pool_id: pool_id_str.clone(),
                            conversion_id: conversion_id.clone(),
                            status: status.clone(),
                            fee: received_fee,
                            purpose: Some(purpose.clone()),
                            amount_adjustment: None,
                        }),
                        ..Default::default()
                    },
                    &self.spark_wallet,
                    &self.storage,
                    false,
                )
                .await
                .map_err(ConversionError::Sdk)?;
                Ok::<_, ConversionError>(Some(payment_id))
            } else {
                Ok(None)
            }
        };

        let refund_fut = async {
            if let Some(identifier) = &refund_identifier {
                let metadata = PaymentMetadata {
                    conversion_info: Some(ConversionInfo::Amm {
                        pool_id: pool_id_str.clone(),
                        conversion_id: conversion_id.clone(),
                        status: status.clone(),
                        fee: None,
                        purpose: None,
                        amount_adjustment: None,
                    }),
                    ..Default::default()
                };
                crate::utils::payments::insert_or_cache_payment_metadata(
                    identifier,
                    metadata,
                    &self.spark_wallet,
                    &self.storage,
                    false,
                )
                .await
                .map_err(ConversionError::Sdk)?;
            }
            Ok::<_, ConversionError>(())
        };

        let (sent_payment_id, received_payment_id, ()) =
            tokio::try_join!(sent_fut, received_fut, refund_fut)?;

        Ok((sent_payment_id, received_payment_id))
    }

    /// For `ToBitcoin` conversions, ensures `amount_in` meets the min conversion limit
    /// and avoids leaving token dust (a balance below the min).
    ///
    /// Returns `(adjusted_amount_in, adjustment_reason)`.
    async fn maybe_adjust_to_min_limit(
        &self,
        options: &ConversionOptions,
        token_identifier: Option<&String>,
        amount_in: u128,
    ) -> Result<(u128, Option<AmountAdjustmentReason>), ConversionError> {
        let ConversionType::ToBitcoin {
            from_token_identifier,
        } = &options.conversion_type
        else {
            return Ok((amount_in, None));
        };

        // Fetch ToBitcoin limits (denominated in token units)
        let limits = self
            .fetch_limits(&FetchConversionLimitsRequest {
                conversion_type: options.conversion_type.clone(),
                token_identifier: token_identifier.cloned(),
            })
            .await?;

        let Some(min_from_amount) = limits.min_from_amount else {
            return Ok((amount_in, None));
        };

        // Floor check: ensure amount_in meets the min conversion limit
        let adjusted = amount_in.max(min_from_amount);

        // Dust check: if converting would leave a token balance below the min,
        // convert all tokens to avoid dust
        let token_balances = self.spark_wallet.get_token_balances().await?;
        let token_balance = token_balances
            .get(from_token_identifier)
            .map_or(0, |b| b.balance);

        let remaining = token_balance.saturating_sub(adjusted);
        if remaining > 0 && remaining < min_from_amount {
            info!(
                "Adjusting ToBitcoin conversion to avoid token dust: \
                 converting all {token_balance} tokens (remaining {remaining} < min {min_from_amount})"
            );
            return Ok((
                token_balance,
                Some(AmountAdjustmentReason::IncreasedToAvoidDust),
            ));
        }

        if adjusted != amount_in {
            info!(
                "Floored ToBitcoin conversion amount_in from {amount_in} to {adjusted} (min {min_from_amount})"
            );
        }

        Ok((
            adjusted,
            if adjusted == amount_in {
                None
            } else {
                Some(AmountAdjustmentReason::FlooredToMinLimit)
            },
        ))
    }

    /// Resolves a `ConversionAmount` into `(amount_in, amount_out, amount_adjustment)`.
    ///
    /// For `MinAmountOut`: uses `estimate_internal` to compute `amount_in` from desired output.
    /// For `AmountIn`: simulates the swap to compute expected `amount_out`.
    async fn resolve_amount(
        &self,
        options: &ConversionOptions,
        token_identifier: Option<&String>,
        amount: &ConversionAmount,
    ) -> Result<(u128, u128, Option<AmountAdjustmentReason>), ConversionError> {
        let estimate = self
            .validate(Some(options), token_identifier, amount.clone())
            .await?
            .ok_or(ConversionError::ConversionFailed(
                "No conversion estimate available".to_string(),
            ))?;

        // For MinAmountOut, use the original requested minimum as min_amount_out
        // (not the simulated output, which may be higher and cause unnecessary failures).
        // For AmountIn, use the slippage-adjusted estimated output.
        let min_amount_out = match amount {
            ConversionAmount::MinAmountOut(min_out) => *min_out,
            ConversionAmount::AmountIn(_) => estimate.amount_out,
        };
        Ok((
            estimate.amount_in,
            min_amount_out,
            estimate.amount_adjustment,
        ))
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
        transfer_id: Option<TransferId>,
    ) -> Result<TokenConversionResponse, ConversionError> {
        // Determine amount_in and min_amount_out based on ConversionAmount variant
        let (amount_in, min_amount_out, amount_adjustment): (
            u128,
            u128,
            Option<AmountAdjustmentReason>,
        ) = self
            .resolve_amount(options, token_identifier, &amount)
            .await?;

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
                transfer_id,
            })
            .await;

        match response_res {
            Ok(response) => {
                debug!(
                    "Conversion executed: accepted {}, error {:?}, fee_amount: {:?}",
                    response.accepted, response.error, response.fee_amount,
                );
                // Fee from ExecuteSwapResponse is denominated in the non-BTC asset (token units).
                // Route to the token-side payment: sent if asset_in is the token, received if
                // asset_in is BTC (meaning the token is on the received side).
                let fee_split = response.fee_amount.map(|fee| {
                    if conversion_pool.asset_in_address == BTC_ASSET_ADDRESS {
                        FeeSplit::Received(fee)
                    } else {
                        FeeSplit::Sent(fee)
                    }
                });

                let (sent_payment_id, received_payment_id) =
                    Box::pin(self.update_payment_conversion_info(
                        &pool_id,
                        response.transfer_id,
                        response.outbound_transfer_id,
                        response.refund_transfer_id,
                        fee_split,
                        purpose,
                        amount_adjustment.clone(),
                    ))
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
                    let _ = Box::pin(self.update_payment_conversion_info(
                        &pool_id,
                        transaction_identifier.clone(),
                        None,
                        None,
                        None,
                        purpose,
                        amount_adjustment.clone(),
                    ))
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
        amount: ConversionAmount,
    ) -> Result<Option<ConversionEstimate>, ConversionError> {
        let Some(options) = options else {
            return Ok(None);
        };

        match amount {
            ConversionAmount::MinAmountOut(amount_out) => {
                // Estimates the amount in from desired output
                let conversion_pool = self
                    .get_conversion_pool(options, token_identifier, amount_out)
                    .await?;
                self.estimate_internal(&conversion_pool, options, token_identifier, amount_out)
                    .await
                    .map(Some)
            }
            ConversionAmount::AmountIn(amount_in) => {
                // Simulate to get expected output and fee
                let conversion_pool = self
                    .get_conversion_pool(options, token_identifier, 0)
                    .await?;
                let response = self
                    .flashnet_client
                    .simulate_swap(SimulateSwapRequest {
                        asset_in_address: conversion_pool.asset_in_address.clone(),
                        asset_out_address: conversion_pool.asset_out_address.clone(),
                        pool_id: conversion_pool.pool.lp_public_key,
                        amount_in,
                        integrator_bps: if self.integrator_fee_bps > 0 {
                            Some(self.integrator_fee_bps)
                        } else {
                            None
                        },
                    })
                    .await?;

                let max_slippage = options
                    .max_slippage_bps
                    .unwrap_or(DEFAULT_CONVERSION_MAX_SLIPPAGE_BPS);
                let estimated_out = response
                    .amount_out
                    .saturating_mul(10_000u128.saturating_sub(u128::from(max_slippage)))
                    .saturating_div(10_000);

                Ok(Some(ConversionEstimate {
                    options: options.clone(),
                    amount_in,
                    amount_out: estimated_out,
                    fee: response.fee_paid_asset_in.unwrap_or(0),
                    amount_adjustment: None,
                }))
            }
        }
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
