use std::{collections::HashMap, str::FromStr, sync::Arc, time::Duration};

use bitcoin::secp256k1::PublicKey;
use flashnet::{
    AssetTransfer, BTC_ASSET_ADDRESS, CacheStore, ClawbackRequest, ClawbackTransfer,
    ExecuteSwapRequest, ExecuteSwapResponse, FlashnetClient, FlashnetConfig, FlashnetError,
    GetMinAmountsRequest, ListClawbackTransfersRequest, ListPoolsRequest, ListUserSwapsRequest,
    PoolSortOrder, SimulateSwapRequest, Swap, SwapOutcome, SwapSortOrder,
};
use platform_utils::time::{SystemTime, UNIX_EPOCH};
use spark_wallet::{SparkWallet, TransferId};
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

use crate::{
    AmountAdjustmentReason, EventEmitter, Network, Payment, PaymentDetails, PaymentMetadata,
    RefundPendingConversionsResponse, Storage, SwapDegradation,
    persist::{StorageListPaymentsRequest, StoragePaymentDetailsFilter},
    token_conversion::{ConversionAmount, DEFAULT_CONVERSION_MAX_SLIPPAGE_BPS},
    utils::{
        payments::{
            fetch_and_process_payment, insert_payment_metadata_with_cache_fallback,
            insert_payment_with_metadata, resolve_and_insert_payment_metadata, resolve_payment_id,
        },
        polling::{PollSchedule, poll_until},
    },
};

use super::{
    ConversionError, ConversionEstimate, ConversionInfo, ConversionOptions, ConversionPurpose,
    ConversionStatus, ConversionType, FeeSplit, FetchConversionLimitsRequest,
    FetchConversionLimitsResponse, ResolvedConversion, TokenConversionPool,
    TokenConversionResponse, TokenConverter,
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
/// Headroom above slippage and integrator fee before a simulated output counts
/// as evidence the input was overstated. Measured overshoot peaks at ~14 bps
/// across a 1000x range of sizes, far below a mispriced quote.
const OUTPUT_CEILING_MARGIN_BPS: u128 = 100;
/// The share of the proportional output an input raised above the one the
/// request derived must still buy.
///
/// Loose because a larger input does not buy a proportionally larger output:
/// price impact grows with the trade. It never binds below a doubling, which is
/// as far as the dust adjustment on its own reaches.
const INPUT_SCALED_FLOOR_BPS: u128 = 5_000;
/// Rows read back from a listing when reconciling. Neither is paginated: a
/// clawback missed here is picked up by the next init, and a swap missed here
/// falls through to the refund path, which is the behaviour before reconciling
/// existed.
const RECONCILE_LISTING_LIMIT: u32 = 100;

/// Returns true when the transfer is older than `cutoff_secs`, i.e. eligible
/// for clawback. Backend emits RFC 3339; a missing or unparseable timestamp
/// falls back to eligible, trusting Flashnet's clawback-eligibility verdict
/// over silently skipping on an unexpected format.
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

/// The conversion as it applies to each of its legs, as `(sent, received)`.
///
/// The fee follows the same split as the success path: it is denominated in the
/// token, so it belongs to the leg holding it, which is the received one when
/// the input was BTC. The adjustment and any degradation describe the input
/// that was sent. The pool, the conversion id and the purpose describe the
/// conversion itself and stay on both.
fn split_legs(info: &ConversionInfo, input_is_btc: bool) -> (ConversionInfo, ConversionInfo) {
    let token_side_fee = match info {
        ConversionInfo::Amm { fee, .. } => *fee,
        _ => None,
    };

    let mut sent = info.clone();
    if let ConversionInfo::Amm { fee, .. } = &mut sent
        && input_is_btc
    {
        *fee = None;
    }

    let mut received = info.clone();
    if let ConversionInfo::Amm {
        fee,
        amount_adjustment,
        degradation,
        ..
    } = &mut received
    {
        *fee = if input_is_btc { token_side_fee } else { None };
        *amount_adjustment = None;
        *degradation = None;
    }

    (sent, received)
}

/// Builds the resolved metadata for a conversion. Carries the prior AMM fields
/// (`conversion_id`, `fee`, `purpose`, `amount_adjustment`) forward when
/// present; otherwise emits a placeholder keyed on `fallback_pool_id` with a
/// fresh `conversion_id` and unset fields.
fn resolved_info_from_prior(
    prev: Option<ConversionInfo>,
    fallback_pool_id: &PublicKey,
    status: ConversionStatus,
) -> ConversionInfo {
    match prev {
        Some(ConversionInfo::Amm {
            pool_id,
            conversion_id,
            fee,
            purpose,
            amount_adjustment,
            degradation,
            ..
        }) => ConversionInfo::Amm {
            pool_id,
            conversion_id,
            status,
            fee,
            purpose,
            amount_adjustment,
            degradation,
        },
        _ => ConversionInfo::Amm {
            pool_id: fallback_pool_id.to_string(),
            conversion_id: uuid::Uuid::now_v7().to_string(),
            status,
            fee: None,
            purpose: None,
            amount_adjustment: None,
            degradation: None,
        },
    }
}

/// Whether the swap names a delivery that can address a payment.
///
/// The shape follows the asset delivered: a Spark transfer id for BTC, a token
/// transaction hash otherwise. Neither can be looked up if it is malformed, so
/// the conversion would complete against nothing.
fn delivery_is_addressable(swap: &Swap) -> bool {
    if swap.asset_out_address == BTC_ASSET_ADDRESS {
        TransferId::from_str(&swap.outbound_transfer_id).is_ok()
    } else {
        hex::decode(&swap.outbound_transfer_id).is_ok_and(|hash| hash.len() == 32)
    }
}

/// Finds the swap the pool ran against the transfer we sent it.
///
/// Compared whole: a partial match would attribute another swap's delivery to
/// this conversion and skip a refund that is still owed.
fn swap_for_transfer<'a>(
    swaps: &'a [Swap],
    inbound_transfer_id: &str,
    expected_pool: Option<PublicKey>,
) -> Option<&'a Swap> {
    let swap = swaps
        .iter()
        .find(|swap| swap.inbound_transfer_id == inbound_transfer_id)?;

    // Adopting a swap is terminal, so anything that does not line up is left
    // to the refund path, which is retried. The amount is not compared: the
    // listing's `amount_in` may or may not be net of `fee_paid`, and a check
    // that never matches disables the reconcile silently.
    let mismatch = if swap.amount_out == 0 {
        Some("delivered nothing".to_string())
    } else if !delivery_is_addressable(swap) {
        Some(format!(
            "names delivery {:?}, which cannot address a payment",
            swap.outbound_transfer_id
        ))
    } else if expected_pool.is_some_and(|p| swap.pool_lp_public_key != p) {
        Some(format!("names pool {}", swap.pool_lp_public_key))
    } else {
        None
    };
    if let Some(reason) = mismatch {
        warn!("Swap for {inbound_transfer_id} {reason}");
        return None;
    }
    Some(swap)
}

/// What a pass over one `RefundNeeded` row concluded.
enum RefundOutcome {
    /// The pool returned the transfer that was sent to it.
    Refunded,
    /// The clawback was refused or errored, so the transfer is still with the
    /// pool. Retried next pass.
    NotRefunded,
    /// The swap had in fact run, so the transfer was spent and there is nothing
    /// to return. Terminal.
    AlreadyExecuted,
}

/// The status a conversion carries, from whether the pool took the input and
/// which transfers it named. A pool that refused still populates identifiers.
fn conversion_status(
    executed: bool,
    inbound_identifier: Option<&str>,
    refund_identifier: Option<&str>,
) -> ConversionStatus {
    // An empty identifier names no transfer.
    let inbound_identifier = inbound_identifier.filter(|id| !id.is_empty());
    let refund_identifier = refund_identifier.filter(|id| !id.is_empty());
    match (executed, inbound_identifier, refund_identifier) {
        // Took the input and named what paid us.
        (true, Some(_), _) => ConversionStatus::Completed,
        // The input came back, whether or not the swap ran.
        (_, _, Some(_)) => ConversionStatus::Refunded,
        // Nothing came back. The input may still be at the pool, and the
        // clawback targets the transfer we sent, so it is attempted even when
        // the pool claims to have executed without naming a delivery.
        (_, _, None) => ConversionStatus::RefundNeeded,
    }
}

/// The caller's slippage tolerance, refused above 100%.
///
/// A larger value would sign away the floor derived from it.
fn resolve_max_slippage_bps(options: &ConversionOptions) -> Result<u32, ConversionError> {
    let max_slippage_bps = options
        .max_slippage_bps
        .unwrap_or(DEFAULT_CONVERSION_MAX_SLIPPAGE_BPS);
    if max_slippage_bps > 10_000 {
        return Err(ConversionError::ValidationFailed(format!(
            "max slippage {max_slippage_bps} bps is above 10000"
        )));
    }
    Ok(max_slippage_bps)
}

/// The input to convert, given the protocol minimum and the balance held.
///
/// Raises the input to the minimum, and then to the whole balance where what
/// would be left over is itself below the minimum and so could never be
/// converted. The second raise is bounded at twice the first: it needs a
/// remainder below a minimum the raised input already clears.
fn adjust_to_min_limit(
    amount_in: u128,
    min_from_amount: u128,
    token_balance: u128,
) -> (u128, Option<AmountAdjustmentReason>) {
    let adjusted = amount_in.max(min_from_amount);
    let remaining = token_balance.saturating_sub(adjusted);
    if remaining > 0 && remaining < min_from_amount {
        return (
            token_balance,
            Some(AmountAdjustmentReason::IncreasedToAvoidDust),
        );
    }
    if adjusted == amount_in {
        return (amount_in, None);
    }
    (adjusted, Some(AmountAdjustmentReason::FlooredToMinLimit))
}

/// The least output a conversion must return, given how far its input was
/// raised above the one the requested output derived.
///
/// A protocol minimum or the dust adjustment can raise the input with no effect
/// on the requested output, which a raised input then clears at any rate.
/// Scaling with the input requires a swap that spends more to deliver more.
/// Returns the request unchanged when the input was not raised.
fn scaled_min_amount_out(requested_out: u128, amount_in: u128, calculated_amount_in: u128) -> u128 {
    if calculated_amount_in == 0 || amount_in <= calculated_amount_in {
        return requested_out;
    }
    let scale_bps = amount_in
        .checked_mul(INPUT_SCALED_FLOOR_BPS)
        .and_then(|scaled| scaled.checked_div(calculated_amount_in))
        .unwrap_or(u128::MAX);
    requested_out
        .checked_mul(scale_bps)
        .map_or(u128::MAX, |scaled| scaled / 10_000)
        .max(requested_out)
}

/// Bounds a simulated output against the input it was derived from.
///
/// `target_out` is the output the input was derived for: the request, rounded up
/// for a BTC output. A small request rounds up by more than the tolerance.
fn check_simulated_output(
    simulated: u128,
    min_amount_out: u128,
    target_out: u128,
    amount_in: u128,
    calculated_amount_in: u128,
    ceiling_bps: u128,
) -> Result<(), String> {
    if simulated < min_amount_out {
        return Err(format!(
            "returned {simulated} but expected at least {min_amount_out}"
        ));
    }

    // The ceiling bounds an input the pool's price derived. A raised input did
    // not come from that price, and extrapolating the bound to it understates
    // what the pool returns, since the rate improves with size. The scaled
    // floor bounds that case instead.
    if calculated_amount_in > 0 && amount_in > calculated_amount_in {
        return Ok(());
    }
    // Rounded up: below a target of 91 the margin is under one unit and
    // truncates away, leaving no tolerance at all.
    let ceiling = target_out
        .checked_mul(ceiling_bps)
        .map_or(target_out, |scaled| scaled.div_ceil(10_000));
    if simulated > ceiling {
        return Err(format!("returned {simulated}, above the {ceiling} ceiling"));
    }

    Ok(())
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
                Ok(RefundOutcome::Refunded) => report.refunded = report.refunded.saturating_add(1),
                Ok(RefundOutcome::AlreadyExecuted) => {
                    report.skipped = report.skipped.saturating_add(1);
                }
                Ok(RefundOutcome::NotRefunded) => report.failed = report.failed.saturating_add(1),
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
    async fn refund_payment(&self, payment: &Payment) -> Result<RefundOutcome, ConversionError> {
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
        let Some(
            info @ ConversionInfo::Amm {
                pool_id,
                status: ConversionStatus::RefundNeeded,
                ..
            },
        ) = conversion_info
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

        // The swap may have run since this row was marked, in which case the
        // input is spent and the clawback would be refused on every pass.
        if let Some(swap) = self.find_executed_swap(Some(pool_id), &clawback_id).await {
            info!(
                "Conversion {} was executed after all, delivering {} via {}",
                payment.id, swap.amount_out, swap.outbound_transfer_id
            );
            self.record_executed_swap(
                &payment.id,
                &clawback_id,
                pool_id,
                Some(info.clone()),
                &swap,
            )
            .await?;
            return Ok(RefundOutcome::AlreadyExecuted);
        }

        // payment.id is the storage row id. Pass the already-loaded conversion
        // info so the write preserves its fields without re-reading the row.
        let returned = self
            .clawback_and_record_refunded(
                &clawback_id,
                pool_id,
                Some(payment.id.clone()),
                Some(info.clone()),
            )
            .await?;
        Ok(if returned {
            RefundOutcome::Refunded
        } else {
            RefundOutcome::NotRefunded
        })
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
                .clawback_and_record_refunded(
                    &transfer.id,
                    transfer.lp_identity_public_key,
                    None,
                    None,
                )
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

    /// Claws back one transfer, then records `Refunded` locally while
    /// preserving the prior AMM fields. `Ok(true)` = Flashnet accepted (a
    /// failed metadata write is cached for the next sync); `Ok(false)` =
    /// rejected, local state untouched for a retry; `Err` = the clawback call
    /// failed (funds not returned). `payment_id` skips row-id resolution;
    /// `prior_info` skips the metadata re-read.
    async fn clawback_and_record_refunded(
        &self,
        clawback_id: &str,
        pool_id: PublicKey,
        payment_id: Option<String>,
        prior_info: Option<ConversionInfo>,
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
                warn!(
                    "Clawback for {clawback_id} not accepted (request_id={}): {:?}",
                    r.request_id, r.error
                );
                return Ok(false);
            }
            Err(e) => {
                error!("Clawback for {clawback_id} failed: {e}");
                return Err(e.into());
            }
        }

        // Resolve the storage row id once. The local path passes it directly;
        // reconcile resolves it from clawback_id, which for tokens is a bare
        // tx hash rather than the {hash}:{vout} row id.
        let resolved_id = match payment_id {
            Some(id) => Some(id),
            None => resolve_payment_id(clawback_id, &self.spark_wallet, &self.storage, true)
                .await
                .ok(),
        };

        // Preserve the prior ConversionInfo::Amm fields. Prefer the
        // caller-supplied info (local path, definitely present); otherwise read
        // the resolved row. Falls back to a placeholder when neither is present
        // (reconcile recovering a row whose original metadata write never landed).
        let prev = match prior_info {
            Some(info) => Some(info),
            None => match &resolved_id {
                Some(id) => self
                    .storage
                    .get_payment_by_id(id.clone())
                    .await
                    .ok()
                    .and_then(|p| crate::utils::conversions::extract_conversion_info(p.details)),
                None => None,
            },
        };
        let metadata = PaymentMetadata {
            conversion_info: Some(resolved_info_from_prior(
                prev,
                &pool_id,
                ConversionStatus::Refunded,
            )),
            ..Default::default()
        };

        match resolved_id {
            Some(id) => {
                // Cache the mark under clawback_id (the sync-time identifier) if
                // the write fails: the clawback already succeeded, so surface
                // Ok rather than counting a returned refund as failed.
                insert_payment_metadata_with_cache_fallback(
                    &self.storage,
                    id,
                    clawback_id,
                    metadata,
                )
                .await
                .map_err(ConversionError::Sdk)?;
            }
            None => {
                // Row id didn't resolve (row not present locally): cache under
                // the identifier so the next sync reapplies the mark.
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
        // Ahead of the listings: the caller's own parameter needs no network.
        let max_slippage_bps = resolve_max_slippage_bps(conversion_options)?;

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

        // Select the best pool using multi-factor scoring
        let pool = flashnet::select_best_pool(
            &pools,
            &asset_in_address,
            &asset_out_address,
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
    ) -> Result<(ConversionEstimate, u128), ConversionError> {
        let TokenConversionPool {
            asset_in_address,
            asset_out_address,
            pool,
        } = conversion_pool;
        let max_slippage_bps = resolve_max_slippage_bps(conversion_options)?;

        // Calculate the required amount in for the desired amount out
        let calculated_amount_in = pool.calculate_amount_in(
            asset_in_address,
            amount_out,
            max_slippage_bps,
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

        // The signed floor bounds what is received, not what is sent. An
        // output far above the request means the derived input was overstated.
        let target_out =
            pool.effective_amount_out(asset_in_address, amount_out, self.network.into())?;
        let min_amount_out = scaled_min_amount_out(amount_out, amount_in, calculated_amount_in);
        let ceiling_bps = 10_000u128
            .saturating_add(u128::from(max_slippage_bps))
            .saturating_add(u128::from(self.integrator_fee_bps))
            .saturating_add(OUTPUT_CEILING_MARGIN_BPS);
        debug!(
            "Conversion estimate: requested {amount_out} (target {target_out}), \
             floor {min_amount_out}, input {amount_in} (derived {calculated_amount_in}), \
             simulated {}",
            response.amount_out
        );
        check_simulated_output(
            response.amount_out,
            min_amount_out,
            target_out,
            amount_in,
            calculated_amount_in,
            ceiling_bps,
        )
        .map_err(|reason| ConversionError::ValidationFailed(format!("Validation {reason}")))?;

        Ok((
            ConversionEstimate {
                options: conversion_options.clone(),
                amount_in,
                amount_out: response.amount_out,
                fee: response.fee_paid_asset_in.unwrap_or(0),
                amount_adjustment,
            },
            min_amount_out,
        ))
    }

    /// Looks for a swap the pool ran against `inbound_transfer_id`, the input
    /// transfer we sent.
    ///
    /// The listing has no transfer-id filter, so this scans the newest page.
    /// `pool_id` narrows the request and is held against the result; pass `None`
    /// where the pool is not known to be the one that ran the swap, since
    /// filtering on a guess hides the very entry being looked for.
    async fn find_executed_swap(
        &self,
        pool_id: Option<PublicKey>,
        inbound_transfer_id: &str,
    ) -> Option<Swap> {
        let request = ListUserSwapsRequest {
            pool_lp_pubkey: pool_id,
            sort: Some(SwapSortOrder::TimestampDesc),
            limit: Some(RECONCILE_LISTING_LIMIT),
            ..Default::default()
        };
        let swaps = match self.flashnet_client.list_user_swaps(request).await {
            Ok(response) => response.swaps,
            Err(e) => {
                // A listing failure means no swap: fall through to the refund
                // path.
                warn!("Could not list swaps to reconcile {inbound_transfer_id}: {e}");
                return None;
            }
        };
        let found = swap_for_transfer(&swaps, inbound_transfer_id, pool_id).cloned();
        // The matched entry only, not the page it came from: enough to see
        // what a reconcile decided on, without a page per receive.
        debug!(
            "Reconciled {inbound_transfer_id} against {} swaps: {found:?}",
            swaps.len()
        );
        found
    }

    /// Records a conversion whose swap turns out to have run, and links the
    /// delivery it names so the incoming funds are not left unattributed.
    ///
    /// Moving the row off `RefundNeeded` is what ends the clawback attempts: a
    /// clawback against a consumed transfer is refused, and the row would
    /// otherwise return on every pass.
    async fn record_executed_swap(
        &self,
        payment_id: &str,
        cache_key: &str,
        pool_id: PublicKey,
        prior: Option<ConversionInfo>,
        swap: &Swap,
    ) -> Result<(), ConversionError> {
        let mut info = resolved_info_from_prior(prior, &pool_id, ConversionStatus::Completed);
        // The swap response is the better source and is carried forward when a
        // prior attempt recorded one. Otherwise the listing's fee stands in.
        if let ConversionInfo::Amm { fee, .. } = &mut info
            && fee.is_none()
        {
            *fee = Some(swap.fee_paid);
        }
        let (sent, received) = split_legs(&info, swap.asset_in_address == BTC_ASSET_ADDRESS);
        // The delivery is linked first. Writing the status is what ends the
        // clawback attempts, so a failure between the two has to leave the row
        // refundable: it is retried, and this runs again. The reverse order
        // would stop the retries with the delivery still unattributed.
        resolve_and_insert_payment_metadata(
            &swap.outbound_transfer_id,
            PaymentMetadata {
                conversion_info: Some(received),
                ..Default::default()
            },
            &self.spark_wallet,
            &self.storage,
            false,
        )
        .await
        .map_err(ConversionError::Sdk)?;

        insert_payment_metadata_with_cache_fallback(
            &self.storage,
            payment_id.to_string(),
            // The sync-time identifier, which for a token is the bare hash
            // rather than the row's {hash}:{vout}.
            cache_key,
            PaymentMetadata {
                conversion_info: Some(sent),
                ..Default::default()
            },
        )
        .await
        .map_err(ConversionError::Sdk)?;
        Ok(())
    }

    /// Records both legs of a conversion whose swap already ran, keyed on the
    /// transfers rather than on storage rows.
    ///
    /// Writes the metadata only. The success path also inserts local `Payment`
    /// rows and claims the received leg, which needs the `AssetTransfer` this
    /// path does not hold: the transfer belongs to an earlier attempt. Both
    /// legs therefore surface on the next sync rather than immediately.
    async fn record_completed_legs(
        &self,
        sent_identifier: &str,
        swap: &Swap,
        purpose: &ConversionPurpose,
    ) -> Result<TokenConversionResponse, ConversionError> {
        let info = ConversionInfo::Amm {
            // The pool that ran it, which need not be the one just selected.
            pool_id: swap.pool_lp_public_key.to_string(),
            conversion_id: uuid::Uuid::now_v7().to_string(),
            status: ConversionStatus::Completed,
            fee: Some(swap.fee_paid),
            purpose: Some(purpose.clone()),
            // The swap being adopted was sized by the attempt that ran it, not
            // by this one, so this attempt's adjustment does not describe it.
            amount_adjustment: None,
            // The listing states that a swap ran, not how it went.
            degradation: None,
        };
        let (sent, received) = split_legs(&info, swap.asset_in_address == BTC_ASSET_ADDRESS);
        let sent_payment_id = resolve_and_insert_payment_metadata(
            sent_identifier,
            PaymentMetadata {
                conversion_info: Some(sent),
                ..Default::default()
            },
            &self.spark_wallet,
            &self.storage,
            true,
        )
        .await
        .map_err(ConversionError::Sdk)?;
        let received_payment_id = resolve_and_insert_payment_metadata(
            &swap.outbound_transfer_id,
            PaymentMetadata {
                conversion_info: Some(received),
                ..Default::default()
            },
            &self.spark_wallet,
            &self.storage,
            false,
        )
        .await
        .map_err(ConversionError::Sdk)?;
        Ok(TokenConversionResponse {
            sent_payment_id,
            received_payment_id,
        })
    }

    /// Updates the payment with the conversion info.
    ///
    /// Arguments:
    /// * `pool_id` - The pool id used for the conversion.
    /// * `outbound_asset_transfer` - The outbound `AssetTransfer` for the sent leg.
    /// * `inbound_identifier` - The inbound spark transfer id or token transaction hash if the conversion was successful.
    /// * `refund_identifier` - The inbound refund spark transfer id or token transaction hash if the conversion was refunded.
    /// * `executed` - Whether the pool took the input. Distinct from the
    ///   identifiers, which the pool populates whatever it decided.
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
        executed: bool,
        fee_split: Option<FeeSplit>,
        purpose: &ConversionPurpose,
        amount_adjustment: Option<AmountAdjustmentReason>,
        degradation: Option<SwapDegradation>,
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

        let status = conversion_status(
            executed,
            inbound_identifier.as_deref(),
            refund_identifier.as_deref(),
        );
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
                        // On the sent leg, which exists even when the response
                        // named no delivery.
                        degradation,
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

        // Only a completed conversion has a received leg. `RefundNeeded` on an
        // inbound row would have the refunder claw back the inbound identifier.
        let received_fut = async {
            if let (Some(identifier), ConversionStatus::Completed) = (&inbound_identifier, &status)
            {
                let payment_id = crate::utils::payments::resolve_and_insert_payment_metadata(
                    identifier,
                    PaymentMetadata {
                        conversion_info: Some(ConversionInfo::Amm {
                            pool_id: pool_id_str.clone(),
                            conversion_id: conversion_id.clone(),
                            status: ConversionStatus::Completed,
                            fee: received_fee,
                            purpose: Some(purpose.clone()),
                            amount_adjustment: None,
                            degradation: None,
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
                        degradation: None,
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

        let token_balances = self.spark_wallet.get_token_balances().await?;
        let token_balance = token_balances
            .get(from_token_identifier)
            .map_or(0, |b| b.balance);

        let adjustment = adjust_to_min_limit(amount_in, min_from_amount, token_balance);
        match adjustment {
            (adjusted, Some(AmountAdjustmentReason::IncreasedToAvoidDust)) => info!(
                "Adjusting ToBitcoin conversion to avoid token dust: \
                 converting all {adjusted} tokens (min {min_from_amount})"
            ),
            (adjusted, Some(AmountAdjustmentReason::FlooredToMinLimit)) => info!(
                "Floored ToBitcoin conversion amount_in from {amount_in} to {adjusted} (min {min_from_amount})"
            ),
            _ => {}
        }
        Ok(adjustment)
    }

    /// Prices a conversion and keeps hold of the pool it was priced against.
    ///
    /// Selecting again before executing can pick a different pool, because
    /// selection scores on the amount it is given and the two calls are given
    /// different ones.
    async fn resolve_conversion(
        &self,
        options: &ConversionOptions,
        token_identifier: Option<&String>,
        amount: &ConversionAmount,
    ) -> Result<ResolvedConversion, ConversionError> {
        match amount {
            ConversionAmount::MinAmountOut(amount_out) => {
                let conversion_pool = self
                    .get_conversion_pool(options, token_identifier, *amount_out)
                    .await?;
                let (estimate, min_amount_out) = self
                    .estimate_internal(&conversion_pool, options, token_identifier, *amount_out)
                    .await?;
                Ok(ResolvedConversion {
                    conversion_pool,
                    min_amount_out,
                    estimate,
                })
            }
            ConversionAmount::AmountIn(amount_in) => {
                // Quote first, then select on the resulting output. Selecting on
                // zero derives an input of zero for every pool, tying them on
                // the term that carries half the score.
                let probe = self
                    .get_conversion_pool(options, token_identifier, 0)
                    .await?;
                let response = self
                    .flashnet_client
                    .simulate_swap(SimulateSwapRequest {
                        asset_in_address: probe.asset_in_address.clone(),
                        asset_out_address: probe.asset_out_address.clone(),
                        pool_id: probe.pool.lp_public_key,
                        amount_in: *amount_in,
                        integrator_bps: if self.integrator_fee_bps > 0 {
                            Some(self.integrator_fee_bps)
                        } else {
                            None
                        },
                    })
                    .await?;

                let max_slippage = resolve_max_slippage_bps(options)?;
                let estimated_out = response
                    .amount_out
                    .saturating_mul(10_000u128.saturating_sub(u128::from(max_slippage)))
                    .saturating_div(10_000);
                let conversion_pool = self
                    .get_conversion_pool(options, token_identifier, estimated_out)
                    .await?;

                // The probe is selected on an output of zero, where every pool
                // derives an input of zero and ties on the term carrying half
                // the score, so its quote must not set the floor signed here.
                // Re-quote only when the second selection moved: on a pair with
                // a single viable pool the two land together and the first
                // quote already came from the pool that will execute.
                let (response, estimated_out) =
                    if conversion_pool.pool.lp_public_key == probe.pool.lp_public_key {
                        (response, estimated_out)
                    } else {
                        let response = self
                            .flashnet_client
                            .simulate_swap(SimulateSwapRequest {
                                asset_in_address: conversion_pool.asset_in_address.clone(),
                                asset_out_address: conversion_pool.asset_out_address.clone(),
                                pool_id: conversion_pool.pool.lp_public_key,
                                amount_in: *amount_in,
                                integrator_bps: if self.integrator_fee_bps > 0 {
                                    Some(self.integrator_fee_bps)
                                } else {
                                    None
                                },
                            })
                            .await?;
                        let estimated_out = response
                            .amount_out
                            .saturating_mul(10_000u128.saturating_sub(u128::from(max_slippage)))
                            .saturating_div(10_000);
                        (response, estimated_out)
                    };

                Ok(ResolvedConversion {
                    conversion_pool,
                    min_amount_out: estimated_out,
                    estimate: ConversionEstimate {
                        options: options.clone(),
                        amount_in: *amount_in,
                        amount_out: estimated_out,
                        fee: response.fee_paid_asset_in.unwrap_or(0),
                        amount_adjustment: None,
                    },
                })
            }
        }
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
        // A caller-supplied transfer id is known before the transfer, so a swap
        // already carrying it means an earlier attempt got through. Only the
        // BTC leg has one: a token transaction hash does not exist until the
        // transaction is built, so a token retry cannot be recognised here.
        if let Some(transfer_id) = &transfer_id
            && options.conversion_type == ConversionType::FromBitcoin
        {
            let sent_identifier = transfer_id.to_string();
            // Unfiltered by pool: selection re-runs on each attempt and scores
            // on live reserves, so the pool chosen now may not be the one that
            // ran the earlier swap.
            if let Some(swap) = self.find_executed_swap(None, &sent_identifier).await {
                info!(
                    "Conversion for transfer {sent_identifier} already ran on pool {}, delivering {} via {}",
                    swap.pool_lp_public_key, swap.amount_out, swap.outbound_transfer_id
                );
                return self
                    .record_completed_legs(&sent_identifier, &swap, purpose)
                    .await;
            }
        }

        // Price the conversion, keeping the pool it was priced against so the
        // amounts below cannot be handed to a different one.
        let ResolvedConversion {
            conversion_pool,
            min_amount_out,
            estimate,
        } = self
            .resolve_conversion(options, token_identifier, &amount)
            .await?;
        let amount_in = estimate.amount_in;
        let amount_adjustment = estimate.amount_adjustment;
        let pool_id = conversion_pool.pool.lp_public_key;

        // Execute the conversion
        let response_res = self
            .flashnet_client
            .execute_swap(
                &conversion_pool.pool,
                ExecuteSwapRequest {
                    asset_in_address: conversion_pool.asset_in_address.clone(),
                    asset_out_address: conversion_pool.asset_out_address.clone(),
                    amount_in,
                    max_slippage_bps: resolve_max_slippage_bps(options)?,
                    min_amount_out,
                    integrator_override: None,
                    transfer_id,
                },
            )
            .await;

        match response_res {
            Ok(ExecuteSwapResponse {
                flashnet_response,
                outcome,
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
                        outcome.is_executed(),
                        fee_split,
                        purpose,
                        amount_adjustment.clone(),
                        match &outcome {
                            SwapOutcome::AcceptedDegraded { degradation } => {
                                Some((*degradation).into())
                            }
                            _ => None,
                        },
                    ))
                    .await?;

                if let Some(received_payment_id) = received_payment_id
                    && outcome.is_executed()
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
                // The failure may have followed a swap the pool ran, in which
                // case the input is spent and a clawback would be refused.
                let executed = self.find_executed_swap(Some(pool_id), &transfer.id()).await;

                let update_res = Box::pin(
                    self.update_payment_conversion_info(
                        &pool_id,
                        transfer,
                        executed
                            .as_ref()
                            .map(|swap| swap.outbound_transfer_id.clone()),
                        // A refund identifier arrives only in a swap response, which
                        // is what failed. The listing records swaps, not refunds.
                        None,
                        executed.is_some(),
                        None,
                        purpose,
                        amount_adjustment.clone(),
                        // The listing states that a swap ran, not how it went.
                        None,
                    ),
                )
                .await;
                if let Err(err) = update_res {
                    warn!("Could not record the outcome for {}: {err}", transfer.id());
                }

                if let Some(swap) = executed {
                    return Err(ConversionError::ConversionFailed(format!(
                        "Convert token failed after the swap ran, delivering {} via {}: {}",
                        swap.amount_out,
                        swap.outbound_transfer_id,
                        *source.clone()
                    )));
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
        let resolved = self
            .resolve_conversion(options, token_identifier, &amount)
            .await?;
        Ok(Some(resolved.estimate))
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
                RefundPendingConversionsResponse::default()
            }
        };
        let remote = match self.reconcile_with_flashnet().await {
            Ok(r) => r,
            Err(e) => {
                warn!("Reconcile with Flashnet failed: {e}");
                // Both passes errored: surface via Err. A single-pass error
                // stays Ok with the other pass's counts; the errored pass
                // contributes none rather than a phantom per-conversion `failed`.
                if local_failed {
                    return Err(e);
                }
                RefundPendingConversionsResponse::default()
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

    fn options(max_slippage_bps: Option<u32>) -> ConversionOptions {
        ConversionOptions {
            conversion_type: ConversionType::ToBitcoin {
                from_token_identifier: "token".to_string(),
            },
            max_slippage_bps,
            completion_timeout_secs: None,
        }
    }

    #[test]
    fn an_input_under_the_minimum_is_floored_to_it() {
        assert_eq!(adjust_to_min_limit(700, 640, 10_000), (700, None));
        assert_eq!(adjust_to_min_limit(640, 640, 10_000), (640, None));
        assert_eq!(
            adjust_to_min_limit(500, 640, 10_000),
            (640, Some(AmountAdjustmentReason::FlooredToMinLimit))
        );
    }

    #[test]
    fn a_remainder_that_could_never_convert_is_swept_in() {
        // Below the minimum, so it could not be converted on its own.
        assert_eq!(
            adjust_to_min_limit(700, 640, 1_000),
            (1_000, Some(AmountAdjustmentReason::IncreasedToAvoidDust))
        );
        // Exactly the minimum, so it can be, and is left.
        assert_eq!(adjust_to_min_limit(700, 640, 1_340), (700, None));
        // Nothing left over at all.
        assert_eq!(adjust_to_min_limit(700, 640, 700), (700, None));
        // Balance below the input: the conversion is not resized to it.
        assert_eq!(adjust_to_min_limit(700, 640, 100), (700, None));
    }

    #[test]
    fn the_dust_sweep_never_more_than_doubles_the_floored_input() {
        // The bound the scaled floor's haircut is set against. Anything past a
        // doubling came from the minimum, not from the sweep.
        for amount_in in [1u128, 500, 640, 700, 10_000, u128::MAX / 4] {
            for min_from_amount in [1u128, 640, 10_000, u128::MAX / 4] {
                for balance in [0u128, 1, 700, 10_000, u128::MAX / 2, u128::MAX] {
                    let (adjusted, _) = adjust_to_min_limit(amount_in, min_from_amount, balance);
                    let floored = amount_in.max(min_from_amount);
                    assert!(
                        adjusted <= floored.saturating_mul(2),
                        "{amount_in}/{min_from_amount}/{balance} gave {adjusted}"
                    );
                }
            }
        }
    }

    #[test]
    fn an_unraised_input_keeps_the_requested_floor() {
        assert_eq!(scaled_min_amount_out(1_000, 640, 640), 1_000);
        assert_eq!(scaled_min_amount_out(1_000, 500, 640), 1_000);
        assert_eq!(scaled_min_amount_out(1_000, 640, 0), 1_000);
    }

    #[test]
    fn the_dust_adjustment_alone_never_binds() {
        // It converts the whole balance only when the remainder would fall
        // below the minimum the input was already floored to, which bounds it
        // at twice that input. The floor is still the request there.
        assert_eq!(scaled_min_amount_out(1_000, 1_279, 640), 1_000);
        assert_eq!(scaled_min_amount_out(1_000, 1_280, 640), 1_000);
        // Below the doubling the scaled value falls under the request, which
        // must not weaken the floor that already applied.
        assert_eq!(scaled_min_amount_out(1_000, 700, 640), 1_000);
    }

    #[test]
    fn a_raised_input_raises_the_floor() {
        // The first ratio that binds, just past the doubling.
        assert_eq!(scaled_min_amount_out(1_000, 1_400, 640), 1_093);
        assert_eq!(scaled_min_amount_out(1_000, 6_400, 640), 5_000);
        assert_eq!(scaled_min_amount_out(1_000, 640_000, 640), 500_000);
        // A request of zero asks for nothing, at any input.
        assert_eq!(scaled_min_amount_out(0, 640_000, 640), 0);
    }

    #[test]
    fn a_floor_too_large_to_represent_refuses_rather_than_drops() {
        // No fill can clear it, so the conversion fails on the floor instead of
        // running with the raise unbounded.
        assert_eq!(scaled_min_amount_out(u128::MAX, u128::MAX, 1), u128::MAX);
        assert_eq!(scaled_min_amount_out(u128::MAX / 2, 2, 1), u128::MAX);
    }

    #[test]
    fn no_raise_escapes_the_floor() {
        // Whatever the raise and whatever the token's scale, the floor tracks
        // it. Collapsing back to the request is what would leave a raised input
        // free to be filled at any rate.
        let calculated = 10u128.pow(10);
        for requested_out in [1u128, 1_000, 10u128.pow(18), 10u128.pow(30)] {
            for raise in [2u128, 39, 1_000, 10u128.pow(12)] {
                let floor = scaled_min_amount_out(requested_out, calculated * raise, calculated);
                match requested_out.checked_mul(raise) {
                    // Half the proportional output, as the haircut allows.
                    Some(proportional) => assert_eq!(
                        floor,
                        (proportional / 2).max(requested_out),
                        "requested {requested_out} raised {raise}x"
                    ),
                    // Beyond representing, so no fill clears it.
                    None => assert_eq!(floor, u128::MAX),
                }
            }
        }
    }

    #[test]
    fn a_high_decimal_token_still_scales_its_floor() {
        // Scaling the request by the raised input first overflows here, and the
        // request alone bounds nothing: the input is 100x what the output
        // derived, so the floor has to rise with it.
        let requested_out = 10u128.pow(30);
        let floor = scaled_min_amount_out(requested_out, 10u128.pow(12), 10u128.pow(10));
        assert_eq!(floor, requested_out * 50);
        assert!(requested_out.checked_mul(10u128.pow(12)).is_none());
    }

    #[test]
    fn a_raised_input_must_buy_a_raised_output() {
        // A whole token balance spent for the originally requested output. The
        // ceiling scales with the input and does not see it.
        let floor = scaled_min_amount_out(1_000, 640_000, 640);
        assert!(check_simulated_output(1_000, floor, 1_000, 640_000, 640, 10_115).is_err());
        assert!(check_simulated_output(floor - 1, floor, 1_000, 640_000, 640, 10_115).is_err());
        assert!(check_simulated_output(floor, floor, 1_000, 640_000, 640, 10_115).is_ok());
    }

    #[test]
    fn slippage_above_a_hundred_percent_is_refused() {
        assert_eq!(
            resolve_max_slippage_bps(&options(None)).unwrap(),
            DEFAULT_CONVERSION_MAX_SLIPPAGE_BPS
        );
        assert_eq!(resolve_max_slippage_bps(&options(Some(0))).unwrap(), 0);
        assert_eq!(
            resolve_max_slippage_bps(&options(Some(10_000))).unwrap(),
            10_000
        );
        assert!(resolve_max_slippage_bps(&options(Some(10_001))).is_err());
        assert!(resolve_max_slippage_bps(&options(Some(u32::MAX))).is_err());
    }

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

    /// An unrecognised timestamp must still let the clawback proceed instead
    /// of being silently bucketed as `skipped`.
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

    fn sample_pool_key() -> PublicKey {
        PublicKey::from_str("02894808873b896e21d29856a6d7bb346fb13c019739adb9bf0b6a8b7e28da53da")
            .unwrap()
    }

    /// The clawed-back mark must carry the prior conversion's fields forward,
    /// flipping only the status. This is the reconcile/local path preserving
    /// `fee`/`purpose`/`amount_adjustment` instead of overwriting with a placeholder.
    #[test]
    fn refunded_info_preserves_prior_amm_fields() {
        let prior = ConversionInfo::Amm {
            pool_id: "prior-pool".to_string(),
            conversion_id: "conv-abc".to_string(),
            status: ConversionStatus::RefundNeeded,
            fee: Some(4200),
            purpose: Some(ConversionPurpose::AutoConversion),
            amount_adjustment: Some(AmountAdjustmentReason::FlooredToMinLimit),
            degradation: Some(SwapDegradation::BelowMinimum),
        };
        let ConversionInfo::Amm {
            pool_id,
            conversion_id,
            status,
            fee,
            purpose,
            amount_adjustment,
            degradation,
        } = resolved_info_from_prior(Some(prior), &sample_pool_key(), ConversionStatus::Refunded)
        else {
            panic!("expected Amm");
        };
        assert_eq!(pool_id, "prior-pool");
        assert_eq!(conversion_id, "conv-abc");
        assert_eq!(status, ConversionStatus::Refunded);
        assert_eq!(fee, Some(4200));
        assert_eq!(purpose, Some(ConversionPurpose::AutoConversion));
        assert_eq!(
            amount_adjustment,
            Some(AmountAdjustmentReason::FlooredToMinLimit)
        );
        // A status change must not erase the verdict on how the swap went.
        assert_eq!(degradation, Some(SwapDegradation::BelowMinimum));
    }

    /// With no prior metadata (row's original write never landed), the mark
    /// falls back to the placeholder keyed on the passed pool id.
    #[test]
    fn refunded_info_falls_back_to_placeholder_when_absent() {
        let ConversionInfo::Amm {
            pool_id,
            conversion_id,
            status,
            fee,
            purpose,
            amount_adjustment,
            ..
        } = resolved_info_from_prior(None, &sample_pool_key(), ConversionStatus::Refunded)
        else {
            panic!("expected Amm");
        };
        assert_eq!(pool_id, sample_pool_key().to_string());
        assert!(!conversion_id.is_empty());
        assert_eq!(status, ConversionStatus::Refunded);
        assert_eq!(fee, None);
        assert_eq!(purpose, None);
        assert_eq!(amount_adjustment, None);
    }

    /// slippage 10 + integrator 5 + margin 100, the production configuration.
    const CEILING_BPS: u128 = 10_115;

    fn check(
        simulated: u128,
        requested: u128,
        target: u128,
        amount_in: u128,
        derived: u128,
    ) -> Result<(), String> {
        check_simulated_output(
            simulated,
            requested,
            target,
            amount_in,
            derived,
            CEILING_BPS,
        )
    }

    #[test]
    fn a_small_btc_conversion_is_bounded_against_the_rounded_target() {
        // Below ~6300 sats the 63-sat round-up exceeds the 1.15% tolerance on
        // its own, so a ceiling built from the raw request rejects these.
        for (requested, target, simulated) in [
            (100u128, 128u128, 129u128),
            (500, 512, 512),
            (1_010, 1_024, 1_025),
            (2_000, 2_048, 2_050),
            (5_000, 5_056, 5_061),
            (6_400, 6_400, 6_406),
        ] {
            assert!(
                check(simulated, requested, target, 1_000, 1_000).is_ok(),
                "rejected a {requested} sat conversion simulating {simulated}"
            );
        }
    }

    #[test]
    fn an_output_above_the_ceiling_is_rejected() {
        let err = check(1_200, 1_000, 1_000, 1_000, 1_000).unwrap_err();
        assert!(err.contains("ceiling"), "{err}");
    }

    #[test]
    fn an_output_below_the_request_is_rejected() {
        let err = check(999, 1_000, 1_000, 1_000, 1_000).unwrap_err();
        assert!(err.contains("at least"), "{err}");
    }

    #[test]
    fn an_output_meeting_the_request_but_under_the_rounded_target_is_accepted() {
        // The plain floor is the request, not the target: a pool returning
        // exactly what was asked for on a small BTC conversion is fine.
        assert!(check(100, 100, 128, 1_000, 1_000).is_ok());
        assert!(check(127, 100, 128, 1_000, 1_000).is_ok());
    }

    #[test]
    fn a_ceiling_that_would_overflow_falls_back_to_the_stricter_bound() {
        // Saturating here would hand back an unbounded ceiling and pass
        // anything; the unscaled bound is the safe direction.
        let err = check(u128::MAX, 1, u128::MAX / 2, 1_000, 1_000).unwrap_err();
        assert!(err.contains("ceiling"), "{err}");
    }

    #[test]
    fn the_ceiling_does_not_judge_a_raised_input() {
        // Mainnet, 2026-08-07: 15,462 token units bought 23 sats, and the
        // 602,650 the minimum raised it to bought 935 rather than the 896 that
        // rate extrapolates to. The rate improves with size, so a ceiling
        // carried over from the small quote refuses a legitimate conversion.
        assert!(check_simulated_output(935, 23, 23, 602_650, 15_462, 10_110).is_ok());
    }

    #[test]
    fn a_small_target_keeps_its_ceiling_margin() {
        // 23 * 10_110 / 10_000 truncates to 23, leaving no tolerance.
        assert!(check_simulated_output(24, 23, 23, 640, 640, 10_110).is_ok());
        assert!(check_simulated_output(25, 23, 23, 640, 640, 10_110).is_err());
    }

    #[test]
    fn a_derived_input_of_zero_keeps_the_unscaled_ceiling() {
        // All-zero operands would exercise nothing. A zero derived input has to
        // leave the base ceiling standing rather than divide by it.
        assert!(check(5_000, 5_000, 5_056, 1_000, 0).is_ok());
        assert!(check(1_000_000, 5_000, 5_056, 1_000, 0).is_err());
        assert!(check(0, 0, 0, 0, 0).is_ok());
    }

    #[test]
    fn the_ceiling_tracks_the_configured_slippage() {
        // Pins the bps construction, which the other tests hardcode.
        let bps =
            10_000 + u128::from(DEFAULT_CONVERSION_MAX_SLIPPAGE_BPS) + OUTPUT_CEILING_MARGIN_BPS;
        assert_eq!(
            bps,
            CEILING_BPS - 5,
            "integrator fee is the remaining 5 bps"
        );
        // A caller raising slippage widens the ceiling rather than being
        // rejected for the extra buffer it asked for.
        assert!(check_simulated_output(1_050, 1_000, 1_000, 1, 1, 10_500).is_ok());
        assert!(check_simulated_output(1_050, 1_000, 1_000, 1, 1, 10_115).is_err());
    }

    #[test]
    fn an_empty_identifier_is_not_delivery() {
        // The pool populating the field but leaving it empty names no transfer.
        // Reading it as delivery would record Completed with nothing received
        // and nothing queued for refund.
        assert_eq!(
            conversion_status(true, Some(""), None),
            ConversionStatus::RefundNeeded
        );
        assert_eq!(
            conversion_status(false, Some(""), Some("")),
            ConversionStatus::RefundNeeded
        );
        assert_eq!(
            conversion_status(true, Some(""), Some("refund")),
            ConversionStatus::Refunded
        );
    }

    #[test]
    fn conversion_status_covers_every_combination() {
        use ConversionStatus::{Completed, RefundNeeded, Refunded};
        let cases = [
            // Took the input and named what paid us.
            (true, Some("in"), Some("refund"), Completed),
            (true, Some("in"), None, Completed),
            // The input came back.
            (true, None, Some("refund"), Refunded),
            (false, Some("in"), Some("refund"), Refunded),
            (false, None, Some("refund"), Refunded),
            // Claimed execution but named nothing. Attempted anyway: the pool
            // may not have run it, and the clawback targets our own transfer.
            (true, None, None, RefundNeeded),
            // Refused and returned nothing: still recoverable. An outbound id
            // here is meaningless, since nothing was delivered.
            (false, Some("in"), None, RefundNeeded),
            (false, None, None, RefundNeeded),
        ];
        for (executed, inbound, refund, expected) in cases {
            assert_eq!(
                conversion_status(executed, inbound, refund),
                expected,
                "executed {executed}, inbound {inbound:?}, refund {refund:?}"
            );
        }
    }

    #[test]
    fn a_delivered_conversion_is_never_marked_for_refund() {
        // A clawback against a transfer the pool consumed is refused and
        // retried on every pass, so it is only worth attempting where the
        // input may still be recoverable.
        for refund in [Some("refund"), None] {
            assert_ne!(
                conversion_status(true, Some("in"), refund),
                ConversionStatus::RefundNeeded
            );
        }
    }

    fn swap(inbound: &str, outbound: &str) -> Swap {
        Swap {
            id: format!("swap-{inbound}"),
            pool_lp_public_key: sample_pool_key(),
            amount_in: 1_000,
            amount_out: 900,
            asset_in_address: "in".to_string(),
            asset_out_address: "out".to_string(),
            price: None,
            timestamp: "2026-08-06T00:00:00Z".to_string(),
            fee_paid: 1,
            pool_asset_a_address: None,
            pool_asset_b_address: None,
            inbound_transfer_id: inbound.to_string(),
            // The listing names a token delivery by its transaction hash, so
            // the label stands in as one rather than being used verbatim.
            outbound_transfer_id: format!("{:0>64}", hex::encode(outbound)),
        }
    }

    #[test]
    fn a_swap_is_matched_by_the_transfer_we_sent() {
        let swaps = [
            swap("someone-elses-input", "delivery-a"),
            swap("our-input", "delivery-b"),
            swap("later-input", "delivery-c"),
        ];
        let found = swap_for_transfer(&swaps, "our-input", None).expect("match");
        assert_eq!(
            found.outbound_transfer_id,
            swap("our-input", "delivery-b").outbound_transfer_id
        );
    }

    #[test]
    fn no_swap_for_the_transfer_is_not_a_match() {
        let swaps = [swap("other-input", "delivery-a")];
        assert!(swap_for_transfer(&swaps, "our-input", None).is_none());
        assert!(swap_for_transfer(&[], "our-input", None).is_none());
    }

    #[test]
    fn the_delivery_side_is_never_matched_against() {
        // Matching the outbound id would attribute a swap whose input was
        // someone else's, marking the conversion completed and dropping a
        // refund that is still owed.
        let swaps = [swap("other-input", "our-input")];
        assert!(swap_for_transfer(&swaps, "our-input", None).is_none());
    }

    #[test]
    fn a_partial_transfer_id_is_not_a_match() {
        let swaps = [swap("our-input-extended", "delivery-a")];
        assert!(swap_for_transfer(&swaps, "our-input", None).is_none());
        assert!(swap_for_transfer(&swaps, "our-input-extended", None).is_some());
    }

    #[test]
    fn a_swap_that_delivered_nothing_is_not_adopted() {
        // Recording it would complete the conversion with nothing received,
        // where leaving it refundable keeps the input in play.
        let mut swaps = [swap("our-input", "delivery-a")];
        swaps[0].amount_out = 0;
        assert!(swap_for_transfer(&swaps, "our-input", None).is_none());
    }

    #[test]
    fn a_delivery_that_cannot_address_a_payment_is_not_adopted() {
        // The shape follows the asset delivered, so each is judged against its
        // own: a Spark transfer id for BTC, a transaction hash for a token.
        // Judged against the other, every real entry of that direction would be
        // refused and the reconcile would quietly stop adopting anything.
        let token_delivery = swap("our-input", "delivery-a");
        assert!(
            swap_for_transfer(std::slice::from_ref(&token_delivery), "our-input", None).is_some()
        );

        let mut btc_delivery = token_delivery.clone();
        btc_delivery.asset_out_address = BTC_ASSET_ADDRESS.to_string();
        btc_delivery.outbound_transfer_id = "019ff036-1c0b-7342-a419-95d170c73fb1".to_string();
        assert!(
            swap_for_transfer(std::slice::from_ref(&btc_delivery), "our-input", None).is_some()
        );

        // Each rejects the other's shape, and both reject an unnamed delivery.
        let mut swapped = btc_delivery.clone();
        swapped.outbound_transfer_id = token_delivery.outbound_transfer_id.clone();
        assert!(swap_for_transfer(&[swapped], "our-input", None).is_none());

        let mut swapped = token_delivery.clone();
        swapped.outbound_transfer_id = btc_delivery.outbound_transfer_id.clone();
        assert!(swap_for_transfer(&[swapped], "our-input", None).is_none());

        for mut empty in [token_delivery, btc_delivery] {
            empty.outbound_transfer_id = String::new();
            assert!(swap_for_transfer(&[empty], "our-input", None).is_none());
        }
    }

    #[test]
    fn the_fee_is_recorded_against_the_leg_holding_the_token() {
        let info = ConversionInfo::Amm {
            pool_id: "pool".to_string(),
            conversion_id: "conv-abc".to_string(),
            status: ConversionStatus::Completed,
            fee: Some(646),
            purpose: Some(ConversionPurpose::AutoConversion),
            amount_adjustment: Some(AmountAdjustmentReason::FlooredToMinLimit),
            degradation: Some(SwapDegradation::BelowMinimum),
        };
        let parts = |info: &ConversionInfo| match info {
            ConversionInfo::Amm {
                fee,
                purpose,
                amount_adjustment,
                degradation,
                ..
            } => (
                *fee,
                purpose.is_some(),
                amount_adjustment.is_some(),
                degradation.is_some(),
            ),
            _ => panic!("not an amm conversion"),
        };

        // Out of the token: the leg that sent it paid the fee.
        let (sent, received) = split_legs(&info, false);
        assert_eq!(parts(&sent).0, Some(646));
        assert_eq!(parts(&received).0, None);

        // Into the token: the fee is denominated in what was delivered.
        let (sent, received) = split_legs(&info, true);
        assert_eq!(parts(&sent).0, None);
        assert_eq!(parts(&received).0, Some(646));

        // The purpose describes the conversion, so it stays on both legs. The
        // adjustment and the degradation describe the input that was sent.
        assert_eq!(parts(&sent), (None, true, true, true));
        assert_eq!(parts(&received), (Some(646), true, false, false));
    }

    #[test]
    fn a_swap_from_another_pool_is_not_adopted() {
        // What this decides is terminal, so an entry from elsewhere would
        // forfeit a refund with no retry.
        let other_pool = PublicKey::from_str(
            "0202e9c857e89901e7211897cc0e69be843f865de845f60589614a693f3c966cef",
        )
        .unwrap();
        let swaps = [swap("our-input", "delivery-a")];

        assert!(swap_for_transfer(&swaps, "our-input", Some(other_pool),).is_none());
        assert!(swap_for_transfer(&swaps, "our-input", Some(sample_pool_key()),).is_some());
        // Unset where the caller does not know which pool ran it.
        assert!(swap_for_transfer(&swaps, "our-input", None).is_some());
    }

    #[test]
    fn an_executed_conversion_keeps_its_prior_fields() {
        let prior = ConversionInfo::Amm {
            pool_id: "pool".to_string(),
            conversion_id: "conversion-1".to_string(),
            status: ConversionStatus::RefundNeeded,
            fee: Some(7),
            purpose: Some(ConversionPurpose::AutoConversion),
            amount_adjustment: None,
            degradation: None,
        };
        let ConversionInfo::Amm {
            conversion_id,
            status,
            fee,
            ..
        } = resolved_info_from_prior(Some(prior), &sample_pool_key(), ConversionStatus::Completed)
        else {
            panic!("expected an AMM conversion");
        };
        assert_eq!(conversion_id, "conversion-1");
        assert_eq!(status, ConversionStatus::Completed);
        assert_eq!(fee, Some(7));
    }
}
