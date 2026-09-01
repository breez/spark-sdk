use std::{
    collections::HashSet,
    str::FromStr,
    sync::{Arc, Mutex},
    time::Duration,
};

use bitcoin::{
    Transaction,
    consensus::{encode::deserialize_hex, serialize},
    hex::DisplayHex,
};
use platform_utils::tokio;
use spark_wallet::{
    InstantStaticDepositPlan, InstantStaticDepositQuoteResult, ListTransfersRequest,
    MIN_RELAY_FEE_SAT_PER_VBYTE, TransferId, WalletTransfer,
};
use tracing::{error, info, trace, warn};

use crate::{
    ClaimDepositQuote, ClaimDepositRequest, ClaimDepositResponse, DepositInfo, Fee,
    FetchClaimDepositQuoteRequest, FetchClaimDepositQuoteResponse, InstantClaimStatus,
    ListUnclaimedDepositsRequest, ListUnclaimedDepositsResponse, MaxFee, Network,
    RefundDepositRequest, RefundDepositResponse, RefundState,
    chain::Outspend,
    error::SdkError,
    models::Payment,
    persist::UpdateDepositPayload,
    sdk::RuntimeEvent,
    utils::deposit_chain_syncer::TxOutput,
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

        // Held for the whole attempt, so a sync pass or a second call on the same
        // outpoint cannot run one alongside it.
        let Some(_claim_guard) = self.claim_guards.try_acquire(TxOutput {
            txid: request.txid.clone(),
            vout: request.vout,
        }) else {
            return Err(SdkError::DepositClaimInProgress {
                tx: request.txid.clone(),
                vout: request.vout,
            });
        };

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
    ///
    /// The early quote is requested from the provider on each call rather than read
    /// from cache, so call this when a user is deciding, not on a timer.
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

        let existing = self
            .storage
            .list_deposits()
            .await?
            .into_iter()
            .find(|d| d.txid == detailed_utxo.txid.to_string() && d.vout == detailed_utxo.vout);
        // Nothing stored means no refund to outbid.
        let fee_to_outbid = match &existing {
            Some(deposit) => self.refund_fee_to_outbid(&detailed_utxo, deposit).await?,
            None => None,
        };

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

        check_replacement_fee(&tx, detailed_utxo.value, fee_to_outbid)?;

        // `store_refund` only ever updates, so the row has to exist: without one the
        // refund would be broadcast, persisted nowhere and never rebroadcast. A
        // refund can be asked for before the background sync has inserted the row.
        // Insert it only once the operators have signed, so a txid and vout they do
        // not recognise leaves nothing behind.
        if existing.is_none() {
            // Mature: the operators only sign a refund once the deposit has enough
            // confirmations.
            self.storage
                .add_deposit(
                    detailed_utxo.txid.to_string(),
                    detailed_utxo.vout,
                    detailed_utxo.value,
                    true,
                )
                .await?;
        }

        // Store before broadcasting: a signed refund that is only in flight is
        // lost if the process dies, and the rebroadcast on sync needs it.
        self.store_refund(
            &detailed_utxo,
            &tx_hex,
            &tx_id,
            RefundState::BroadcastPending { last_error: None },
        )
        .await?;

        let broadcast_error = self
            .chain_service
            .broadcast_transaction(tx_hex.clone())
            .await
            .err();
        // Record why the broadcast was refused before returning, so the deposit
        // carries the reason rather than waiting for the next sync to retry.
        let state = match &broadcast_error {
            None => RefundState::Broadcast,
            Some(e) => RefundState::BroadcastPending {
                last_error: Some(e.to_string()),
            },
        };

        if let Err(e) = self
            .store_refund(&detailed_utxo, &tx_hex, &tx_id, state)
            .await
        {
            error!("Failed to record refund state: {e:?}");
        }

        if let Some(e) = broadcast_error {
            return Err(e.into());
        }
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

    async fn store_refund(
        &self,
        detailed_utxo: &DetailedUtxo,
        tx_hex: &str,
        tx_id: &str,
        state: RefundState,
    ) -> Result<(), SdkError> {
        self.storage
            .update_deposit(
                detailed_utxo.txid.to_string(),
                detailed_utxo.vout,
                UpdateDepositPayload::Refund {
                    refund_tx: tx_hex.to_string(),
                    refund_txid: tx_id.to_string(),
                    state,
                },
            )
            .await?;
        Ok(())
    }

    /// Fee in sats that a new refund has to outbid, or `None` when there is
    /// nothing to outbid. `None` while the deposit output is unspent: a refund
    /// that never reached the network conflicts with nothing, so re-creating it
    /// at a lower fee has to stay possible.
    async fn refund_fee_to_outbid(
        &self,
        detailed_utxo: &DetailedUtxo,
        deposit: &DepositInfo,
    ) -> Result<Option<PendingRefund>, SdkError> {
        let Some(refund_tx) = deposit.refund_tx.as_ref() else {
            return Ok(None);
        };
        // A refund that cannot be decoded is no basis for holding a new one back.
        let Ok(tx) = deserialize_hex::<Transaction>(refund_tx) else {
            warn!(
                "Stored refund of deposit {}:{} does not decode, not requiring a fee bump",
                detailed_utxo.txid, detailed_utxo.vout
            );
            return Ok(None);
        };
        let stored = refund_fee_sats(&tx, detailed_utxo.value).map(|fee_sats| PendingRefund {
            fee_sats,
            vsize: tx.vsize().try_into().unwrap_or(u64::MAX),
        });

        let outspend = self
            .chain_service
            .get_outspend(detailed_utxo.txid.to_string(), detailed_utxo.vout)
            .await;
        match outspend {
            Ok(Outspend::Unspent) => Ok(None),
            Ok(Outspend::Spent { txid, status, .. }) if status.confirmed => {
                Err(SdkError::InvalidInput(format!(
                    "Deposit {}:{} was already spent by {txid}",
                    detailed_utxo.txid, detailed_utxo.vout
                )))
            }
            // A conflicting transaction is on the network. The floor is the stored
            // refund's fee, which is the spender whenever this wallet made it. If
            // something else did, an underpriced replacement is refused by the
            // network rather than by this check.
            Ok(Outspend::Spent { .. }) => Ok(stored),
            // The outpoint cannot be read, so ask whether the stored refund itself
            // reached the network. That is authoritative, unlike the recorded state,
            // which a lost write can leave stale. Gating on a refund that never
            // landed would raise the bar on every retry, since each attempt stores
            // its own fee before broadcasting.
            Err(_) => {
                let Some(refund_txid) = deposit.refund_tx_id.clone() else {
                    return Ok(None);
                };
                match self.chain_service.get_transaction_status(refund_txid).await {
                    Ok(status) if status.confirmed => Err(SdkError::InvalidInput(format!(
                        "Deposit {}:{} was already refunded",
                        detailed_utxo.txid, detailed_utxo.vout
                    ))),
                    Ok(_) => Ok(stored),
                    Err(_) => Ok(None),
                }
            }
        }
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

/// Selects the shallowest of the fulfillment plans the SSP returned with the
/// quote (the one crediting at the fewest confirmations) and checks the spread
/// (`deposit - credit`) against `max_fee_sats`, the same resolved ceiling the
/// claim at maturity is held to. An unset ceiling arrives here as zero, which
/// admits nothing.
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

/// The refund a replacement has to displace. Its size matters as well as its
/// fee: the replacement has to beat its feerate, not just its total.
#[derive(Clone, Copy)]
struct PendingRefund {
    fee_sats: u64,
    vsize: u64,
}

/// Rejects a refund that cannot displace one already on the network. A
/// replacement only relays if it outbids the refund it conflicts with, and
/// overwriting the stored transaction with one that cannot relay would leave the
/// deposit with no way out.
fn check_replacement_fee(
    tx: &Transaction,
    deposit_value_sats: u64,
    pending: Option<PendingRefund>,
) -> Result<(), SdkError> {
    let Some(pending) = pending else {
        return Ok(());
    };
    let pending_fee_sats = pending.fee_sats;
    let required_fee_sats =
        replacement_min_fee_sats(&pending, tx.vsize().try_into().unwrap_or(u64::MAX));
    let fee_sats = refund_fee_sats(tx, deposit_value_sats).ok_or_else(|| {
        SdkError::Generic("refund pays out more than the deposit holds".to_string())
    })?;
    if fee_sats < required_fee_sats {
        return Err(SdkError::RefundReplacementFeeTooLow {
            pending_fee_sats,
            required_fee_sats,
        });
    }
    Ok(())
}

/// Fee a refund pays, from the deposit output it spends. `None` if the refund
/// pays out more than the deposit holds.
fn refund_fee_sats(refund_tx: &Transaction, deposit_value_sats: u64) -> Option<u64> {
    let out_sats: u64 = refund_tx.output.iter().map(|o| o.value.to_sat()).sum();
    deposit_value_sats.checked_sub(out_sats)
}

/// Minimum fee a replacement must pay to displace a refund already on the
/// network: more than that refund pays, plus the relay cost of its own size.
fn replacement_min_fee_sats(pending: &PendingRefund, replacement_vsize: u64) -> u64 {
    // Cover the pending fee plus the replacement's own relay bandwidth.
    let bandwidth = pending
        .fee_sats
        .saturating_add(replacement_vsize.saturating_mul(MIN_RELAY_FEE_SAT_PER_VBYTE));
    // And beat its feerate outright, which only bites when the replacement is the
    // larger transaction, as it is when the destination widens to taproot. The
    // comparison is made on feerates truncated to whole sat/kvB, so a fee that is
    // higher as an exact rational can still land in the same bucket and be refused.
    let pending_per_kvb = pending
        .fee_sats
        .saturating_mul(1000)
        .checked_div(pending.vsize)
        .unwrap_or(u64::MAX);
    let feerate = pending_per_kvb
        .saturating_add(1)
        .saturating_mul(replacement_vsize)
        .div_ceil(1000);
    bandwidth.max(feerate)
}

/// Serialises claim attempts on the same deposit within this process.
///
/// A claim spends several seconds fetching a quote, transferring and signing
/// before anything reaches storage, so a second attempt starting in that window
/// finds no trace of the first. Holding the outpoint for the whole attempt closes
/// that, which persisted state cannot: it is only written once the claim returns.
#[derive(Clone, Default)]
pub(crate) struct ClaimGuards {
    in_flight: Arc<Mutex<HashSet<TxOutput>>>,
}

impl ClaimGuards {
    /// `None` when an attempt on this outpoint is already running.
    pub(crate) fn try_acquire(&self, outpoint: TxOutput) -> Option<ClaimGuard> {
        let mut in_flight = self
            .in_flight
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !in_flight.insert(outpoint.clone()) {
            return None;
        }
        Some(ClaimGuard {
            guards: self.clone(),
            outpoint,
        })
    }

    fn release(&self, outpoint: &TxOutput) {
        self.in_flight
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(outpoint);
    }
}

/// Releases the outpoint when dropped, so a claim that fails or panics does not
/// leave it locked out.
pub(crate) struct ClaimGuard {
    guards: ClaimGuards,
    outpoint: TxOutput,
}

impl Drop for ClaimGuard {
    fn drop(&mut self) {
        self.guards.release(&self.outpoint);
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::{
        Amount, OutPoint, ScriptBuf, Transaction, TxIn, TxOut, absolute::LockTime,
        transaction::Version,
    };
    use spark_wallet::{
        CurrencyAmount, InstantStaticDepositPlan, InstantStaticDepositQuote,
        InstantStaticDepositQuoteResult,
    };

    use super::{
        ClaimGuards, InstantClaimPlan, PendingRefund, SdkError, TxOutput, check_replacement_fee,
        claim_deposit_quote, is_pending_confirmation_error, refund_fee_sats,
        replacement_min_fee_sats, select_instant_claim_plan,
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

    fn outpoint(vout: u32) -> TxOutput {
        TxOutput {
            txid: "tx".to_string(),
            vout,
        }
    }

    fn refund_paying_out(out_sats: u64) -> Transaction {
        Transaction {
            version: Version::non_standard(3),
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::null(),
                ..Default::default()
            }],
            output: vec![TxOut {
                value: Amount::from_sat(out_sats),
                script_pubkey: ScriptBuf::new(),
            }],
        }
    }

    #[test]
    fn a_second_attempt_on_the_same_outpoint_is_refused() {
        let guards = ClaimGuards::default();
        let first = guards.try_acquire(outpoint(0));
        assert!(first.is_some());
        assert!(guards.try_acquire(outpoint(0)).is_none());
        drop(first);
        assert!(guards.try_acquire(outpoint(0)).is_some());
    }

    #[test]
    fn different_outpoints_do_not_block_each_other() {
        let guards = ClaimGuards::default();
        let _first = guards.try_acquire(outpoint(0));
        assert!(guards.try_acquire(outpoint(1)).is_some());
    }

    #[test]
    fn fee_is_what_the_refund_leaves_behind() {
        assert_eq!(
            refund_fee_sats(&refund_paying_out(99_889), 100_000),
            Some(111)
        );
        assert_eq!(
            refund_fee_sats(&refund_paying_out(100_000), 100_000),
            Some(0)
        );
        // A refund cannot pay out more than the deposit holds.
        assert_eq!(refund_fee_sats(&refund_paying_out(100_001), 100_000), None);
    }

    #[test]
    fn replacement_must_cover_the_pending_fee_and_its_own_relay() {
        // Displacing a 111 sat refund with a 111 vbyte replacement costs 222,
        // not 112: the replacement also pays to relay its own bytes.
        let pending = PendingRefund {
            fee_sats: 111,
            vsize: 111,
        };
        assert_eq!(replacement_min_fee_sats(&pending, 111), 222);
        let free = PendingRefund {
            fee_sats: 0,
            vsize: 111,
        };
        assert_eq!(replacement_min_fee_sats(&free, 111), 111);
    }

    #[test]
    fn a_larger_replacement_must_beat_the_pending_feerate_too() {
        // Covering the pending fee plus the replacement's own bandwidth is not
        // enough when the replacement is bigger: 3000 sats over 99 vB is 30.3
        // sat/vB, and 3111 over 111 vB would be 28.0, which the network refuses.
        let pending = PendingRefund {
            fee_sats: 3_000,
            vsize: 99,
        };
        let required = replacement_min_fee_sats(&pending, 111);
        assert_eq!(required, 3_364);
        assert!(
            required * pending.vsize > pending.fee_sats * 111,
            "a replacement has to beat the pending feerate outright"
        );

        // Same size, so paying the bandwidth is all it takes.
        assert_eq!(replacement_min_fee_sats(&pending, 99), 3_099);

        // Truncation to whole sat/kvB: 1045 over 111 vB and 932 over 99 both come
        // to 9414, which is a tie and refused, so the floor has to be 1046.
        let tie = PendingRefund {
            fee_sats: 932,
            vsize: 99,
        };
        assert_eq!(replacement_min_fee_sats(&tie, 111), 1_046);

        // A small pending fee never reaches the feerate rule.
        let small = PendingRefund {
            fee_sats: 300,
            vsize: 99,
        };
        assert_eq!(replacement_min_fee_sats(&small, 111), 411);
    }

    #[test]
    fn replacement_is_rejected_until_it_outbids_the_pending_refund() {
        let deposit = 100_000u64;
        let pending = 500u64;
        let vsize = refund_paying_out(0).vsize() as u64;
        let required = replacement_min_fee_sats(
            &PendingRefund {
                fee_sats: pending,
                vsize,
            },
            vsize,
        );

        // Nothing on the network to outbid, so any fee is fine.
        assert!(check_replacement_fee(&refund_paying_out(deposit - 1), deposit, None).is_ok());

        let pending_refund = PendingRefund {
            fee_sats: pending,
            vsize,
        };

        // A single sat short of the floor is not enough, even though it pays
        // more than the refund it is trying to displace.
        let short = refund_paying_out(deposit - required + 1);
        assert!(refund_fee_sats(&short, deposit).unwrap() > pending);
        assert!(matches!(
            check_replacement_fee(&short, deposit, Some(pending_refund)),
            Err(SdkError::RefundReplacementFeeTooLow {
                pending_fee_sats,
                required_fee_sats,
            }) if pending_fee_sats == pending && required_fee_sats == required
        ));

        // Paying exactly the floor is accepted.
        let exact = refund_paying_out(deposit - required);
        assert!(check_replacement_fee(&exact, deposit, Some(pending_refund)).is_ok());
    }
}
