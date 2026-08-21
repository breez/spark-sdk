use std::{str::FromStr, time::Duration};

use bitcoin::{consensus::serialize, hex::DisplayHex};
use platform_utils::tokio;
use spark_wallet::{
    InstantStaticDepositPlan, InstantStaticDepositQuoteResult, ListTransfersRequest, TransferId,
    WalletTransfer,
};
use tracing::{error, info, trace, warn};

use crate::{
    ClaimDepositQuote, ClaimDepositRequest, ClaimDepositResponse, Fee,
    FetchClaimDepositQuoteRequest, FetchClaimDepositQuoteResponse, InstantClaimStatus,
    ListUnclaimedDepositsRequest, ListUnclaimedDepositsResponse, MaxFee, Network,
    RefundDepositRequest, RefundDepositResponse,
    error::SdkError,
    models::Payment,
    persist::UpdateDepositPayload,
    sdk::RuntimeEvent,
    utils::utxo_fetcher::{CachedUtxoFetcher, DetailedUtxo},
};

use super::{BreezSdk, CLAIM_TX_SIZE_VBYTES};

/// Confirmations a deposit needs before the operators treat it as mature. Mirrors
/// operator policy the SDK cannot observe directly.
fn maturity_confirmations(network: Network) -> u32 {
    match network {
        Network::Regtest => 1,
        Network::Mainnet => 3,
    }
}

// Retry parameters for looking up the transfer created by a static deposit
// claim while it propagates across Spark operators.
const CLAIM_TRANSFER_LOOKUP_MAX_ATTEMPTS: u32 = 3;
const CLAIM_TRANSFER_LOOKUP_BASE_DELAY_MS: u64 = 500;

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
#[allow(clippy::needless_pass_by_value)]
impl BreezSdk {
    pub async fn claim_deposit(
        &self,
        request: ClaimDepositRequest,
    ) -> Result<ClaimDepositResponse, SdkError> {
        self.maybe_ensure_spark_private_mode_initialized().await?;
        let detailed_utxo =
            CachedUtxoFetcher::new(self.chain_service.clone(), self.storage.clone())
                .fetch_detailed_utxo(&request.txid, request.vout)
                .await?;

        let max_fee = request
            .max_fee
            .or(self.config.max_deposit_claim_fee.clone());

        // An unreadable depth counts as mature.
        let confirmations = self
            .deposit_confirmations(&request.txid)
            .await
            .unwrap_or_else(|e| {
                warn!(
                    "Could not read the chain depth for {}:{}: {e}",
                    request.txid, request.vout
                );
                u32::MAX
            });
        // Immature deposits take the early path, bounded by the same ceiling.
        if !self
            .is_deposit_mature_at(&detailed_utxo, confirmations)
            .await?
        {
            return self
                .instant_claim_deposit(&detailed_utxo, max_fee, confirmations)
                .await;
        }

        match self.claim_utxo(&detailed_utxo, max_fee).await {
            Ok(transfer_id) => {
                let transfer = self.lookup_claim_transfer_with_retry(transfer_id).await?;
                let payment: Payment = transfer.try_into()?;
                // Insert the payment before returning so callers that
                // immediately list payments see the claim.
                let should_emit_event = self.storage.apply_payment_update(payment.clone()).await?;
                self.storage
                    .delete_deposit(detailed_utxo.txid.to_string(), detailed_utxo.vout)
                    .await?;
                self.event_emitter
                    .emit_runtime_event(RuntimeEvent::DepositClaimed {
                        payment: Box::new(payment.clone()),
                        should_emit_event,
                    })
                    .await;
                Ok(ClaimDepositResponse {
                    payment: Some(payment),
                })
            }
            Err(e) => {
                error!("Failed to claim deposit: {e:?}");
                self.storage
                    .update_deposit(
                        detailed_utxo.txid.to_string(),
                        detailed_utxo.vout,
                        UpdateDepositPayload::ClaimError {
                            error: e.clone().into(),
                        },
                    )
                    .await?;
                Err(e)
            }
        }
    }

    /// Quotes both ways of claiming a deposit, so the caller can offer a choice
    /// between claiming ahead of maturity for a spread and waiting for the cheaper
    /// claim at maturity.
    pub async fn fetch_claim_deposit_quote(
        &self,
        request: FetchClaimDepositQuoteRequest,
    ) -> Result<FetchClaimDepositQuoteResponse, SdkError> {
        let detailed_utxo =
            CachedUtxoFetcher::new(self.chain_service.clone(), self.storage.clone())
                .fetch_detailed_utxo(&request.txid, request.vout)
                .await?;

        let (confirmations, instant, mature) = tokio::join!(
            self.deposit_confirmations(&request.txid),
            self.fetch_instant_claim_quote(&detailed_utxo),
            self.fetch_mature_claim_quote(&detailed_utxo),
        );
        let confirmations = confirmations?;
        let mature = mature?;
        let is_mature = self
            .is_deposit_mature_at(&detailed_utxo, confirmations)
            .await?;
        // Withhold the early claim unless it credits sooner than maturity. The
        // depth it becomes claimable at is reported, not filtered on.
        let instant = instant.filter(|quote| {
            let earlier = quote.confirmations_required < mature.confirmations_required;
            if is_mature || !earlier {
                info!(
                    "Withholding the early claim for {}:{}: {}",
                    request.txid,
                    request.vout,
                    if is_mature {
                        "the deposit has already matured".to_string()
                    } else {
                        format!(
                            "it credits at {} confirmations, no sooner than maturity at {}",
                            quote.confirmations_required, mature.confirmations_required
                        )
                    }
                );
            }
            !is_mature && earlier
        });

        Ok(FetchClaimDepositQuoteResponse {
            amount_sats: detailed_utxo.value,
            confirmations,
            instant,
            mature,
        })
    }

    pub async fn refund_deposit(
        &self,
        request: RefundDepositRequest,
    ) -> Result<RefundDepositResponse, SdkError> {
        let detailed_utxo =
            CachedUtxoFetcher::new(self.chain_service.clone(), self.storage.clone())
                .fetch_detailed_utxo(&request.txid, request.vout)
                .await?;
        let tx = self
            .spark_wallet
            .refund_static_deposit(
                detailed_utxo.clone().tx,
                Some(detailed_utxo.vout),
                &request.destination_address,
                request.fee.into(),
            )
            .await?;
        let tx_hex = serialize(&tx).as_hex().to_string();
        let tx_id = tx.compute_txid().as_raw_hash().to_string();

        // Store the refund transaction details separately
        self.storage
            .update_deposit(
                detailed_utxo.txid.to_string(),
                detailed_utxo.vout,
                UpdateDepositPayload::Refund {
                    refund_tx: tx_hex.clone(),
                    refund_txid: tx_id.clone(),
                },
            )
            .await?;

        self.chain_service
            .broadcast_transaction(tx_hex.clone())
            .await?;
        Ok(RefundDepositResponse { tx_id, tx_hex })
    }

    #[allow(unused_variables)]
    pub async fn list_unclaimed_deposits(
        &self,
        request: ListUnclaimedDepositsRequest,
    ) -> Result<ListUnclaimedDepositsResponse, SdkError> {
        let deposits = self.storage.list_deposits().await?;
        Ok(ListUnclaimedDepositsResponse { deposits })
    }
}

impl BreezSdk {
    /// Looks up the transfer produced by a static deposit claim, retrying
    /// while the Spark operators have not yet indexed it. The SSP commits
    /// the claim synchronously, but there is a brief window before the
    /// transfer becomes queryable from the operators; transient query
    /// errors are also retried. Returns the last error if every attempt
    /// fails.
    async fn lookup_claim_transfer_with_retry(
        &self,
        transfer_id: String,
    ) -> Result<WalletTransfer, SdkError> {
        let parsed_id = TransferId::from_str(&transfer_id).map_err(SdkError::Generic)?;
        let mut last_error: Option<SdkError> = None;

        for attempt in 0..CLAIM_TRANSFER_LOOKUP_MAX_ATTEMPTS {
            if attempt > 0 {
                let delay_ms = CLAIM_TRANSFER_LOOKUP_BASE_DELAY_MS
                    .saturating_mul(2u64.saturating_pow(attempt.saturating_sub(1)));
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                trace!(
                    "Retrying claim transfer lookup (attempt {}/{}) for transfer {transfer_id}",
                    attempt.saturating_add(1),
                    CLAIM_TRANSFER_LOOKUP_MAX_ATTEMPTS
                );
            }

            match self
                .spark_wallet
                .list_transfers(ListTransfersRequest {
                    transfer_ids: vec![parsed_id.clone()],
                    paging: None,
                })
                .await
            {
                Ok(mut resp) => {
                    if let Some(transfer) = resp.items.pop() {
                        return Ok(transfer);
                    }
                    last_error = None;
                }
                Err(e) => last_error = Some(e.into()),
            }
        }

        Err(last_error
            .unwrap_or_else(|| SdkError::Generic("transfer not found after claim".to_string())))
    }

    /// Confirmations on the deposit transaction, 0 while it is unconfirmed. Needs
    /// two chain calls: a transaction reports the height it confirmed at, never its
    /// depth.
    pub(super) async fn deposit_confirmations(&self, txid: &str) -> Result<u32, SdkError> {
        self.deposit_confirmations_at_tip(txid, None).await
    }

    /// Confirmations on the deposit transaction against an already-known chain tip,
    /// which a caller resolving several deposits fetches once.
    pub(super) async fn deposit_confirmations_at_tip(
        &self,
        txid: &str,
        tip_height: Option<u32>,
    ) -> Result<u32, SdkError> {
        let status = self
            .chain_service
            .get_transaction_status(txid.to_string())
            .await?;
        // An unconfirmed transaction can still carry a height.
        if !status.confirmed {
            return Ok(0);
        }
        // Confirmed without a height: at least one deep.
        let Some(block_height) = status.block_height else {
            return Ok(1);
        };
        let tip_height = match tip_height {
            Some(tip) => tip,
            None => self.chain_service.tip_height().await?,
        };
        Ok(tip_height.saturating_sub(block_height).saturating_add(1))
    }

    /// Quotes claiming `detailed_utxo` ahead of maturity. `None` when the provider
    /// offers nothing for it, including when it cannot yet be quoted at all: this
    /// is one of two options being priced for display, so it reports absence
    /// rather than failing the call.
    async fn fetch_instant_claim_quote(
        &self,
        detailed_utxo: &DetailedUtxo,
    ) -> Option<ClaimDepositQuote> {
        let quote_result = self
            .spark_wallet
            .fetch_instant_static_deposit_quote(detailed_utxo.tx.clone(), Some(detailed_utxo.vout))
            .await
            .inspect_err(|e| {
                info!(
                    "No instant quote for {}:{}: {e}",
                    detailed_utxo.txid, detailed_utxo.vout
                );
            })
            .ok()?;
        let plan = quote_result
            .fulfillment_plans
            .iter()
            .min_by_key(|p| p.confirmations)?;
        // What the provider offered, before filtering.
        info!(
            "Early claim quoted for {}:{} ({} sats): credits {} at {} confirmations",
            detailed_utxo.txid,
            detailed_utxo.vout,
            detailed_utxo.value,
            quote_result.quote.credit_amount.original_value,
            plan.confirmations
        );
        Some(claim_deposit_quote(
            u32::try_from(plan.confirmations.unsigned_abs()).unwrap_or(u32::MAX),
            detailed_utxo.value,
            quote_result.quote.credit_amount.original_value,
            false,
        ))
    }

    /// Quotes claiming `detailed_utxo` at maturity. The provider may decline to
    /// quote a deposit that has not matured, so this falls back to an estimate
    /// from current on-chain fees.
    async fn fetch_mature_claim_quote(
        &self,
        detailed_utxo: &DetailedUtxo,
    ) -> Result<ClaimDepositQuote, SdkError> {
        match self
            .spark_wallet
            .fetch_static_deposit_claim_quote(detailed_utxo.tx.clone(), Some(detailed_utxo.vout))
            .await
        {
            Ok(quote) => Ok(claim_deposit_quote(
                maturity_confirmations(self.config.network),
                detailed_utxo.value,
                quote.credit_amount_sats,
                false,
            )),
            Err(e) => {
                info!(
                    "No mature quote for {}:{}, estimating: {e}",
                    detailed_utxo.txid, detailed_utxo.vout
                );
                let fee_sats = self
                    .chain_service
                    .recommended_fees()
                    .await?
                    .fastest_fee
                    .saturating_mul(CLAIM_TX_SIZE_VBYTES);
                Ok(claim_deposit_quote(
                    maturity_confirmations(self.config.network),
                    detailed_utxo.value,
                    detailed_utxo.value.saturating_sub(fee_sats),
                    true,
                ))
            }
        }
    }

    /// Whether an early claim on this deposit is still settling. Until it does and
    /// the deposit is reconciled away, any further claim would be a second claim on
    /// the same UTXO.
    async fn instant_claim_in_flight(&self, txid: String, vout: u32) -> Result<bool, SdkError> {
        Ok(self
            .storage
            .list_deposits()
            .await?
            .iter()
            .find(|d| d.txid == txid && d.vout == vout)
            .and_then(|d| d.instant_claim_status.as_ref())
            .is_some_and(|s| matches!(s, InstantClaimStatus::Submitted { .. })))
    }

    /// Whether the deposit can be claimed at maturity rather than early: true if the
    /// operators say so, or the chain alone is deep enough. Either suffices because
    /// the stored flag is only as fresh as the last deposit sync, which need never
    /// have run, and trusting it alone pays a spread on a deposit that was already
    /// claimable at maturity.
    async fn is_deposit_mature_at(
        &self,
        detailed_utxo: &DetailedUtxo,
        confirmations: u32,
    ) -> Result<bool, SdkError> {
        let stored_mature = self
            .storage
            .list_deposits()
            .await?
            .into_iter()
            .find(|d| d.txid == detailed_utxo.txid.to_string() && d.vout == detailed_utxo.vout)
            .is_some_and(|d| d.is_mature);
        let required = maturity_confirmations(self.config.network);
        let is_mature = stored_mature || confirmations >= required;
        info!(
            "Deposit {}:{} is {} (operators: {}, chain: {}/{} confirmations)",
            detailed_utxo.txid,
            detailed_utxo.vout,
            if is_mature {
                "mature"
            } else {
                "not yet mature"
            },
            if stored_mature {
                "mature"
            } else {
                "not mature"
            },
            confirmations,
            required
        );
        Ok(is_mature)
    }

    /// Claims a specific not-yet-mature deposit instantly, on demand.
    /// The transfer settles asynchronously, so no payment is returned.
    async fn instant_claim_deposit(
        &self,
        detailed_utxo: &DetailedUtxo,
        max_fee: Option<MaxFee>,
        confirmations: u32,
    ) -> Result<ClaimDepositResponse, SdkError> {
        if self
            .instant_claim_in_flight(detailed_utxo.txid.to_string(), detailed_utxo.vout)
            .await?
        {
            info!(
                "Early claim already in flight for utxo {}:{}",
                detailed_utxo.txid, detailed_utxo.vout
            );
            return Ok(ClaimDepositResponse { payment: None });
        }

        let row_exists = self
            .storage
            .list_deposits()
            .await?
            .iter()
            .any(|d| d.txid == detailed_utxo.txid.to_string() && d.vout == detailed_utxo.vout);

        let resolved_max_fee = self.resolve_max_claim_fee(max_fee).await?;
        let outcome = match self
            .instant_claim_utxo(detailed_utxo, resolved_max_fee, confirmations)
            .await
        {
            Ok(outcome) => outcome,
            // Transient quote-fetch failure: leave unmarked so a retry works.
            Err(e) => {
                error!("Instant claim transient error: {e:?}");
                return Err(e);
            }
        };

        // Persist the resolved status. A manual claim can run before the background
        // sync has inserted the deposit row, in which case update_deposit would be a
        // no-op and the marker (which stops the sync from re-submitting or
        // normal-claiming a still-in-flight deposit) would be lost, so insert the row
        // first when missing. reconcile_deposits removes it once the claim settles.
        if !row_exists {
            self.storage
                .add_deposit(
                    detailed_utxo.txid.to_string(),
                    detailed_utxo.vout,
                    detailed_utxo.value,
                    false,
                )
                .await?;
        }
        self.storage
            .update_deposit(
                detailed_utxo.txid.to_string(),
                detailed_utxo.vout,
                UpdateDepositPayload::InstantClaim {
                    status: outcome.status(confirmations),
                },
            )
            .await?;

        match outcome {
            InstantClaimOutcome::Submitted(claim_id) => {
                info!(
                    "Instant claimed utxo {}:{} with claim_id: {claim_id}",
                    detailed_utxo.txid, detailed_utxo.vout
                );
                Ok(ClaimDepositResponse { payment: None })
            }
            InstantClaimOutcome::Declined { error, .. } => {
                error!("Instant claim declined: {error:?}");
                Err(error)
            }
        }
    }

    /// Attempts an instant static deposit claim for `detailed_utxo`, ahead of
    /// maturity, bounded by the same fee ceiling the claim at maturity uses.
    /// `Ok(Submitted)` on a submitted claim, `Ok(Declined)` for a terminal outcome
    /// (no plan offered, spread over the ceiling, or a rejected claim), and `Err`
    /// for the transient cases that should be retried: a failed quote fetch (the
    /// SSP may not have indexed the tx yet) and a claim the SSP rejected for
    /// insufficient depth.
    pub(super) async fn instant_claim_utxo(
        &self,
        detailed_utxo: &DetailedUtxo,
        resolved_max_fee: Option<(Fee, u64)>,
        confirmations: u32,
    ) -> Result<InstantClaimOutcome, SdkError> {
        // An unresolved max fee is recorded as a zero ceiling: it admits nothing,
        // and unlike none it stays retryable once one is configured.
        let max_fee_sats = resolved_max_fee.as_ref().map_or(0, |(_, sats)| *sats);

        let quote_result = self
            .spark_wallet
            .fetch_instant_static_deposit_quote(detailed_utxo.tx.clone(), Some(detailed_utxo.vout))
            .await?;
        info!(
            "Instant quote for {}:{} ({} sats, ceiling {} sats): {quote_result:?}",
            detailed_utxo.txid, detailed_utxo.vout, detailed_utxo.value, max_fee_sats
        );
        // Price the spread against the on-chain UTXO value we already hold, not the
        // SSP-reported deposit amount, so the fee gate does not depend on the quote.
        match select_instant_claim_plan(
            &quote_result,
            detailed_utxo.value,
            max_fee_sats,
            confirmations,
            maturity_confirmations(self.config.network),
        ) {
            InstantClaimPlan::Claimable(plan) => {
                match self
                    .spark_wallet
                    .claim_instant_static_deposit(
                        detailed_utxo.tx.clone(),
                        quote_result.quote,
                        plan,
                    )
                    .await
                {
                    Ok(claim_id) => Ok(InstantClaimOutcome::Submitted(claim_id)),
                    // A depth rejection: the deposit is below the plan's
                    // confirmations, or the operators disagree on how deep it is.
                    Err(e) if is_pending_confirmation_error(&e.to_string()) => Err(e.into()),
                    // The provider rejected the submission or could not be reached.
                    Err(e) => Ok(InstantClaimOutcome::Declined {
                        error: e.into(),
                        max_fee_sats: None,
                    }),
                }
            }
            InstantClaimPlan::NoPlan => Ok(InstantClaimOutcome::Declined {
                error: SdkError::Generic("No instant claim plan available".to_string()),
                max_fee_sats: None,
            }),
            InstantClaimPlan::FeeExceeded {
                quoted_sats,
                quoted_rate,
            } => Ok(InstantClaimOutcome::Declined {
                error: SdkError::MaxDepositClaimFeeExceeded {
                    tx: detailed_utxo.txid.to_string(),
                    vout: detailed_utxo.vout,
                    max_fee: resolved_max_fee.map(|(fee, _)| fee),
                    required_fee_sats: quoted_sats,
                    required_fee_rate_sat_per_vbyte: quoted_rate,
                },
                max_fee_sats: Some(max_fee_sats),
            }),
        }
    }
}

/// Result of an instant claim attempt.
pub(super) enum InstantClaimOutcome {
    /// The claim was submitted and carries the claim id.
    Submitted(String),
    /// The claim was declined: no plan offered, the spread was over the ceiling,
    /// or the submission failed. `max_fee_sats` is the ceiling that declined it,
    /// unset when no ceiling was involved.
    Declined {
        error: SdkError,
        max_fee_sats: Option<u64>,
    },
}

impl InstantClaimOutcome {
    /// The status to persist on the deposit for this resolved outcome.
    pub(super) fn status(&self, confirmations: u32) -> InstantClaimStatus {
        match self {
            InstantClaimOutcome::Submitted(claim_id) => InstantClaimStatus::Submitted {
                claim_id: claim_id.clone(),
            },
            InstantClaimOutcome::Declined { max_fee_sats, .. } => InstantClaimStatus::Declined {
                max_fee_sats: *max_fee_sats,
                confirmations,
            },
        }
    }
}

/// Message fragments of the depth rejections that clear on their own. Neither the
/// SSP nor the operators return an error code for these, so the message is all
/// there is to go on. `enough confirmations` covers the SSP and the operators
/// alike, which differ only by contraction; the second covers the SSP's separate
/// "deep enough on some operators but not all" rejection.
const PENDING_CONFIRMATION_MARKERS: [&str; 2] =
    ["enough confirmations", "operators have not seen it"];

/// Whether a rejected claim is the SSP or the operators waiting for the UTXO to
/// reach the required depth, which clears within seconds. A phrasing outside
/// [`PENDING_CONFIRMATION_MARKERS`] is terminal, so the deposit stops being
/// retried and falls through to the claim at maturity.
fn is_pending_confirmation_error(message: &str) -> bool {
    let message = message.to_lowercase();
    PENDING_CONFIRMATION_MARKERS
        .iter()
        .any(|marker| message.contains(marker))
}

/// Prices one way of claiming a deposit, from the credit it would leave.
fn claim_deposit_quote(
    confirmations_required: u32,
    deposit_sats: u64,
    credit_amount_sats: u64,
    is_estimate: bool,
) -> ClaimDepositQuote {
    let fee_sats = deposit_sats.saturating_sub(credit_amount_sats);
    ClaimDepositQuote {
        confirmations_required,
        credit_amount_sats,
        fee_sats,
        fee_rate_sat_per_vbyte: fee_sats.div_ceil(CLAIM_TX_SIZE_VBYTES),
        is_estimate,
    }
}

/// Classification of an instant quote's shallowest plan against the fee ceiling.
enum InstantClaimPlan {
    /// The plan is within the ceiling and should be claimed.
    Claimable(InstantStaticDepositPlan),
    /// The quote carried no fulfillment plans at all.
    NoPlan,
    /// The SSP spread (`deposit - credit`) exceeds the ceiling, in sats and as the
    /// on-chain rate it implies over the claim tx (both for the decline message).
    FeeExceeded { quoted_sats: u64, quoted_rate: u64 },
}

/// Selects the shallowest fulfillment plan (the one crediting at the fewest
/// confirmations) and checks the SSP spread (`deposit - credit`) against
/// `max_fee_sats`, the same resolved ceiling the claim at maturity is held to. An
/// unset ceiling arrives here as zero, which admits nothing.
fn select_instant_claim_plan(
    quote_result: &InstantStaticDepositQuoteResult,
    deposit_sats: u64,
    max_fee_sats: u64,
    confirmations: u32,
    maturity_confirmations: u32,
) -> InstantClaimPlan {
    let Some(plan) = quote_result
        .fulfillment_plans
        .iter()
        .min_by_key(|p| p.confirmations)
    else {
        return InstantClaimPlan::NoPlan;
    };
    // The deposit is deep enough to claim at maturity, so an early claim buys
    // nothing. The operator feed can still report it immature.
    if u64::from(confirmations) >= u64::from(maturity_confirmations) {
        return InstantClaimPlan::NoPlan;
    }
    // Skip plans that only credit once the deposit has matured anyway.
    let plan_confirmations = plan.confirmations.unsigned_abs();
    if plan_confirmations >= u64::from(maturity_confirmations) {
        return InstantClaimPlan::NoPlan;
    }
    // Skip plans the deposit is not deep enough for yet: submitting one is
    // rejected on depth.
    if plan_confirmations > u64::from(confirmations) {
        return InstantClaimPlan::NoPlan;
    }
    // Priced off the quote's credit, which is what the claim signs.
    let quoted_sats = deposit_sats.saturating_sub(quote_result.quote.credit_amount.original_value);
    if quoted_sats <= max_fee_sats {
        InstantClaimPlan::Claimable(plan.clone())
    } else {
        InstantClaimPlan::FeeExceeded {
            quoted_sats,
            quoted_rate: quoted_sats.div_ceil(CLAIM_TX_SIZE_VBYTES),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        InstantClaimPlan, claim_deposit_quote, is_pending_confirmation_error,
        select_instant_claim_plan,
    };
    use spark_wallet::{
        CurrencyAmount, InstantStaticDepositPlan, InstantStaticDepositQuote,
        InstantStaticDepositQuoteResult,
    };

    fn sats(value: u64) -> CurrencyAmount {
        CurrencyAmount {
            original_value: value,
            ..Default::default()
        }
    }

    /// Builds a quote for a `deposit_sats` UTXO with one fulfillment plan per
    /// `(confirmations, credit_sats)` pair. The quote-level credit mirrors the
    /// first plan's.
    fn quote_result(deposit_sats: u64, plans: &[(i64, u64)]) -> InstantStaticDepositQuoteResult {
        InstantStaticDepositQuoteResult {
            quote: InstantStaticDepositQuote {
                id: "quote-id".to_string(),
                transaction_id: "tx".to_string(),
                output_index: 0,
                deposit_amount: sats(deposit_sats),
                credit_amount: sats(plans.first().map_or(0, |(_, credit)| *credit)),
                quote_signature: "00".to_string(),
            },
            fulfillment_plans: plans
                .iter()
                .enumerate()
                .map(
                    |(i, (confirmations, credit_sats))| InstantStaticDepositPlan {
                        id: format!("plan-{i}"),
                        amount: sats(*credit_sats),
                        confirmations: *confirmations,
                    },
                )
                .collect(),
        }
    }

    #[test]
    fn quotes_the_fee_as_the_credit_shortfall() {
        // 20_000 deposit crediting 18_810 costs 1_190, which over the 99 vbyte
        // claim tx rounds up to 13 sat/vbyte.
        let quote = claim_deposit_quote(1, 20_000, 18_810, false);
        assert_eq!(quote.confirmations_required, 1);
        assert_eq!(quote.credit_amount_sats, 18_810);
        assert_eq!(quote.fee_sats, 1_190);
        assert_eq!(quote.fee_rate_sat_per_vbyte, 13);
        assert!(!quote.is_estimate);
    }

    #[test]
    fn quotes_a_free_claim_at_a_zero_rate() {
        let quote = claim_deposit_quote(3, 20_000, 20_000, true);
        assert_eq!(quote.fee_sats, 0);
        assert_eq!(quote.fee_rate_sat_per_vbyte, 0);
        assert!(quote.is_estimate);
    }

    #[test]
    fn quotes_zero_rather_than_underflowing_on_a_credit_above_the_deposit() {
        // Not something the provider should return, but the arithmetic must not
        // wrap into an enormous fee if it ever does.
        let quote = claim_deposit_quote(0, 20_000, 25_000, false);
        assert_eq!(quote.fee_sats, 0);
        assert_eq!(quote.fee_rate_sat_per_vbyte, 0);
    }

    #[test]
    fn selects_zero_conf_plan_within_ceiling() {
        // Spread 1_000, ceiling 2_000 -> claim.
        let q = quote_result(100_000, &[(0, 99_000), (1, 99_500)]);
        let InstantClaimPlan::Claimable(plan) = select_instant_claim_plan(&q, 100_000, 2_000, 0, 3)
        else {
            panic!("expected a claimable 0-conf plan");
        };
        assert_eq!(plan.confirmations, 0);
    }

    #[test]
    fn selects_shallowest_plan_when_no_zero_conf_plan() {
        // A deposit that has already confirmed gets no 0-conf plan: claim at the
        // shallowest depth offered rather than waiting for maturity.
        let q = quote_result(100_000, &[(1, 99_000), (2, 99_500)]);
        let InstantClaimPlan::Claimable(plan) = select_instant_claim_plan(&q, 100_000, 2_000, 1, 3)
        else {
            panic!("expected the 1-conf plan to be claimable");
        };
        assert_eq!(plan.confirmations, 1);
    }

    #[test]
    fn selects_shallowest_plan_regardless_of_order() {
        let q = quote_result(100_000, &[(3, 99_900), (1, 99_500), (0, 99_000)]);
        let InstantClaimPlan::Claimable(plan) = select_instant_claim_plan(&q, 100_000, 2_000, 0, 3)
        else {
            panic!("expected a claimable plan");
        };
        assert_eq!(plan.confirmations, 0);
    }

    #[test]
    fn skips_a_plan_that_credits_no_sooner_than_maturity() {
        // The SSP quotes its floor, not this deposit's depth, so a deposit well
        // past maturity is still offered a shallow plan. Claiming it would pay a
        // spread for a wait the deposit no longer has.
        let q = quote_result(100_000, &[(1, 99_000)]);
        assert!(matches!(
            select_instant_claim_plan(&q, 100_000, 10_000, 1, 1),
            InstantClaimPlan::NoPlan
        ));
        // The same plan is worth claiming when maturity really is further out.
        assert!(matches!(
            select_instant_claim_plan(&q, 100_000, 10_000, 1, 3),
            InstantClaimPlan::Claimable(_)
        ));
    }

    #[test]
    fn skips_when_the_deposit_is_already_deep_enough_to_mature() {
        // The operator feed can lag the chain, so a deposit past maturity depth
        // still reaches the selector. Paying a spread buys no time.
        let q = quote_result(100_000, &[(1, 99_000)]);
        assert!(matches!(
            select_instant_claim_plan(&q, 100_000, 10_000, 5, 3),
            InstantClaimPlan::NoPlan
        ));
    }

    #[test]
    fn skips_a_plan_the_deposit_is_not_deep_enough_for() {
        // The SSP quotes a floor above this deposit's depth: submitting it is
        // rejected on depth, so wait for a confirmation and re-quote.
        let q = quote_result(100_000, &[(2, 99_000)]);
        assert!(matches!(
            select_instant_claim_plan(&q, 100_000, 10_000, 1, 3),
            InstantClaimPlan::NoPlan
        ));
        // The same plan is claimable once the deposit reaches the floor.
        assert!(matches!(
            select_instant_claim_plan(&q, 100_000, 10_000, 2, 3),
            InstantClaimPlan::Claimable(_)
        ));
    }

    #[test]
    fn skips_when_no_plans_offered() {
        let q = quote_result(100_000, &[]);
        assert!(matches!(
            select_instant_claim_plan(&q, 100_000, 100_000, 0, 3),
            InstantClaimPlan::NoPlan
        ));
    }

    #[test]
    fn declines_when_no_ceiling_is_set() {
        // An unset max fee reaches the gate as zero, which admits nothing. It is
        // recorded as a real ceiling, so configuring one later still retries.
        let q = quote_result(100_000, &[(0, 99_000)]);
        assert!(matches!(
            select_instant_claim_plan(&q, 100_000, 0, 0, 3),
            InstantClaimPlan::FeeExceeded { .. }
        ));
    }

    #[test]
    fn gates_on_the_credit_the_claim_signs() {
        // The user statement signs the quote's credit, so that is what the ceiling
        // has to be checked against. Here the quote credits 90_000 while the plan
        // that gets selected (the shallowest) names 99_000: gating on the plan
        // would see a 1_000 spread and admit it, when the claim actually authorizes
        // a 10_000 one.
        let q = quote_result(100_000, &[(3, 90_000), (0, 99_000)]);
        assert_eq!(q.quote.credit_amount.original_value, 90_000);
        assert!(matches!(
            select_instant_claim_plan(&q, 100_000, 2_000, 0, 3),
            InstantClaimPlan::FeeExceeded {
                quoted_sats: 10_000,
                ..
            }
        ));
    }

    #[test]
    fn skips_when_spread_over_ceiling() {
        // Spread 5_000 against a 1_000 ceiling -> skip. The reported rate is the
        // spread over the claim tx, so it is comparable with `MaxFee::Rate`.
        let q = quote_result(100_000, &[(0, 95_000)]);
        assert!(matches!(
            select_instant_claim_plan(&q, 100_000, 1_000, 0, 3),
            InstantClaimPlan::FeeExceeded {
                quoted_sats: 5_000,
                quoted_rate: 51
            }
        ));
    }

    #[test]
    fn rejects_any_spread_at_a_zero_ceiling() {
        let q = quote_result(100_000, &[(0, 99_000)]);
        assert!(matches!(
            select_instant_claim_plan(&q, 100_000, 0, 0, 3),
            InstantClaimPlan::FeeExceeded { .. }
        ));
    }

    #[test]
    fn accepts_spread_equal_to_ceiling() {
        // Inclusive at the ceiling.
        let q = quote_result(100_000, &[(0, 99_000)]);
        assert!(matches!(
            select_instant_claim_plan(&q, 100_000, 1_000, 0, 3),
            InstantClaimPlan::Claimable(_)
        ));
    }

    #[test]
    fn one_ceiling_admits_small_declines_large() {
        // The SSP spread carries a term proportional to the deposit, so an absolute
        // ceiling admits small deposits and makes large ones wait: the inverse of a
        // bps cap. Spreads model `100 flat + 990 on-chain + 50 bps`.
        let ceiling = 2_000;
        // 20_000 deposit: spread 1_190 -> claimed.
        let small = quote_result(20_000, &[(0, 18_810)]);
        assert!(matches!(
            select_instant_claim_plan(&small, 20_000, ceiling, 0, 3),
            InstantClaimPlan::Claimable(_)
        ));
        // 1_000_000 deposit: spread 6_090 -> declined, waits for maturity.
        let large = quote_result(1_000_000, &[(0, 993_910)]);
        assert!(matches!(
            select_instant_claim_plan(&large, 1_000_000, ceiling, 0, 3),
            InstantClaimPlan::FeeExceeded { .. }
        ));
    }

    #[test]
    fn treats_ssp_depth_rejections_as_retryable() {
        // Verbatim SSP rejections: the UTXO is not deep enough yet, or is deep
        // enough but has not propagated to every operator.
        assert!(is_pending_confirmation_error(
            "graphql error: UTXO does not have enough confirmations. Required: 1, got: 0"
        ));
        assert!(is_pending_confirmation_error(
            "graphql error: UTXO needs 1 confirmations on every Spark operator before it \
             can be claimed. Some operators have not seen it that deep yet. Retry in a \
             few seconds."
        ));
        // The Spark operators phrase it differently again, with a contraction.
        assert!(is_pending_confirmation_error(
            "deposit tx doesn't have enough confirmations: confirmation height: 100 \
             current block height: 100"
        ));
        // A rejected statement is terminal and must stay that way.
        assert!(!is_pending_confirmation_error(
            "graphql error: Something went wrong."
        ));
    }

    #[test]
    fn prices_spread_off_passed_deposit_not_quote() {
        // The quote claims a 100_000 deposit, but we pass the real on-chain value
        // (50_000). Spread is priced off the passed value: 50_000 - 49_500 = 500,
        // within the 1_000 ceiling -> claim. Pricing off the quote's 100_000 would
        // give a 50_500 spread and decline, so a claim proves the passed value
        // drives the gate.
        let q = quote_result(100_000, &[(0, 49_500)]);
        assert!(matches!(
            select_instant_claim_plan(&q, 50_000, 1_000, 0, 3),
            InstantClaimPlan::Claimable(_)
        ));
    }
}
