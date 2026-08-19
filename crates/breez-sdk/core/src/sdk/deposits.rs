use std::{str::FromStr, time::Duration};

use bitcoin::{consensus::serialize, hex::DisplayHex};
use platform_utils::tokio;
use spark_wallet::{
    InstantStaticDepositPlan, InstantStaticDepositQuoteResult, ListTransfersRequest, TransferId,
    WalletTransfer,
};
use tracing::{error, info, trace};

use crate::{
    ClaimDepositRequest, ClaimDepositResponse, InstantClaimDeclineReason, InstantClaimStatus,
    ListUnclaimedDepositsRequest, ListUnclaimedDepositsResponse, RefundDepositRequest,
    RefundDepositResponse,
    error::SdkError,
    models::Payment,
    persist::UpdateDepositPayload,
    sdk::RuntimeEvent,
    utils::utxo_fetcher::{CachedUtxoFetcher, DetailedUtxo},
};

use super::BreezSdk;

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

        if let Some(max_instant_fee_bps) = request.max_instant_fee_bps {
            return self
                .instant_claim_deposit(&detailed_utxo, max_instant_fee_bps)
                .await;
        }

        let max_fee = request
            .max_fee
            .or(self.config.max_deposit_claim_fee.clone());
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

    /// Claims a specific not-yet-mature deposit instantly, on demand.
    /// The transfer settles asynchronously, so no payment is returned.
    async fn instant_claim_deposit(
        &self,
        detailed_utxo: &DetailedUtxo,
        max_instant_fee_bps: u32,
    ) -> Result<ClaimDepositResponse, SdkError> {
        let existing = self
            .storage
            .list_deposits()
            .await?
            .into_iter()
            .find(|d| d.txid == detailed_utxo.txid.to_string() && d.vout == detailed_utxo.vout);
        // Do not re-submit a claim that is already in flight for this deposit.
        if matches!(
            existing
                .as_ref()
                .and_then(|d| d.instant_claim_status.as_ref()),
            Some(InstantClaimStatus::Submitted { .. })
        ) {
            info!(
                "Instant claim already in flight for utxo {}:{}",
                detailed_utxo.txid, detailed_utxo.vout
            );
            return Ok(ClaimDepositResponse { payment: None });
        }
        let row_exists = existing.is_some();

        let outcome = match self
            .instant_claim_utxo(detailed_utxo, max_instant_fee_bps)
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
                    status: outcome.status(),
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
    /// maturity. `Ok(Submitted)` on a submitted claim, `Ok(Declined)` for a terminal
    /// outcome (no plan offered, spread over the ceiling, or a rejected claim), and
    /// `Err` for the transient cases that should be retried: a failed quote fetch
    /// (the SSP may not have indexed the tx yet) and a claim the SSP rejected for
    /// insufficient depth.
    pub(super) async fn instant_claim_utxo(
        &self,
        detailed_utxo: &DetailedUtxo,
        max_instant_fee_bps: u32,
    ) -> Result<InstantClaimOutcome, SdkError> {
        // A failed quote fetch is transient (retry); everything after it is
        // terminal and should be marked, not retried.
        let quote_result = self
            .spark_wallet
            .fetch_instant_static_deposit_quote(detailed_utxo.tx.clone(), Some(detailed_utxo.vout))
            .await?;
        // Log the plans: a decline is otherwise indistinguishable from the SSP
        // offering none. The UTXO value is not in the quote, and it is what the
        // spread is priced against below.
        info!(
            "Instant quote for {}:{} ({} sats): {quote_result:?}",
            detailed_utxo.txid, detailed_utxo.vout, detailed_utxo.value
        );
        // Price the spread against the on-chain UTXO value we already hold, not the
        // SSP-reported deposit amount, so the fee gate does not depend on the quote.
        match select_instant_claim_plan(&quote_result, detailed_utxo.value, max_instant_fee_bps) {
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
                    // A depth rejection clears on its own, so retry it rather than
                    // marking the deposit, which would be terminal. This is the
                    // common case: the cascade attempts as soon as the operators
                    // surface the deposit, which is when they are still catching up.
                    Err(e) if is_pending_confirmation_error(&e.to_string()) => Err(e.into()),
                    Err(e) => Ok(InstantClaimOutcome::Declined {
                        error: e.into(),
                        reason: InstantClaimDeclineReason::SubmissionFailed,
                    }),
                }
            }
            InstantClaimPlan::NoPlan => Ok(InstantClaimOutcome::Declined {
                error: SdkError::Generic("No instant claim plan available".to_string()),
                reason: InstantClaimDeclineReason::NoPlan,
            }),
            InstantClaimPlan::FeeExceeded {
                quoted_sats,
                quoted_bps,
            } => Ok(InstantClaimOutcome::Declined {
                error: SdkError::Generic(format!(
                    "Instant claim declined for {}:{}: SSP spread {quoted_bps} bps ({quoted_sats} sats) exceeds max {max_instant_fee_bps} bps",
                    detailed_utxo.txid, detailed_utxo.vout
                )),
                reason: InstantClaimDeclineReason::FeeExceeded {
                    max_bps: max_instant_fee_bps,
                    quoted_bps,
                    quoted_sats,
                },
            }),
        }
    }
}

/// Result of an instant claim attempt.
pub(super) enum InstantClaimOutcome {
    /// Claim submitted; carries the claim id. Settles asynchronously.
    Submitted(String),
    /// A resolved decline to mark rather than retry via `Err`: no plan offered,
    /// spread over the ceiling, or a failed submission. Distinct from a failed
    /// quote fetch, which is transient and retries. Carries the error for the
    /// caller and the reason for the persisted status.
    Declined {
        error: SdkError,
        reason: InstantClaimDeclineReason,
    },
}

impl InstantClaimOutcome {
    /// The status to persist on the deposit for this resolved outcome.
    pub(super) fn status(&self) -> InstantClaimStatus {
        match self {
            InstantClaimOutcome::Submitted(claim_id) => InstantClaimStatus::Submitted {
                claim_id: claim_id.clone(),
            },
            InstantClaimOutcome::Declined { reason, .. } => InstantClaimStatus::Declined {
                reason: reason.clone(),
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

/// Classification of an instant quote's shallowest plan against the bps ceiling.
enum InstantClaimPlan {
    /// The plan is within the ceiling and should be claimed.
    Claimable(InstantStaticDepositPlan),
    /// The quote carried no fulfillment plans at all.
    NoPlan,
    /// The SSP spread (`deposit - credit`) exceeds the ceiling, in both sats and
    /// its basis-points-of-deposit form (for the decline message).
    FeeExceeded { quoted_sats: u64, quoted_bps: u32 },
}

/// Selects the shallowest fulfillment plan (the one crediting at the fewest
/// confirmations) and checks the SSP spread (`deposit - credit`) against `max_bps`,
/// as basis points of `deposit_sats` (the on-chain UTXO value). The spread carries a
/// fixed component, so its bps grows as the deposit shrinks: a single bps ceiling
/// therefore admits large deposits and declines small ones.
fn select_instant_claim_plan(
    quote_result: &InstantStaticDepositQuoteResult,
    deposit_sats: u64,
    max_bps: u32,
) -> InstantClaimPlan {
    // Ties keep the SSP's ordering: min_by_key returns the first of equal minima.
    let Some(plan) = quote_result
        .fulfillment_plans
        .iter()
        .min_by_key(|p| p.confirmations)
    else {
        return InstantClaimPlan::NoPlan;
    };
    let quoted_sats = deposit_sats.saturating_sub(plan.amount.original_value);
    let within = u128::from(quoted_sats).saturating_mul(10_000)
        <= u128::from(max_bps).saturating_mul(u128::from(deposit_sats));
    if within {
        InstantClaimPlan::Claimable(plan.clone())
    } else {
        // spread <= deposit, so quoted_bps <= 10_000; it fits u32 (the ceiling type).
        let quoted_bps = u128::from(quoted_sats)
            .saturating_mul(10_000)
            .checked_div(u128::from(deposit_sats))
            .and_then(|bps| u32::try_from(bps).ok())
            .unwrap_or(0);
        InstantClaimPlan::FeeExceeded {
            quoted_sats,
            quoted_bps,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{InstantClaimPlan, is_pending_confirmation_error, select_instant_claim_plan};
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
    fn selects_zero_conf_plan_within_bps() {
        // Spread 1_000 of 100_000 = 100 bps, ceiling 200 bps -> claim.
        let q = quote_result(100_000, &[(0, 99_000), (1, 99_500)]);
        let InstantClaimPlan::Claimable(plan) = select_instant_claim_plan(&q, 100_000, 200) else {
            panic!("expected a claimable 0-conf plan");
        };
        assert_eq!(plan.confirmations, 0);
    }

    #[test]
    fn selects_shallowest_plan_when_no_zero_conf_plan() {
        // A deposit that has already confirmed gets no 0-conf plan: claim at the
        // shallowest depth offered rather than waiting for maturity.
        let q = quote_result(100_000, &[(1, 99_000), (2, 99_500)]);
        let InstantClaimPlan::Claimable(plan) = select_instant_claim_plan(&q, 100_000, 200) else {
            panic!("expected the 1-conf plan to be claimable");
        };
        assert_eq!(plan.confirmations, 1);
    }

    #[test]
    fn selects_shallowest_plan_regardless_of_order() {
        let q = quote_result(100_000, &[(3, 99_900), (1, 99_500), (0, 99_000)]);
        let InstantClaimPlan::Claimable(plan) = select_instant_claim_plan(&q, 100_000, 200) else {
            panic!("expected a claimable plan");
        };
        assert_eq!(plan.confirmations, 0);
    }

    #[test]
    fn skips_when_no_plans_offered() {
        let q = quote_result(100_000, &[]);
        assert!(matches!(
            select_instant_claim_plan(&q, 100_000, 10_000),
            InstantClaimPlan::NoPlan
        ));
    }

    #[test]
    fn gates_on_the_selected_plans_own_credit() {
        // The 0-conf plan's 5_000 spread (500 bps) breaches the 200 bps ceiling; the
        // cheaper 3-conf plan must not rescue it.
        let q = quote_result(100_000, &[(0, 95_000), (3, 99_000)]);
        assert!(matches!(
            select_instant_claim_plan(&q, 100_000, 200),
            InstantClaimPlan::FeeExceeded {
                quoted_sats: 5_000,
                quoted_bps: 500
            }
        ));
        // Conversely, an expensive deeper plan must not decline an affordable
        // shallow one.
        let q = quote_result(100_000, &[(0, 99_000), (3, 90_000)]);
        assert!(matches!(
            select_instant_claim_plan(&q, 100_000, 200),
            InstantClaimPlan::Claimable(_)
        ));
    }

    #[test]
    fn skips_when_spread_over_bps_ceiling() {
        // Spread 5_000 of 100_000 = 500 bps, ceiling 100 bps -> skip.
        let q = quote_result(100_000, &[(0, 95_000)]);
        assert!(matches!(
            select_instant_claim_plan(&q, 100_000, 100),
            InstantClaimPlan::FeeExceeded {
                quoted_sats: 5_000,
                quoted_bps: 500
            }
        ));
    }

    #[test]
    fn rejects_any_spread_at_zero_bps() {
        // A 0 bps ceiling admits only a zero spread.
        let q = quote_result(100_000, &[(0, 99_000)]);
        assert!(matches!(
            select_instant_claim_plan(&q, 100_000, 0),
            InstantClaimPlan::FeeExceeded { .. }
        ));
    }

    #[test]
    fn accepts_spread_equal_to_bps_ceiling() {
        // Spread 1_000 of 100_000 = 100 bps, ceiling exactly 100 bps -> claim (inclusive).
        let q = quote_result(100_000, &[(0, 99_000)]);
        assert!(matches!(
            select_instant_claim_plan(&q, 100_000, 100),
            InstantClaimPlan::Claimable(_)
        ));
    }

    #[test]
    fn one_bps_cap_admits_large_declines_small() {
        // The SSP spread is ~199 sats + 300 bps, so its effective bps falls as the
        // deposit grows. A single cap therefore admits a large deposit and declines
        // a small one (which should wait for the normal path). Values are measured.
        let cap_bps = 400;
        // 1_000 deposit: spread 229 -> 2290 bps -> declined.
        let small = quote_result(1_000, &[(0, 771)]);
        assert!(matches!(
            select_instant_claim_plan(&small, 1_000, cap_bps),
            InstantClaimPlan::FeeExceeded { .. }
        ));
        // 100_000 deposit: spread 3_199 -> 319 bps -> claimed.
        let large = quote_result(100_000, &[(0, 96_801)]);
        assert!(matches!(
            select_instant_claim_plan(&large, 100_000, cap_bps),
            InstantClaimPlan::Claimable(_)
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
        // (50_000). Spread is priced off the passed value: 50_000 - 49_500 = 500 =
        // 100 bps, within the 150 bps ceiling -> claim. Pricing off the quote's
        // 100_000 would give a 50_500 spread and decline, so a claim proves the
        // passed value drives the gate.
        let q = quote_result(100_000, &[(0, 49_500)]);
        assert!(matches!(
            select_instant_claim_plan(&q, 50_000, 150),
            InstantClaimPlan::Claimable(_)
        ));
    }
}
