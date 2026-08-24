use platform_utils::time::{Instant, SystemTime};
use platform_utils::tokio;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tracing::{debug, error, info, trace, warn};

use super::{
    BreezSdk, CLAIM_TX_SIZE_VBYTES, SYNC_PAGING_LIMIT, SyncType, deposits::InstantClaimOutcome,
    parse_input,
};
use crate::{
    DepositInfo, Fee, InputType, InstantClaimStatus, MaxFee, PaymentDetails, PaymentType,
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

/// Whether a matured deposit should be claimed now, given its instant-claim
/// status. A `Submitted` instant claim is still settling, so the deposit must not
/// be claimed until that claim settles and it is reconciled out; any other status
/// (declined, or no instant attempt) is fine to claim.
fn should_claim_matured_deposit(status: Option<&InstantClaimStatus>) -> bool {
    !matches!(status, Some(InstantClaimStatus::Submitted { .. }))
}

/// Whether the cascade should (re)attempt an early claim. A decline is retried
/// once the deposit gains a confirmation, or once the ceiling that declined it
/// rises. A submitted claim is never retried.
fn instant_claim_worth_attempting(
    status: Option<&InstantClaimStatus>,
    confirmations: u32,
    ceiling_sats: u64,
) -> bool {
    match status {
        None => true,
        Some(InstantClaimStatus::Declined {
            max_fee_sats,
            confirmations: declined_at,
        }) => confirmations > *declined_at || max_fee_sats.is_some_and(|prev| ceiling_sats > prev),
        Some(InstantClaimStatus::Submitted { .. }) => false,
    }
}

/// Indexes the deposits that carry an instant-claim status by their outpoint.
fn instant_claim_status_map(deposits: &[DepositInfo]) -> HashMap<TxOutput, InstantClaimStatus> {
    deposits
        .iter()
        .filter_map(|d| {
            d.instant_claim_status.clone().map(|status| {
                (
                    TxOutput {
                        txid: d.txid.clone(),
                        vout: d.vout,
                    },
                    status,
                )
            })
        })
        .collect()
}

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

    #[allow(clippy::too_many_lines)]
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

        // Read after the chain sync (a round-trip per UTXO): a manual instant claim
        // landing during the sync must be visible here, or the cascade could normal-
        // claim on top of the in-flight instant one.
        let instant_status = instant_claim_status_map(&self.storage.list_deposits().await?);

        // Resolved once per pass, and only when an immature deposit could use it.
        let instant_ceiling = if all_utxos.iter().any(|(_, is_mature)| !is_mature) {
            match self
                .resolve_max_claim_fee(self.config.max_deposit_claim_fee.clone())
                .await
            {
                Ok(resolved) => resolved,
                Err(e) => {
                    warn!("Could not resolve the max claim fee, skipping instant claims: {e}");
                    None
                }
            }
        } else {
            None
        };

        // The tip is the same for every deposit in the pass, so read it once. A
        // failure falls back to a per-deposit read.
        let tip_height = if instant_ceiling.is_some() {
            self.chain_service.tip_height().await.ok()
        } else {
            None
        };

        let mut claimed_deposits: Vec<DepositInfo> = Vec::new();
        let mut unclaimed_deposits: Vec<DepositInfo> = Vec::new();
        for (detailed_utxo, is_mature) in all_utxos {
            let key = TxOutput {
                txid: detailed_utxo.txid.to_string(),
                vout: detailed_utxo.vout,
            };
            let res = if is_mature {
                // A submitted instant claim settles asynchronously; until it settles
                // and the UTXO leaves the operator feed, the deposit can surface as
                // mature. Claiming it here would race that in-flight claim, so skip it
                // (reconcile_deposits drops the row once the claim settles).
                if !should_claim_matured_deposit(instant_status.get(&key)) {
                    continue;
                }
                // Mature deposit: claim via the normal path.
                self.claim_utxo_and_resolve_deposit(
                    &detailed_utxo,
                    self.config.max_deposit_claim_fee.clone(),
                    &mut claimed_deposits,
                    &mut unclaimed_deposits,
                )
                .await
            } else {
                // Not yet mature: attempt one early claim, held to the same
                // ceiling. Without a ceiling there is nothing to claim within.
                let Some(ceiling) = instant_ceiling.clone() else {
                    continue;
                };
                let Ok(confirmations) = self
                    .deposit_confirmations_at_tip(&detailed_utxo.txid.to_string(), tip_height)
                    .await
                else {
                    continue;
                };
                if !instant_claim_worth_attempting(
                    instant_status.get(&key),
                    confirmations,
                    ceiling.1,
                ) {
                    continue;
                }
                self.instant_claim_utxo_and_resolve_deposit(
                    &detailed_utxo,
                    Some(ceiling),
                    confirmations,
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
        resolved_max_fee: Option<(Fee, u64)>,
        confirmations: u32,
        claimed_deposits: &mut Vec<DepositInfo>,
    ) -> Result<(), SdkError> {
        let outcome = match self
            .instant_claim_utxo(detailed_utxo, resolved_max_fee, confirmations)
            .await
        {
            Ok(outcome) => outcome,
            Err(e) => {
                // Transient transport/indexing error (e.g. the SSP has not indexed
                // the mempool tx yet): do NOT mark, so the next sync retries.
                warn!(
                    "Instant claim transient error for utxo {}:{}, will retry: {e}",
                    detailed_utxo.txid, detailed_utxo.vout
                );
                return Ok(());
            }
        };

        // Mark, don't delete: reconcile_deposits removes the row once the UTXO
        // leaves the feed.
        let status = outcome.status(confirmations);
        self.storage
            .update_deposit(
                detailed_utxo.txid.to_string(),
                detailed_utxo.vout,
                UpdateDepositPayload::InstantClaim {
                    status: status.clone(),
                },
            )
            .await?;

        match outcome {
            InstantClaimOutcome::Submitted(claim_id) => {
                info!(
                    "Instant claimed utxo {}:{} with claim_id: {claim_id}",
                    detailed_utxo.txid, detailed_utxo.vout
                );
                let mut info = detailed_utxo.clone().into_deposit_info(false);
                info.instant_claim_status = Some(status);
                claimed_deposits.push(info);
            }
            InstantClaimOutcome::Declined { error, .. } => {
                // Declining an early claim leaves the deposit to claim at maturity,
                // which is the ordinary outcome rather than a failure. No claim_error
                // is persisted and no UnclaimedDeposits event fires, unlike a claim at
                // maturity that fails.
                info!(
                    "Instant claim declined for utxo {}:{}: {error}",
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
    pub(super) async fn resolve_max_claim_fee(
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
        // Awaited rather than left to the background collection, so a caller
        // that syncs before going offline knows the collection has run by the
        // time this returns. After the sync, so the leaves it brought in are
        // collected for too.
        if self.config.exit_chain_auto_fetch_enabled {
            self.runtime.collect_exit_chains(self).await?;
        }
        Ok(SyncWalletResponse {})
    }
}

#[cfg(test)]
mod tests {
    use super::{instant_claim_worth_attempting, should_claim_matured_deposit};
    use crate::InstantClaimStatus;

    fn declined(max_fee_sats: Option<u64>, confirmations: u32) -> InstantClaimStatus {
        InstantClaimStatus::Declined {
            max_fee_sats,
            confirmations,
        }
    }

    fn submitted() -> InstantClaimStatus {
        InstantClaimStatus::Submitted {
            claim_id: "claim-1".to_string(),
        }
    }

    #[test]
    fn claim_matured_deposit_skips_only_submitted() {
        // No instant attempt, or a declined one, falls through to the claim at
        // maturity.
        assert!(should_claim_matured_deposit(None));
        assert!(should_claim_matured_deposit(Some(&declined(None, 1))));
        assert!(should_claim_matured_deposit(Some(&declined(
            Some(1_000),
            1
        ))));
        // A submitted instant claim is in flight, so claiming the matured deposit
        // is skipped until the claim settles.
        assert!(!should_claim_matured_deposit(Some(&submitted())));
    }

    #[test]
    fn instant_retry_declines_once_per_confirmation() {
        // Never attempted -> attempt.
        assert!(instant_claim_worth_attempting(None, 1, 500));
        // A confirmation deeper than the decline: the provider lowers both its
        // spread and its floor with depth, so retry.
        assert!(instant_claim_worth_attempting(
            Some(&declined(Some(500), 1)),
            2,
            500
        ));
        assert!(instant_claim_worth_attempting(
            Some(&declined(None, 1)),
            2,
            500
        ));
        // Same depth, but the ceiling that declined it has been raised.
        assert!(instant_claim_worth_attempting(
            Some(&declined(Some(500), 1)),
            1,
            600
        ));
        // Same depth and the same ceiling: nothing has moved, so do not re-quote.
        assert!(!instant_claim_worth_attempting(
            Some(&declined(Some(500), 1)),
            1,
            500
        ));
        // A decline no ceiling caused does not re-quote on a ceiling change.
        assert!(!instant_claim_worth_attempting(
            Some(&declined(None, 1)),
            1,
            600
        ));
        // A shallower depth than the decline, as a reorg would give.
        assert!(!instant_claim_worth_attempting(
            Some(&declined(Some(500), 2)),
            1,
            500
        ));
        // An in-flight submission is never re-attempted.
        assert!(!instant_claim_worth_attempting(Some(&submitted()), 2, 500));
    }
}
