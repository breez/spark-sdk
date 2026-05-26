use platform_utils::time::{Instant, SystemTime};
use platform_utils::tokio;
use std::sync::Arc;
use tracing::{debug, error, info, trace, warn};

use super::{BreezSdk, CLAIM_TX_SIZE_VBYTES, SYNC_PAGING_LIMIT, SyncType, parse_input};
use crate::{
    DepositInfo, InputType, MaxFee, PaymentDetails, PaymentType,
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
        let existing_keys: std::collections::HashSet<TxOutput> = existing_deposits
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

        // Only claim UTXOs with sufficient confirmations
        let to_claim: Vec<_> = all_utxos
            .into_iter()
            .filter(|(_, is_mature)| *is_mature)
            .map(|(u, _)| u)
            .collect();

        let mut claimed_deposits: Vec<DepositInfo> = Vec::new();
        let mut unclaimed_deposits: Vec<DepositInfo> = Vec::new();
        for detailed_utxo in to_claim {
            match self
                .claim_utxo(&detailed_utxo, self.config.max_deposit_claim_fee.clone())
                .await
            {
                Ok(_) => {
                    info!("Claimed utxo {}:{}", detailed_utxo.txid, detailed_utxo.vout);
                    self.storage
                        .delete_deposit(detailed_utxo.txid.to_string(), detailed_utxo.vout)
                        .await?;
                    claimed_deposits.push(detailed_utxo.into_deposit_info(true));
                }
                Err(e) => {
                    warn!(
                        "Failed to claim utxo {}:{}: {e}",
                        detailed_utxo.txid, detailed_utxo.vout
                    );
                    self.storage
                        .update_deposit(
                            detailed_utxo.txid.to_string(),
                            detailed_utxo.vout,
                            UpdateDepositPayload::ClaimError {
                                error: e.clone().into(),
                            },
                        )
                        .await?;
                    let mut unclaimed_deposit = detailed_utxo.into_deposit_info(true);
                    unclaimed_deposit.claim_error = Some(e.into());
                    unclaimed_deposits.push(unclaimed_deposit);
                }
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

        let Some(max_deposit_claim_fee) = max_claim_fee else {
            return Err(SdkError::MaxDepositClaimFeeExceeded {
                tx: detailed_utxo.txid.to_string(),
                vout: detailed_utxo.vout,
                max_fee: None,
                required_fee_sats: spark_requested_fee_sats,
                required_fee_rate_sat_per_vbyte: spark_requested_fee_rate,
            });
        };
        let max_fee = max_deposit_claim_fee
            .to_fee(self.chain_service.as_ref())
            .await?;
        let max_fee_sats = max_fee.to_sats(CLAIM_TX_SIZE_VBYTES);
        info!(
            "User max fee: {} spark requested fee: {}",
            max_fee_sats, spark_requested_fee_sats
        );
        if spark_requested_fee_sats > max_fee_sats {
            return Err(SdkError::MaxDepositClaimFeeExceeded {
                tx: detailed_utxo.txid.to_string(),
                vout: detailed_utxo.vout,
                max_fee: Some(max_fee),
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
        Ok(SyncWalletResponse {})
    }
}
