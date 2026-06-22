use std::{collections::HashMap, str::FromStr, sync::Arc, time::Duration};

use bitcoin::secp256k1::PublicKey;
use flashnet::{
    AssetTransfer, BTC_ASSET_ADDRESS, CacheStore, ClawbackRequest, ClawbackTransfer,
    ExecuteSwapRequest, ExecuteSwapResponse, FlashnetClient, FlashnetConfig, FlashnetError,
    GetMinAmountsRequest, ListClawbackTransfersRequest, ListPoolsRequest, PoolSortOrder,
    SimulateSwapRequest,
};
use platform_utils::time::{SystemTime, UNIX_EPOCH};
use spark_wallet::{SparkWallet, TransferId};
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

use crate::{
    AmountAdjustmentReason, EventEmitter, Network, Payment, PaymentDetails, PaymentMetadata,
    RefundPendingConversionsResponse, Storage,
    persist::{StorageListPaymentsRequest, StoragePaymentDetailsFilter},
    token_conversion::{ConversionAmount, DEFAULT_CONVERSION_MAX_SLIPPAGE_BPS},
    utils::{
        payments::{
            fetch_and_process_payment, insert_payment_with_metadata,
            resolve_and_insert_payment_metadata,
        },
        polling::{PollSchedule, poll_until},
    },
};

use super::{
    ConversionError, ConversionEstimate, ConversionInfo, ConversionOptions, ConversionPurpose,
    ConversionStatus, ConversionType, FeeSplit, FetchConversionLimitsRequest,
    FetchConversionLimitsResponse, TokenConversionPool, TokenConversionResponse, TokenConverter,
};

// Polling cadence for the received leg of a freshly-completed conversion.
// The pool typically takes 1-3 seconds to advance its outbound transfer to
// the claimable state, so we keep the timeout modest — beyond that, the
// host's next sync_wallet picks up the leg.
const RECEIVED_LEG_POLL_INITIAL_DELAY_MS: u64 = 500;
const RECEIVED_LEG_POLL_MAX_DELAY_MS: u64 = 2000;
const RECEIVED_LEG_POLL_TIMEOUT_SECS: u64 = 15;

/// Min age before reconcile claws a transfer back — guards against racing a
/// concurrent same-seed instance's in-flight healthy swap.
const RECONCILE_MIN_AGE_SECS: u64 = 300;
/// Reconcile listing page size. Not paginated — subsequent inits pick up any
/// overflow.
const RECONCILE_LISTING_LIMIT: u32 = 100;

/// Returns true when the transfer is older than `cutoff_secs`, i.e. eligible
/// for clawback. Backend emits RFC 3339; missing or unparseable timestamps
/// fall back to eligible so reconcile trusts Flashnet's clawback-eligibility
/// verdict instead of silently no-op'ing on an unexpected format. The
/// unparseable branch logs at `warn` so operators see format drift.
fn transfer_is_older_than(transfer: &ClawbackTransfer, cutoff_secs: u64) -> bool {
    let Some(created_at) = transfer.created_at.as_deref() else {
        return true;
    };
    if let Some(secs) = chrono::DateTime::parse_from_rfc3339(created_at)
        .ok()
        .and_then(|dt| u64::try_from(dt.timestamp()).ok())
    {
        secs < cutoff_secs
    } else {
        warn!(
            "Reconcile: unparseable created_at {created_at:?} on clawback transfer {}; treating as eligible",
            transfer.id
        );
        true
    }
}

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
    /// # Arguments
    /// * `storage` - Storage for payment lookups and metadata updates
    /// * `spark_wallet` - Spark wallet for transfer/transaction lookups
    /// * `network` - The network configuration
    pub fn new(
        flashnet_config: FlashnetConfig,
        storage: Arc<dyn Storage>,
        spark_wallet: Arc<SparkWallet>,
        network: Network,
        http_client: Arc<dyn platform_utils::HttpClient>,
    ) -> Self {
        let integrator_fee_bps = flashnet_config
            .integrator_config
            .as_ref()
            .map_or(0, |c| c.fee_bps);

        let flashnet_client = Arc::new(FlashnetClient::new(
            flashnet_config,
            spark_wallet.clone(),
            Arc::new(CacheStore::default()),
            http_client,
        ));

        let (refund_trigger, _) = broadcast::channel(10);

        Self {
            flashnet_client,
            storage,
            spark_wallet,
            network,
            refund_trigger,
            integrator_fee_bps,
        }
    }

    /// Refunds rows marked `ConversionStatus::RefundNeeded`.
    async fn refund_failed_conversions(
        &self,
    ) -> Result<RefundPendingConversionsResponse, ConversionError> {
        debug!("Checking for failed conversions needing refunds");
        let payments = self
            .storage
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

        let mut report = RefundPendingConversionsResponse::default();
        for payment in payments {
            match self.refund_payment(&payment).await {
                Ok(true) => report.refunded = report.refunded.saturating_add(1),
                Ok(false) => report.failed = report.failed.saturating_add(1),
                Err(e) => {
                    error!(
                        "Failed to refund conversion for payment {}: {e:?}",
                        payment.id
                    );
                    report.failed = report.failed.saturating_add(1);
                }
            }
        }
        Ok(report)
    }

    /// Refund a single locally-tracked failed conversion.
    async fn refund_payment(&self, payment: &Payment) -> Result<bool, ConversionError> {
        let (clawback_id, conversion_info) = match &payment.details {
            Some(PaymentDetails::Spark {
                conversion_info, ..
            }) => (payment.id.clone(), conversion_info.as_ref()),
            Some(PaymentDetails::Token {
                tx_hash,
                conversion_info,
                ..
            }) => (tx_hash.clone(), conversion_info.as_ref()),
            _ => {
                return Err(ConversionError::RefundFailed(
                    "Payment is not a Spark or Token conversion".into(),
                ));
            }
        };
        let Some(ConversionInfo::Amm {
            pool_id,
            status: ConversionStatus::RefundNeeded,
            ..
        }) = conversion_info
        else {
            return Err(ConversionError::RefundFailed(
                "Conversion is not an AMM conversion with refund pending status".into(),
            ));
        };
        let pool_id = PublicKey::from_str(pool_id).map_err(|e| {
            ConversionError::RefundFailed(format!("Invalid pool_id {pool_id}: {e}"))
        })?;
        debug!(
            "Conversion refund needed for payment {}: pool_id {pool_id}",
            payment.id
        );
        // payment.id is the storage row id — pass it as the hint so the
        // shared helper skips the operator round-trip.
        self.clawback_and_record_refunded(&clawback_id, pool_id, Some(payment.id.clone()))
            .await
    }

    /// Refunds transfers Flashnet flags as clawback-eligible. Catches the
    /// cases the local refunder can't see (storage write failed, process
    /// killed before mark).
    async fn reconcile_with_flashnet(
        &self,
    ) -> Result<RefundPendingConversionsResponse, ConversionError> {
        let clawback_transfers = self
            .flashnet_client
            .list_clawback_transfers(ListClawbackTransfersRequest {
                limit: Some(RECONCILE_LISTING_LIMIT),
                offset: None,
            })
            .await?;

        debug!(
            "Reconcile: Flashnet listing returned {} clawback transfers",
            clawback_transfers.transfers.len()
        );

        let cutoff_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
            .saturating_sub(RECONCILE_MIN_AGE_SECS);
        let mut res = RefundPendingConversionsResponse::default();
        for transfer in clawback_transfers.transfers {
            if !transfer_is_older_than(&transfer, cutoff_secs) {
                debug!("Reconcile: skipping {} (within age threshold)", transfer.id);
                res.skipped = res.skipped.saturating_add(1);
                continue;
            }
            match self
                .clawback_and_record_refunded(&transfer.id, transfer.lp_identity_public_key, None)
                .await
            {
                Ok(true) => res.refunded = res.refunded.saturating_add(1),
                Ok(false) | Err(_) => res.failed = res.failed.saturating_add(1),
            }
        }
        if res.refunded > 0 || res.skipped > 0 || res.failed > 0 {
            info!(
                "Reconcile pass complete: refunded={}, skipped={}, failed={}",
                res.refunded, res.skipped, res.failed
            );
        }
        Ok(res)
    }

    /// Claws back one transfer. `Ok(true)` = Flashnet accepted and the
    /// local `Refunded` metadata write was attempted (the write itself
    /// propagates as `Err` on failure; resolution misses are silent).
    /// `Ok(false)` = Flashnet rejected, local state untouched so the next
    /// pass retries. Pass `payment_id` when known to skip resolution.
    async fn clawback_and_record_refunded(
        &self,
        clawback_id: &str,
        pool_id: PublicKey,
        payment_id: Option<String>,
    ) -> Result<bool, ConversionError> {
        match self
            .flashnet_client
            .clawback(ClawbackRequest {
                pool_id,
                transfer_id: clawback_id.to_string(),
            })
            .await
        {
            Ok(r) if r.accepted => {
                debug!(
                    "Clawback accepted for {clawback_id}: tracking_id={}",
                    r.spark_status_tracking_id
                );
            }
            Ok(r) => {
                warn!("Clawback for {clawback_id} not accepted: {:?}", r.error);
                return Ok(false);
            }
            Err(e) => {
                error!("Clawback for {clawback_id} failed: {e}");
                return Err(e.into());
            }
        }

        // Preserve the prior ConversionInfo::Amm fields when we can read it;
        // fall back to a placeholder when storage has nothing (reconcile path
        // recovering a row whose original metadata write never landed).
        let prev_amm = self
            .storage
            .get_payment_by_id(
                payment_id
                    .clone()
                    .unwrap_or_else(|| clawback_id.to_string()),
            )
            .await
            .ok()
            .and_then(|p| crate::utils::conversions::extract_conversion_info(p.details));
        let conversion_info = match prev_amm {
            Some(ConversionInfo::Amm {
                pool_id: prev_pool_id,
                conversion_id,
                fee,
                purpose,
                amount_adjustment,
                ..
            }) => ConversionInfo::Amm {
                pool_id: prev_pool_id,
                conversion_id,
                status: ConversionStatus::Refunded,
                fee,
                purpose,
                amount_adjustment,
            },
            _ => ConversionInfo::Amm {
                pool_id: pool_id.to_string(),
                conversion_id: uuid::Uuid::now_v7().to_string(),
                status: ConversionStatus::Refunded,
                fee: None,
                purpose: None,
                amount_adjustment: None,
            },
        };
        let metadata = PaymentMetadata {
            conversion_info: Some(conversion_info),
            ..Default::default()
        };

        // With a payment_id hint, skip resolution and write directly.
        // Without one, use the shared helper: it resolves (operator round-
        // trip for tokens) and caches the metadata for the next sync if
        // resolution fails. That cache fallback covers the otherwise-silent
        // "clawback accepted server-side, local row unreachable" case.
        match payment_id {
            Some(id) => {
                self.storage
                    .insert_payment_metadata(id, metadata)
                    .await
                    .map_err(|e| {
                        warn!("Metadata write failed for {clawback_id}: {e}");
                        ConversionError::from(e)
                    })?;
            }
            None => {
                resolve_and_insert_payment_metadata(
                    clawback_id,
                    metadata,
                    &self.spark_wallet,
                    &self.storage,
                    true,
                )
                .await
                .map_err(ConversionError::Sdk)?;
            }
        }
        Ok(true)
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
    /// * `outbound_asset_transfer` - The outbound `AssetTransfer` for the sent leg.
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
        outbound_asset_transfer: &AssetTransfer,
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
        let outbound_identifier = outbound_asset_transfer.id();
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
        // The sent leg uses the AssetTransfer-aware helper to skip the
        // operator round-trip; inbound/refund stay on the string-identifier
        // helper because the SDK never holds those transfers in hand
        // (they're produced by the pool, not by us).
        let sent_fut = async {
            crate::utils::conversions::resolve_and_insert_payment_metadata_for_transfer(
                outbound_asset_transfer,
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
                let payment_id = crate::utils::payments::resolve_and_insert_payment_metadata(
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
                crate::utils::payments::resolve_and_insert_payment_metadata(
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

    /// Insert local `Payment` records for both legs of a completed swap so
    /// the conversion's `from`/`to` are immediately visible to callers
    /// without waiting for the next `sync_wallet`.
    ///
    /// The sent leg uses the rich `AssetTransfer` that
    /// [`FlashnetClient::execute_swap`] hands back — no extra RPC. The
    /// received leg is fetched by id (the pool created it server-side, so
    /// we don't yet have it locally); for spark transfers we also claim
    /// them so the resulting Payment is terminal.
    async fn process_conversion_payments(
        &self,
        event_emitter: Arc<EventEmitter>,
        outbound_asset_transfer: AssetTransfer,
        sent_payment_id: &str,
        received_payment_id: &str,
    ) {
        // Sent leg: we just produced this transfer ourselves, so the local
        // spark/token wallet state is already current — no claim needed.
        // Build the Payment directly and insert it even if the operator-side
        // status hasn't reached terminal yet; the next sync will promote it
        // to Completed.
        let sent_payment = self
            .build_sent_conversion_payment(outbound_asset_transfer, sent_payment_id)
            .await;
        if let Some(payment) = sent_payment {
            insert_payment_with_metadata(
                self.spark_wallet.clone(),
                self.storage.clone(),
                event_emitter.clone(),
                payment,
            )
            .await;
        }

        // Received leg: look up by id (the pool created it server-side).
        let received_payment = self
            .fetch_received_conversion_payment(received_payment_id)
            .await;
        if let Some(payment) = received_payment {
            insert_payment_with_metadata(
                self.spark_wallet.clone(),
                self.storage.clone(),
                event_emitter,
                payment,
            )
            .await;
        }
    }

    /// Builds the `Payment` record for the sent leg of a conversion
    /// using the response we already have from `execute_swap`.
    async fn build_sent_conversion_payment(
        &self,
        outbound_asset_transfer: AssetTransfer,
        sent_payment_id: &str,
    ) -> Option<Payment> {
        match crate::utils::conversions::payment_from_asset_transfer(
            outbound_asset_transfer,
            &self.spark_wallet,
            &self.storage,
            sent_payment_id,
        )
        .await
        {
            Ok(payment) => payment,
            Err(e) => {
                warn!(
                    "Failed to build Payment from sent asset transfer for conversion {sent_payment_id}: {e:?}"
                );
                None
            }
        }
    }

    /// Fetches the received leg of a conversion by its payment id and processes
    /// the transfer/token. Spark transfers are claimed locally before
    /// being returned; token outputs are already terminal once the
    /// tx is visible on operators.
    ///
    /// Polls briefly because the pool's outbound transfer typically arrives
    /// in `SenderInitiated`/`SenderKeyTweakPending` and only becomes
    /// claimable a moment later. If we're not able to fetch and process the
    /// payment, it will be done so downstream or in the next `sync_wallet` call.
    async fn fetch_received_conversion_payment(&self, payment_id: &str) -> Option<Payment> {
        let schedule = PollSchedule {
            initial_delay: Duration::from_millis(RECEIVED_LEG_POLL_INITIAL_DELAY_MS),
            max_delay: Duration::from_millis(RECEIVED_LEG_POLL_MAX_DELAY_MS),
            timeout: Duration::from_secs(RECEIVED_LEG_POLL_TIMEOUT_SECS),
        };
        let result = poll_until(schedule, None, || {
            fetch_and_process_payment(&self.spark_wallet, self.storage.clone(), payment_id, false)
        })
        .await;
        match result {
            Ok(payment) => Some(payment),
            Err(e) => {
                warn!(
                    "Failed to fetch received conversion payment {payment_id} within timeout: {e:?}"
                );
                None
            }
        }
    }
}

#[macros::async_trait]
impl TokenConverter for FlashnetTokenConverter {
    #[allow(clippy::too_many_lines)]
    async fn convert(
        &self,
        event_emitter: Arc<EventEmitter>,
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
            Ok(ExecuteSwapResponse {
                flashnet_response,
                outbound_asset_transfer,
            }) => {
                debug!(
                    "Conversion executed: accepted {}, error {:?}, fee_amount: {:?}",
                    flashnet_response.accepted,
                    flashnet_response.error,
                    flashnet_response.fee_amount,
                );
                // Fee from FlashnetExecuteSwapResponse is denominated in the non-BTC asset
                // (token units). Route to the token-side payment: sent if asset_in is the token,
                // received if asset_in is BTC (meaning the token is on the received side).
                let fee_split = flashnet_response.fee_amount.map(|fee| {
                    if conversion_pool.asset_in_address == BTC_ASSET_ADDRESS {
                        FeeSplit::Received(fee)
                    } else {
                        FeeSplit::Sent(fee)
                    }
                });

                let (sent_payment_id, received_payment_id) =
                    Box::pin(self.update_payment_conversion_info(
                        &pool_id,
                        &outbound_asset_transfer,
                        flashnet_response.outbound_transfer_id,
                        flashnet_response.refund_transfer_id,
                        fee_split,
                        purpose,
                        amount_adjustment.clone(),
                    ))
                    .await?;

                if let Some(received_payment_id) = received_payment_id
                    && flashnet_response.accepted
                {
                    self.process_conversion_payments(
                        event_emitter,
                        outbound_asset_transfer,
                        &sent_payment_id,
                        &received_payment_id,
                    )
                    .await;
                    Ok(TokenConversionResponse {
                        sent_payment_id,
                        received_payment_id,
                    })
                } else {
                    let error_message = flashnet_response
                        .error
                        .unwrap_or("Conversion not accepted".to_string());
                    Err(ConversionError::ConversionFailed(format!(
                        "Convert token failed, refund in progress: {error_message}",
                    )))
                }
            }
            Err(e) => {
                error!("Convert token failed: {e:?}");
                let FlashnetError::Execution {
                    outbound_asset_transfer: Some(transfer),
                    source,
                } = &e
                else {
                    return Err(e.into());
                };
                // Best-effort RefundNeeded mark; reconcile catches anything we miss.
                let update_res = Box::pin(self.update_payment_conversion_info(
                    &pool_id,
                    transfer,
                    None,
                    None,
                    None,
                    purpose,
                    amount_adjustment.clone(),
                ))
                .await;
                if let Err(err) = update_res {
                    warn!("Could not update {} to RefundNeeded: {err}", transfer.id());
                }
                let _ = self.refund_trigger.send(());
                Err(ConversionError::ConversionFailed(format!(
                    "Convert token failed, refund pending: {}",
                    *source.clone()
                )))
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

    async fn refund_pending(&self) -> Result<RefundPendingConversionsResponse, ConversionError> {
        let mut local_failed = false;
        let local = match self.refund_failed_conversions().await {
            Ok(r) => r,
            Err(e) => {
                warn!("Local refund pass failed: {e}");
                local_failed = true;
                RefundPendingConversionsResponse {
                    failed: 1,
                    ..Default::default()
                }
            }
        };
        let remote = match self.reconcile_with_flashnet().await {
            Ok(r) => r,
            Err(e) => {
                warn!("Reconcile with Flashnet failed: {e}");
                // Both passes errored: surface via Err instead of masking
                // with Ok. Single-pass error stays Ok with `failed += 1`,
                // since the other pass still produced counts.
                if local_failed {
                    return Err(e);
                }
                RefundPendingConversionsResponse {
                    failed: 1,
                    ..Default::default()
                }
            }
        };

        let combined = RefundPendingConversionsResponse {
            refunded: local.refunded.saturating_add(remote.refunded),
            skipped: local.skipped.saturating_add(remote.skipped),
            failed: local.failed.saturating_add(remote.failed),
        };
        if combined.refunded > 0 || combined.skipped > 0 || combined.failed > 0 {
            info!(
                "Refund-pending pass: refunded={} (local={}, remote={}), skipped={} (local={}, remote={}), failed={} (local={}, remote={})",
                combined.refunded,
                local.refunded,
                remote.refunded,
                combined.skipped,
                local.skipped,
                remote.skipped,
                combined.failed,
                local.failed,
                remote.failed,
            );
        }
        Ok(combined)
    }

    async fn refund_local_pending(
        &self,
    ) -> Result<RefundPendingConversionsResponse, ConversionError> {
        self.refund_failed_conversions().await
    }

    fn subscribe_refund_requests(&self) -> Option<broadcast::Receiver<()>> {
        Some(self.refund_trigger.subscribe())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transfer(id: &str, created_at: Option<&str>) -> ClawbackTransfer {
        ClawbackTransfer {
            id: id.to_string(),
            lp_identity_public_key: PublicKey::from_str(
                "02894808873b896e21d29856a6d7bb346fb13c019739adb9bf0b6a8b7e28da53da",
            )
            .unwrap(),
            created_at: created_at.map(str::to_string),
        }
    }

    /// Unix seconds for `2025-09-22T19:09:36Z`.
    const SAMPLE_UNIX_SECS: u64 = 1_758_568_176;

    #[test]
    fn missing_created_at_is_eligible() {
        assert!(transfer_is_older_than(&transfer("id-1", None), 0));
    }

    /// Fail-loud, not-quiet: an unrecognised timestamp must still let the
    /// clawback proceed (with a warn log) instead of being silently bucketed
    /// as `skipped`. That silent-skip is the exact class of failure this PR
    /// exists to prevent.
    #[test]
    fn unparseable_created_at_is_eligible() {
        assert!(transfer_is_older_than(
            &transfer("id-2", Some("not-a-timestamp")),
            u64::MAX,
        ));
    }

    #[test]
    fn rfc3339_z_respects_cutoff() {
        let t = transfer("id-3", Some("2025-09-22T19:09:36.661269Z"));
        assert!(transfer_is_older_than(&t, SAMPLE_UNIX_SECS + 1));
        assert!(!transfer_is_older_than(&t, SAMPLE_UNIX_SECS - 1));
    }

    #[test]
    fn rfc3339_offset_respects_cutoff() {
        let t = transfer("id-4", Some("2025-09-22T19:09:36.661269+00:00"));
        assert!(transfer_is_older_than(&t, SAMPLE_UNIX_SECS + 1));
        assert!(!transfer_is_older_than(&t, SAMPLE_UNIX_SECS - 1));
    }
}
