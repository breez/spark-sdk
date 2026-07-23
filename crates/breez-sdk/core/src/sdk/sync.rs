use platform_utils::time::{Instant, SystemTime};
use platform_utils::tokio;
use std::collections::HashSet;
use std::sync::Arc;
use tracing::{debug, error, info, trace, warn};

use spark_wallet::{InstantStaticDepositPlan, InstantStaticDepositQuoteResult};

use super::{BreezSdk, CLAIM_TX_SIZE_VBYTES, SYNC_PAGING_LIMIT, SyncType, parse_input};
use crate::{
    DepositInfo, Fee, InputType, MaxFee, PaymentDetails, PaymentType,
    error::SdkError,
    events::{InternalSyncedEvent, SdkEvent},
    lnurl::ListMetadataRequest,
    models::{Payment, SyncWalletRequest, SyncWalletResponse},
    persist::{ObjectCacheRepository, UpdateDepositPayload},
    sync::SparkSyncService,
    utils::{
        deposit_chain_syncer::{DepositChainSyncer, TxOutput},
        payments::update_balances,
        utxo_fetcher::DetailedUtxo,
    },
};

impl BreezSdk {
    pub(in crate::sdk) async fn sync_single_lnurl_metadata(&self, payment: &mut Payment) {
        if payment.payment_type != PaymentType::Receive {
            return;
        }

        let Some(PaymentDetails::Lightning {
            invoice,
            lnurl_receive_metadata,
            ..
        }) = &mut payment.details
        else {
            return;
        };

        if lnurl_receive_metadata.is_some() {
            // Already have lnurl metadata
            return;
        }

        let Ok(input) = parse_input(invoice, None).await else {
            error!(
                "Failed to parse invoice for lnurl metadata sync: {}",
                invoice
            );
            return;
        };

        let InputType::Bolt11Invoice(details) = input else {
            error!(
                "Input is not a Bolt11 invoice for lnurl metadata sync: {}",
                invoice
            );
            return;
        };

        // If there is a description hash, we assume this is a lnurl payment.
        if details.description_hash.is_none() {
            return;
        }

        // Let's check whether the lnurl receive metadata was already synced, then return early.
        // Important: Only return early if metadata is actually present (Some), otherwise we need
        // to trigger a sync. This prevents a race condition where the payment is in storage but
        // metadata sync from TransferClaimStarting hasn't completed yet.
        if let Ok(db_payment) = self.storage.get_payment_by_id(payment.id.clone()).await
            && let Some(PaymentDetails::Lightning {
                lnurl_receive_metadata: db_lnurl_receive_metadata @ Some(_),
                ..
            }) = db_payment.details
        {
            *lnurl_receive_metadata = db_lnurl_receive_metadata;
            return;
        }

        // Sync lnurl metadata directly instead of going through the sync trigger,
        // because this function is called from the sync loop's event handler,
        // which would deadlock waiting for itself to process the trigger.
        if let Err(e) = self.sync_lnurl_metadata().await {
            error!("Failed to sync lnurl metadata for invoice {invoice}: {e}");
            return;
        }

        let db_payment = match self.storage.get_payment_by_id(payment.id.clone()).await {
            Ok(p) => p,
            Err(e) => {
                debug!("Payment not found in storage for invoice {}: {e}", invoice);
                return;
            }
        };

        let Some(PaymentDetails::Lightning {
            lnurl_receive_metadata: db_lnurl_receive_metadata,
            ..
        }) = db_payment.details
        else {
            debug!(
                "No lnurl receive metadata in storage for invoice {}",
                invoice
            );
            return;
        };
        *lnurl_receive_metadata = db_lnurl_receive_metadata;
    }

    #[allow(clippy::too_many_lines)]
    pub(super) async fn sync_wallet_internal(
        &self,
        sync_type: SyncType,
        force: bool,
    ) -> Result<(), SdkError> {
        let cache = ObjectCacheRepository::new(self.storage.clone());
        let sync_interval_secs = u64::from(self.config.sync_interval_secs);

        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());

        // Skip if we synced recently (unless forced).
        if !force
            && let Some(last) = cache.get_last_sync_time().await?
            && now.saturating_sub(last) < sync_interval_secs
        {
            debug!("sync_wallet_internal: Synced recently, skipping");
            // When another instance shares our storage and keeps winning the sync
            // race, we would otherwise never emit a Synced event. Emit it here so
            // consumers are still notified that storage is up to date.
            self.event_emitter.emit(&SdkEvent::Synced).await;
            return Ok(());
        }

        // Update last sync time if this is a full sync.
        if sync_type.contains(SyncType::Full)
            && let Err(e) = cache.set_last_sync_time(now).await
        {
            error!("sync_wallet_internal: Failed to update last sync time: {e:?}");
        }

        let start_time = Instant::now();

        let sync_wallet = async {
            let wallet_synced = if sync_type.contains(SyncType::Wallet) {
                debug!("sync_wallet_internal: Starting Wallet sync");
                let wallet_start = Instant::now();
                match self.spark_wallet.sync().await {
                    Ok(()) => {
                        debug!(
                            "sync_wallet_internal: Wallet sync completed in {:?}",
                            wallet_start.elapsed()
                        );
                        true
                    }
                    Err(e) => {
                        error!(
                            "sync_wallet_internal: Spark wallet sync failed in {:?}: {e:?}",
                            wallet_start.elapsed()
                        );
                        false
                    }
                }
            } else {
                trace!("sync_wallet_internal: Skipping Wallet sync");
                false
            };

            let wallet_state_synced = if sync_type.contains(SyncType::WalletState) {
                debug!("sync_wallet_internal: Starting WalletState sync");
                let wallet_state_start = Instant::now();
                match self.sync_wallet_state_to_storage().await {
                    Ok(()) => {
                        debug!(
                            "sync_wallet_internal: WalletState sync completed in {:?}",
                            wallet_state_start.elapsed()
                        );
                        true
                    }
                    Err(e) => {
                        error!(
                            "sync_wallet_internal: Failed to sync wallet state to storage in {:?}: {e:?}",
                            wallet_state_start.elapsed()
                        );
                        false
                    }
                }
            } else {
                trace!("sync_wallet_internal: Skipping WalletState sync");
                false
            };

            (wallet_synced, wallet_state_synced)
        };

        let sync_lnurl = async {
            if sync_type.contains(SyncType::LnurlMetadata) {
                debug!("sync_wallet_internal: Starting LnurlMetadata sync");
                let lnurl_start = Instant::now();
                match self.sync_lnurl_metadata().await {
                    Ok(()) => {
                        debug!(
                            "sync_wallet_internal: LnurlMetadata sync completed in {:?}",
                            lnurl_start.elapsed()
                        );
                        true
                    }
                    Err(e) => {
                        error!(
                            "sync_wallet_internal: Failed to sync lnurl metadata in {:?}: {e:?}",
                            lnurl_start.elapsed()
                        );
                        false
                    }
                }
            } else {
                trace!("sync_wallet_internal: Skipping LnurlMetadata sync");
                false
            }
        };

        let sync_deposits = async {
            if sync_type.contains(SyncType::Deposits) {
                debug!("sync_wallet_internal: Starting Deposits sync");
                let deposits_start = Instant::now();
                match self.check_and_claim_static_deposits().await {
                    Ok(()) => {
                        debug!(
                            "sync_wallet_internal: Deposits sync completed in {:?}",
                            deposits_start.elapsed()
                        );
                        true
                    }
                    Err(e) => {
                        error!(
                            "sync_wallet_internal: Failed to check and claim static deposits in {:?}: {e:?}",
                            deposits_start.elapsed()
                        );
                        false
                    }
                }
            } else {
                trace!("sync_wallet_internal: Skipping Deposits sync");
                false
            }
        };

        let ((wallet, wallet_state), lnurl_metadata, deposits) =
            tokio::join!(sync_wallet, sync_lnurl, sync_deposits);

        let elapsed = start_time.elapsed();
        let event = InternalSyncedEvent {
            wallet,
            wallet_state,
            lnurl_metadata,
            deposits,
            storage_incoming: None,
        };
        info!("sync_wallet_internal: Wallet sync completed in {elapsed:?}: {event:?}");
        self.event_emitter.emit_synced(&event).await;
        Ok(())
    }

    /// Synchronizes wallet state to persistent storage, making sure we have the latest balances and payments.
    pub(super) async fn sync_wallet_state_to_storage(&self) -> Result<(), SdkError> {
        update_balances(self.spark_wallet.clone(), self.storage.clone()).await?;

        let initial_sync_complete = *self.initial_synced_watcher.borrow();
        let sync_service = SparkSyncService::new(
            self.spark_wallet.clone(),
            self.storage.clone(),
            self.event_emitter.clone(),
        );
        sync_service.sync_payments(initial_sync_complete).await?;

        Ok(())
    }

    pub(super) async fn check_and_claim_static_deposits(&self) -> Result<(), SdkError> {
        self.maybe_ensure_spark_private_mode_initialized().await?;
        let existing_deposits = self.storage.list_deposits().await?;
        let existing_keys: HashSet<TxOutput> = existing_deposits
            .iter()
            .map(|d| TxOutput {
                txid: d.txid.clone(),
                vout: d.vout,
            })
            .collect();
        // Deposits for which a 0-conf claim has already been attempted, so the
        // one-shot instant path is not retried every sync.
        let instant_attempted: HashSet<TxOutput> = existing_deposits
            .iter()
            .filter(|d| d.instant_claim_attempted)
            .map(|d| TxOutput {
                txid: d.txid.clone(),
                vout: d.vout,
            })
            .collect();

        let all_utxos = DepositChainSyncer::new(
            self.chain_service.clone(),
            self.storage.clone(),
            self.spark_wallet.clone(),
        )
        .sync()
        .await?;

        // Emit NewDeposits for any deposits not previously known
        let new_deposits: Vec<DepositInfo> = all_utxos
            .iter()
            .filter(|(u, _)| {
                !existing_keys.contains(&TxOutput {
                    txid: u.txid.to_string(),
                    vout: u.vout,
                })
            })
            .map(|(u, is_mature)| u.clone().into_deposit_info(*is_mature))
            .collect();
        if !new_deposits.is_empty() {
            self.event_emitter
                .emit(&SdkEvent::NewDeposits { new_deposits })
                .await;
        }

        let mut claimed_deposits: Vec<DepositInfo> = Vec::new();
        let mut unclaimed_deposits: Vec<DepositInfo> = Vec::new();
        for (detailed_utxo, is_mature) in all_utxos {
            let res = if is_mature {
                // Mature deposit: claim via the normal (legacy) path.
                self.claim_utxo_and_resolve_deposit(
                    &detailed_utxo,
                    self.config.max_deposit_claim_fee.clone(),
                    &mut claimed_deposits,
                    &mut unclaimed_deposits,
                )
                .await
            } else {
                // Not yet mature: attempt a one-shot 0-conf instant claim if enabled.
                // Skip if the instant claim was already attempted.
                let key = TxOutput {
                    txid: detailed_utxo.txid.to_string(),
                    vout: detailed_utxo.vout,
                };
                if instant_attempted.contains(&key) {
                    continue;
                }
                // Skip if instant claims are not enabled (no bps ceiling set).
                let Some(max_instant_fee_bps) = self.config.max_instant_deposit_claim_fee_bps
                else {
                    continue;
                };
                self.instant_claim_utxo_and_resolve_deposit(
                    &detailed_utxo,
                    max_instant_fee_bps,
                    &mut claimed_deposits,
                )
                .await
            };

            if let Err(e) = res {
                warn!(
                    "Failed to update deposit for utxo {}:{}: {e}",
                    detailed_utxo.txid, detailed_utxo.vout
                );
            }
        }

        info!("background claim completed, unclaimed deposits: {unclaimed_deposits:?}");

        if !unclaimed_deposits.is_empty() {
            self.event_emitter
                .emit(&SdkEvent::UnclaimedDeposits { unclaimed_deposits })
                .await;
        }
        if !claimed_deposits.is_empty() {
            self.event_emitter
                .emit(&SdkEvent::ClaimedDeposits { claimed_deposits })
                .await;
        }
        Ok(())
    }

    async fn claim_utxo_and_resolve_deposit(
        &self,
        detailed_utxo: &DetailedUtxo,
        max_claim_fee: Option<MaxFee>,
        claimed_deposits: &mut Vec<DepositInfo>,
        unclaimed_deposits: &mut Vec<DepositInfo>,
    ) -> Result<(), SdkError> {
        match self.claim_utxo(detailed_utxo, max_claim_fee).await {
            Ok(_) => {
                info!("Claimed utxo {}:{}", detailed_utxo.txid, detailed_utxo.vout);
                self.storage
                    .delete_deposit(detailed_utxo.txid.to_string(), detailed_utxo.vout)
                    .await?;
                claimed_deposits.push(detailed_utxo.clone().into_deposit_info(true));
            }
            Err(e) => {
                warn!(
                    "Failed to claim utxo {}:{}: {e}",
                    detailed_utxo.txid, detailed_utxo.vout
                );
                unclaimed_deposits.push(self.record_unclaimed_deposit(detailed_utxo, e).await?);
            }
        }
        Ok(())
    }

    async fn instant_claim_utxo_and_resolve_deposit(
        &self,
        detailed_utxo: &DetailedUtxo,
        max_instant_fee_bps: u32,
        claimed_deposits: &mut Vec<DepositInfo>,
    ) -> Result<(), SdkError> {
        match self
            .instant_claim_utxo(detailed_utxo, max_instant_fee_bps)
            .await
        {
            Ok(InstantClaimOutcome::Submitted(claim_id)) => {
                info!(
                    "Instant claimed utxo {}:{} with claim_id: {claim_id}",
                    detailed_utxo.txid, detailed_utxo.vout
                );
                // Mark, don't delete: the claim settles asynchronously, so the
                // marker keeps the next sync from re-attempting in the window
                // before the operator-side swap lands; reconcile_deposits removes
                // the row once the UTXO leaves the feed.
                self.storage
                    .update_deposit(
                        detailed_utxo.txid.to_string(),
                        detailed_utxo.vout,
                        UpdateDepositPayload::InstantClaimAttempted,
                    )
                    .await?;
                let mut info = detailed_utxo.clone().into_deposit_info(false);
                info.instant_claim_attempted = true;
                claimed_deposits.push(info);
            }
            Ok(InstantClaimOutcome::Declined(reason)) => {
                // Terminal decline (no 0-conf plan, or spread over the ceiling):
                // mark so we don't re-quote every sync; the deposit is claimed by
                // the legacy path at maturity.
                info!(
                    "Instant claim declined for utxo {}:{}: {reason}",
                    detailed_utxo.txid, detailed_utxo.vout
                );
                self.storage
                    .update_deposit(
                        detailed_utxo.txid.to_string(),
                        detailed_utxo.vout,
                        UpdateDepositPayload::InstantClaimAttempted,
                    )
                    .await?;
            }
            Err(e) => {
                // Transient transport/indexing error (e.g. the SSP has not indexed
                // the mempool tx yet): do NOT mark, so the next sync retries.
                warn!(
                    "Instant claim transient error for utxo {}:{}, will retry: {e}",
                    detailed_utxo.txid, detailed_utxo.vout
                );
            }
        }
        Ok(())
    }

    /// Persists a claim failure on the deposit and returns the matching
    /// `DepositInfo` (with `claim_error` set) for the `UnclaimedDeposits` event.
    async fn record_unclaimed_deposit(
        &self,
        utxo: &DetailedUtxo,
        error: SdkError,
    ) -> Result<DepositInfo, SdkError> {
        self.storage
            .update_deposit(
                utxo.txid.to_string(),
                utxo.vout,
                UpdateDepositPayload::ClaimError {
                    error: error.clone().into(),
                },
            )
            .await?;
        let mut info = utxo.clone().into_deposit_info(true);
        info.claim_error = Some(error.into());
        Ok(info)
    }

    pub(super) async fn sync_lnurl_metadata(&self) -> Result<(), SdkError> {
        let Some(lnurl_server_client) = self.lnurl_server_client.clone() else {
            return Ok(());
        };

        let cache = ObjectCacheRepository::new(Arc::clone(&self.storage));
        let mut updated_after = cache.fetch_lnurl_metadata_updated_after().await?;

        loop {
            debug!("Syncing lnurl metadata from updated_after {updated_after}");
            let metadata = lnurl_server_client
                .list_metadata(&ListMetadataRequest {
                    offset: None,
                    limit: Some(SYNC_PAGING_LIMIT),
                    updated_after: Some(updated_after),
                })
                .await?;

            if metadata.metadata.is_empty() {
                debug!("No more lnurl metadata on offset {updated_after}");
                break;
            }

            let len = u32::try_from(metadata.metadata.len())?;
            let last_updated_at = metadata.metadata.last().map(|m| m.updated_at);
            self.storage
                .set_lnurl_metadata(metadata.metadata.into_iter().map(From::from).collect())
                .await?;

            debug!(
                "Synchronized {} lnurl metadata at updated_after {updated_after}",
                len
            );
            updated_after = last_updated_at.unwrap_or(updated_after);
            cache
                .save_lnurl_metadata_updated_after(updated_after)
                .await?;

            if len < SYNC_PAGING_LIMIT {
                // No more invoices to fetch
                break;
            }
        }

        Ok(())
    }

    /// Resolves an on-chain max claim fee to `(fee, ceiling_sats)`, where the sat
    /// ceiling is computed over the claim tx size. `None` means no ceiling is set,
    /// which the caller treats as rejecting the claim.
    async fn resolve_max_claim_fee(
        &self,
        max_claim_fee: Option<MaxFee>,
    ) -> Result<Option<(Fee, u64)>, SdkError> {
        match max_claim_fee {
            None => Ok(None),
            Some(max_fee) => {
                let fee = max_fee.to_fee(self.chain_service.as_ref()).await?;
                let sats = fee.to_sats(CLAIM_TX_SIZE_VBYTES);
                Ok(Some((fee, sats)))
            }
        }
    }

    /// Submits a static deposit claim for `detailed_utxo` and returns the
    /// resulting transfer id.
    pub(super) async fn claim_utxo(
        &self,
        detailed_utxo: &DetailedUtxo,
        max_claim_fee: Option<MaxFee>,
    ) -> Result<String, SdkError> {
        info!(
            "Fetching static deposit claim quote for deposit tx {}:{} and amount: {}",
            detailed_utxo.txid, detailed_utxo.vout, detailed_utxo.value
        );
        let quote = self
            .spark_wallet
            .fetch_static_deposit_claim_quote(detailed_utxo.tx.clone(), Some(detailed_utxo.vout))
            .await?;

        let spark_requested_fee_sats = detailed_utxo.value.saturating_sub(quote.credit_amount_sats);

        let spark_requested_fee_rate = spark_requested_fee_sats.div_ceil(CLAIM_TX_SIZE_VBYTES);

        let resolved_max_fee = self.resolve_max_claim_fee(max_claim_fee).await?;
        if let Some((_, max_fee_sats)) = &resolved_max_fee {
            info!("User max fee: {max_fee_sats} spark requested fee: {spark_requested_fee_sats}");
        }
        let within_limit = resolved_max_fee
            .as_ref()
            .is_some_and(|(_, max_fee_sats)| spark_requested_fee_sats <= *max_fee_sats);
        if !within_limit {
            return Err(SdkError::MaxDepositClaimFeeExceeded {
                tx: detailed_utxo.txid.to_string(),
                vout: detailed_utxo.vout,
                max_fee: resolved_max_fee.map(|(fee, _)| fee),
                required_fee_sats: spark_requested_fee_sats,
                required_fee_rate_sat_per_vbyte: spark_requested_fee_rate,
            });
        }

        info!(
            "Claiming static deposit for utxo {}:{}",
            detailed_utxo.txid, detailed_utxo.vout
        );
        let credit_amount_sats = quote.credit_amount_sats;
        let transfer_id = self.spark_wallet.claim_static_deposit(quote).await?;
        info!(
            "Claimed static deposit for utxo {}:{} (deposit value {}, credit {}), transfer {transfer_id}",
            detailed_utxo.txid, detailed_utxo.vout, detailed_utxo.value, credit_amount_sats,
        );
        Ok(transfer_id)
    }

    /// Attempts an instant 0-conf static deposit claim for `detailed_utxo`.
    /// `Ok(Submitted)` on a submitted claim, `Ok(Declined)` for a terminal
    /// outcome (no 0-conf plan, spread over the ceiling, or a failed claim
    /// submission), and `Err` only for a failed quote fetch, which is transient
    /// (the SSP may not have indexed the mempool tx yet) and should be retried.
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
        match select_instant_claim_plan(&quote_result, max_instant_fee_bps) {
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
                    // A claim submission failure has an unknown outcome (the SSP
                    // may have accepted it before the response was lost), so treat
                    // it as terminal and mark rather than re-submit into the
                    // pre-swap window on the next sync.
                    Err(e) => Ok(InstantClaimOutcome::Declined(e.into())),
                }
            }
            InstantClaimPlan::NoPlan => Ok(InstantClaimOutcome::Declined(SdkError::Generic(
                "No instant (0-conf) claim plan available".to_string(),
            ))),
            InstantClaimPlan::FeeExceeded {
                spread_sats,
                spread_bps,
            } => Ok(InstantClaimOutcome::Declined(SdkError::Generic(format!(
                "Instant claim declined for {}:{}: SSP spread {spread_bps} bps ({spread_sats} sats) exceeds max {max_instant_fee_bps} bps",
                detailed_utxo.txid, detailed_utxo.vout
            )))),
        }
    }
}

/// Result of an instant (0-conf) claim attempt.
pub(super) enum InstantClaimOutcome {
    /// Claim submitted; carries the claim id. Settles asynchronously.
    Submitted(String),
    /// A terminal outcome to mark rather than retry: no 0-conf plan, spread over
    /// the ceiling, or a failed claim submission (whose outcome is unknown, so
    /// re-submitting is unsafe). Distinct from a failed quote fetch, which retries.
    Declined(SdkError),
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
#[allow(clippy::needless_pass_by_value)]
impl BreezSdk {
    /// Synchronizes the wallet with the Spark network
    #[allow(unused_variables)]
    pub async fn sync_wallet(
        &self,
        request: SyncWalletRequest,
    ) -> Result<SyncWalletResponse, SdkError> {
        self.runtime
            .run_user_sync(self, super::SyncType::Full, true)
            .await?;
        Ok(SyncWalletResponse {})
    }
}

/// Classification of an instant quote's 0-conf plan against the bps ceiling.
enum InstantClaimPlan {
    /// The 0-conf plan is within the ceiling and should be claimed.
    Claimable(InstantStaticDepositPlan),
    /// No `confirmations == 0` fulfillment plan was offered.
    NoPlan,
    /// The SSP spread (`deposit - credit`) exceeds the ceiling, in both sats and
    /// its basis-points-of-deposit form (for the decline message).
    FeeExceeded { spread_sats: u64, spread_bps: u64 },
}

/// Selects the 0-conf fulfillment plan and checks the SSP spread
/// (`deposit - credit`) against `max_bps`, as basis points of the deposit value.
/// The spread carries a fixed component, so its bps grows as the deposit shrinks:
/// a single bps ceiling therefore admits large deposits and declines small ones.
fn select_instant_claim_plan(
    quote_result: &InstantStaticDepositQuoteResult,
    max_bps: u32,
) -> InstantClaimPlan {
    let Some(plan) = quote_result
        .fulfillment_plans
        .iter()
        .find(|p| p.confirmations == 0)
    else {
        return InstantClaimPlan::NoPlan;
    };
    // Price the spread off the selected plan's `amount` (the credit for the plan
    // we chose), not the quote-level credit. They are equal today (the SSP
    // returns one plan per quote, verified on deployed regtest), but keying on the
    // plan stays correct if the SSP ever returns multiple plans with differing
    // amounts.
    let deposit_sats = quote_result.quote.deposit_amount.original_value;
    let spread_sats = deposit_sats.saturating_sub(plan.amount.original_value);
    // Compare spread_bps <= max_bps without dividing (avoids rounding at the
    // bound); saturating math keeps `spread * 10_000` from overflowing.
    let within = u128::from(spread_sats).saturating_mul(10_000)
        <= u128::from(max_bps).saturating_mul(u128::from(deposit_sats));
    if within {
        InstantClaimPlan::Claimable(plan.clone())
    } else {
        // checked_div yields None for a zero-value deposit, floored to 0 bps.
        let spread_bps = u128::from(spread_sats)
            .saturating_mul(10_000)
            .checked_div(u128::from(deposit_sats))
            .and_then(|bps| u64::try_from(bps).ok())
            .unwrap_or(0);
        InstantClaimPlan::FeeExceeded {
            spread_sats,
            spread_bps,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{InstantClaimPlan, select_instant_claim_plan};
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

    /// Builds a quote crediting `credit_sats` out of a `deposit_sats` UTXO, with
    /// one fulfillment plan per confirmation count in `plan_confirmations`.
    fn quote_result(
        deposit_sats: u64,
        credit_sats: u64,
        plan_confirmations: &[i64],
    ) -> InstantStaticDepositQuoteResult {
        InstantStaticDepositQuoteResult {
            quote: InstantStaticDepositQuote {
                id: "quote-id".to_string(),
                transaction_id: "tx".to_string(),
                output_index: 0,
                deposit_amount: sats(deposit_sats),
                credit_amount: sats(credit_sats),
                quote_signature: "00".to_string(),
            },
            fulfillment_plans: plan_confirmations
                .iter()
                .enumerate()
                .map(|(i, confirmations)| InstantStaticDepositPlan {
                    id: format!("plan-{i}"),
                    amount: sats(credit_sats),
                    confirmations: *confirmations,
                })
                .collect(),
        }
    }

    #[test]
    fn selects_zero_conf_plan_within_bps() {
        // Spread 1_000 of 100_000 = 100 bps, ceiling 200 bps -> claim.
        let q = quote_result(100_000, 99_000, &[0, 1]);
        let InstantClaimPlan::Claimable(plan) = select_instant_claim_plan(&q, 200) else {
            panic!("expected a claimable 0-conf plan");
        };
        assert_eq!(plan.confirmations, 0);
    }

    #[test]
    fn skips_when_no_zero_conf_plan() {
        // Only 1-conf+ plans available -> the background cascade waits for maturity.
        let q = quote_result(100_000, 99_000, &[1, 2]);
        assert!(matches!(
            select_instant_claim_plan(&q, 10_000),
            InstantClaimPlan::NoPlan
        ));
    }

    #[test]
    fn skips_when_spread_over_bps_ceiling() {
        // Spread 5_000 of 100_000 = 500 bps, ceiling 100 bps -> skip.
        let q = quote_result(100_000, 95_000, &[0]);
        assert!(matches!(
            select_instant_claim_plan(&q, 100),
            InstantClaimPlan::FeeExceeded {
                spread_sats: 5_000,
                spread_bps: 500
            }
        ));
    }

    #[test]
    fn rejects_any_spread_at_zero_bps() {
        // A 0 bps ceiling admits only a zero spread.
        let q = quote_result(100_000, 99_000, &[0]);
        assert!(matches!(
            select_instant_claim_plan(&q, 0),
            InstantClaimPlan::FeeExceeded { .. }
        ));
    }

    #[test]
    fn accepts_spread_equal_to_bps_ceiling() {
        // Spread 1_000 of 100_000 = 100 bps, ceiling exactly 100 bps -> claim (inclusive).
        let q = quote_result(100_000, 99_000, &[0]);
        assert!(matches!(
            select_instant_claim_plan(&q, 100),
            InstantClaimPlan::Claimable(_)
        ));
    }

    #[test]
    fn one_bps_cap_admits_large_declines_small() {
        // The SSP spread is ~199 sats + 300 bps, so its effective bps falls as the
        // deposit grows. A single cap therefore admits a large deposit and declines
        // a small one (which should wait for the legacy path). Values are measured.
        let cap_bps = 400;
        // 1_000 deposit: spread 229 -> 2290 bps -> declined.
        let small = quote_result(1_000, 771, &[0]);
        assert!(matches!(
            select_instant_claim_plan(&small, cap_bps),
            InstantClaimPlan::FeeExceeded { .. }
        ));
        // 100_000 deposit: spread 3_199 -> 319 bps -> claimed.
        let large = quote_result(100_000, 96_801, &[0]);
        assert!(matches!(
            select_instant_claim_plan(&large, cap_bps),
            InstantClaimPlan::Claimable(_)
        ));
    }
}
