use std::sync::Arc;

use spark_wallet::{
    ListTokenTransactionsRequest, ListTransfersRequest, Order, PagingFilter, SparkWallet,
};
use tracing::{error, info};

use crate::{
    EventEmitter, Payment, PaymentDetails, PaymentStatus, SdkError, SdkEvent, Storage,
    persist::{CachedSyncInfo, ObjectCacheRepository},
    utils::token::token_transaction_to_payments,
};

const PAYMENT_SYNC_BATCH_SIZE: u64 = 50;

pub(crate) struct SparkSyncService {
    spark_wallet: Arc<SparkWallet>,
    storage: Arc<dyn Storage>,
    event_emitter: Arc<EventEmitter>,
}

impl SparkSyncService {
    pub fn new(
        spark_wallet: Arc<SparkWallet>,
        storage: Arc<dyn Storage>,
        event_emitter: Arc<EventEmitter>,
    ) -> Self {
        Self {
            spark_wallet,
            storage,
            event_emitter,
        }
    }

    pub async fn sync_payments(&self, initial_sync_complete: bool) -> Result<(), SdkError> {
        let object_repository = ObjectCacheRepository::new(self.storage.clone());
        self.sync_bitcoin_payments_to_storage(&object_repository, initial_sync_complete)
            .await?;
        self.sync_token_payments_to_storage(&object_repository, initial_sync_complete)
            .await?;
        Ok(())
    }

    async fn sync_bitcoin_payments_to_storage(
        &self,
        object_repository: &ObjectCacheRepository,
        initial_sync_complete: bool,
    ) -> Result<(), SdkError> {
        // Get the last offset we processed from storage
        let cached_sync_info = object_repository
            .fetch_sync_info()
            .await?
            .unwrap_or_default();
        let current_offset = cached_sync_info.offset;
        let last_synced_final_token_payment_id =
            cached_sync_info.last_synced_final_token_payment_id;

        // We'll keep querying in batches until we have all transfers
        let mut next_filter = Some(PagingFilter {
            offset: current_offset,
            limit: PAYMENT_SYNC_BATCH_SIZE,
            order: Order::Ascending,
        });
        info!("Syncing payments to storage, offset = {}", current_offset);
        let mut pending_payments: u64 = 0;
        while let Some(filter) = next_filter {
            // Get batch of transfers starting from current offset
            let transfers_response = self
                .spark_wallet
                .list_transfers(ListTransfersRequest {
                    paging: Some(filter.clone()),
                    ..Default::default()
                })
                .await?;

            info!(
                "Syncing payments to storage, offset = {}, transfers = {}",
                filter.offset,
                transfers_response.len()
            );
            // Process transfers in this batch
            for transfer in &transfers_response.items {
                // Create a payment record
                let payment: Payment = transfer.clone().try_into()?;
                // Apply any payment metadata for the payment
                if let Err(e) = self.apply_payment_metadata(&payment).await {
                    error!(
                        "Failed to apply payment metadata for payment {}: {e:?}",
                        payment.id
                    );
                }

                // Emit events for new payment statuses after initial sync, or even before initial sync if the payment is pending
                let should_emit =
                    if initial_sync_complete || payment.status == PaymentStatus::Pending {
                        let maybe_existing_payment_status = self
                            .storage
                            .get_payment_by_id(payment.id.clone())
                            .await
                            .ok()
                            .map(|p| p.status);
                        maybe_existing_payment_status.is_none_or(|s| s != payment.status)
                    } else {
                        false
                    };

                // Insert payment into storage
                if let Err(err) = self.storage.insert_payment(payment.clone()).await {
                    error!("Failed to insert payment: {err:?}");
                }
                if payment.status == PaymentStatus::Pending {
                    pending_payments = pending_payments.saturating_add(1);
                }
                info!("Inserted payment: {payment:?}");

                if should_emit {
                    info!("Emitting payment event on sync: {payment:?}");
                    self.event_emitter
                        .emit(&SdkEvent::from_payment(payment.clone()))
                        .await;
                }
            }

            // Check if we have more transfers to fetch
            let cache_offset = filter
                .offset
                .saturating_add(u64::try_from(transfers_response.len())?);

            // Update our last processed offset in the storage. We should remove pending payments
            // from the offset as they might be removed from the list later.
            let save_res = object_repository
                .save_sync_info(&CachedSyncInfo {
                    offset: cache_offset.saturating_sub(pending_payments),
                    last_synced_final_token_payment_id: last_synced_final_token_payment_id.clone(),
                })
                .await;

            if let Err(err) = save_res {
                error!("Failed to update last sync offset: {err:?}");
            }

            next_filter = transfers_response.next;
        }

        Ok(())
    }

    async fn apply_payment_metadata(&self, payment: &Payment) -> Result<(), SdkError> {
        let identifier = match &payment.details {
            Some(PaymentDetails::Lightning { invoice, .. }) => invoice,
            Some(PaymentDetails::Token { tx_hash, .. }) => tx_hash,
            _ => payment.id.as_str(),
        };

        // Get the payment metadata from storage for this payment
        let cache = ObjectCacheRepository::new(self.storage.clone());
        let Some(metadata) = cache.fetch_payment_metadata(identifier).await? else {
            return Ok(());
        };

        self.storage
            .set_payment_metadata(payment.id.clone(), metadata)
            .await?;

        // Delete the payment metadata since we have applied it
        cache.delete_payment_metadata(identifier).await?;

        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    async fn sync_token_payments_to_storage(
        &self,
        object_repository: &ObjectCacheRepository,
        initial_sync_complete: bool,
    ) -> Result<(), SdkError> {
        info!("Syncing token payments to storage");
        // Get the last synced token payment id we processed from storage
        let cached_sync_info = object_repository
            .fetch_sync_info()
            .await?
            .unwrap_or_default();
        let last_synced_final_token_payment_id =
            cached_sync_info.last_synced_final_token_payment_id;
        let our_public_key = self.spark_wallet.get_identity_public_key();

        // We'll keep querying in batches until we have all token tranactions
        let mut payments_to_sync = Vec::new();
        let mut next_offset = 0;
        let mut has_more = true;
        // We'll keep querying in pages until we already have a completed or failed payment stored
        // or we have fetched all transfers
        'page_loop: while has_more {
            info!("Fetching token transactions, offset = {next_offset}");
            // Get batch of token transactions starting from current offset
            let Ok(token_transactions) = self
                .spark_wallet
                .list_token_transactions(ListTokenTransactionsRequest {
                    paging: Some(PagingFilter::new(
                        Some(next_offset),
                        Some(PAYMENT_SYNC_BATCH_SIZE),
                        Some(Order::Descending),
                    )),
                    ..Default::default()
                })
                .await
            else {
                error!(
                    "Failed to fetch address transactions, stopping sync and processing {} payments",
                    payments_to_sync.len()
                );
                break 'page_loop;
            };
            // If no token transactions to sync
            if token_transactions.is_empty() {
                break 'page_loop;
            }
            // Optimization: if the first transaction corresponds to the last synced final token payment id,
            // we can stop syncing
            if let (Some(first_transaction), Some(last_synced_final_token_payment_id)) = (
                token_transactions.items.first(),
                &last_synced_final_token_payment_id,
            ) {
                // Payment ids have the format <transaction_hash>:<output_index>
                if last_synced_final_token_payment_id.starts_with(&first_transaction.hash) {
                    info!(
                        "Last synced token payment id found ({last_synced_final_token_payment_id:?}), stopping sync and processing {} payments",
                        payments_to_sync.len()
                    );
                    has_more = false;
                    break 'page_loop;
                }
            }

            // Get prev out hashes of first input of each token transaction
            // Assumes all inputs of a tx share the same owner public key
            let token_transactions_prevout_hashes = token_transactions
                .items
                .iter()
                .filter_map(|tx| match &tx.inputs {
                    spark_wallet::TokenInputs::Transfer(token_transfer_input) => {
                        token_transfer_input.outputs_to_spend.first().cloned()
                    }
                    spark_wallet::TokenInputs::Mint(..) | spark_wallet::TokenInputs::Create(..) => {
                        None
                    }
                })
                .map(|output| output.prev_token_tx_hash)
                .collect::<Vec<_>>();

            // Since we are trying to fetch at most 1 parent transaction per token transaction,
            // we can fetch all in one go using same batch size
            let Ok(parent_transactions) = self
                .spark_wallet
                .list_token_transactions(ListTokenTransactionsRequest {
                    paging: Some(PagingFilter::new(
                        None,
                        Some(PAYMENT_SYNC_BATCH_SIZE),
                        Some(Order::Descending),
                    )),
                    owner_public_keys: Some(Vec::new()),
                    token_transaction_hashes: token_transactions_prevout_hashes,
                    ..Default::default()
                })
                .await
            else {
                error!(
                    "Failed to fetch parent transactions, stopping sync and processing {} payments",
                    payments_to_sync.len()
                );
                break 'page_loop;
            };

            info!(
                "Syncing token payments to storage, offset = {next_offset}, transactions = {}",
                token_transactions.len()
            );
            // Process transfers in this page
            for transaction in &token_transactions.items {
                let tx_inputs_are_ours = match &transaction.inputs {
                    spark_wallet::TokenInputs::Transfer(token_transfer_input) => {
                        let first_input = token_transfer_input.outputs_to_spend.first().ok_or(
                            SdkError::Generic("No input in token transfer input".to_string()),
                        )?;
                        let parent_transaction = parent_transactions
                            .items
                            .iter()
                            .find(|tx| tx.hash == first_input.prev_token_tx_hash)
                            .ok_or(SdkError::Generic(
                                "Parent transaction not found".to_string(),
                            ))?;
                        let output = parent_transaction
                            .outputs
                            .get(first_input.prev_token_tx_vout as usize)
                            .ok_or(SdkError::Generic("Output not found".to_string()))?;
                        output.owner_public_key == our_public_key
                    }
                    spark_wallet::TokenInputs::Mint(_) | spark_wallet::TokenInputs::Create(_) => {
                        false
                    }
                };

                // Create payment records
                let payments = token_transaction_to_payments(
                    &self.spark_wallet,
                    object_repository,
                    transaction,
                    tx_inputs_are_ours,
                )
                .await?;

                for payment in payments {
                    // Apply any payment metadata for the payment
                    if let Err(e) = self.apply_payment_metadata(&payment).await {
                        error!(
                            "Failed to apply payment metadata for payment {}: {e:?}",
                            payment.id
                        );
                    }
                    if last_synced_final_token_payment_id
                        .as_ref()
                        .is_some_and(|id| payment.id == *id)
                    {
                        info!(
                            "Last synced token payment id found ({last_synced_final_token_payment_id:?}), stopping sync and processing {} payments",
                            payments_to_sync.len()
                        );
                        has_more = false;
                        break 'page_loop;
                    }
                    payments_to_sync.push(payment);
                }
            }

            // Check if we have more transfers to fetch
            next_offset = next_offset.saturating_add(u64::try_from(token_transactions.len())?);
            has_more = token_transactions.len() as u64 == PAYMENT_SYNC_BATCH_SIZE;
        }

        // Insert what synced payments we have into storage, oldest to newest
        payments_to_sync.sort_by_key(|p| p.timestamp);
        for payment in &payments_to_sync {
            // Emit events for new payment statuses after initial sync, or even before initial sync if the payment is pending
            let should_emit = if initial_sync_complete || payment.status == PaymentStatus::Pending {
                let maybe_existing_payment_status = self
                    .storage
                    .get_payment_by_id(payment.id.clone())
                    .await
                    .ok()
                    .map(|p| p.status);
                maybe_existing_payment_status.is_none_or(|s| s != payment.status)
            } else {
                false
            };

            info!("Inserting token payment: {payment:?}");
            if let Err(e) = self.storage.insert_payment(payment.clone()).await {
                error!("Failed to insert token payment: {e:?}");
            }

            if should_emit {
                info!("Emitting payment event on sync: {payment:?}");
                self.event_emitter
                    .emit(&SdkEvent::from_payment(payment.clone()))
                    .await;
            }
        }

        // We have synced all token transactions or found the last synced payment id.
        // If there was a failure to fetch transactions or no transactions exist,
        // we won't update the last synced token payment id
        if !has_more
            && let Some(last_synced_final_token_payment_id) = payments_to_sync
                .into_iter()
                .rfind(|p| p.status.is_final())
                .map(|p| p.id)
        {
            // Update last synced token payment id to the newest final payment we have processed
            info!("Updating last synced token payment id to {last_synced_final_token_payment_id}");
            object_repository
                .save_sync_info(&CachedSyncInfo {
                    offset: cached_sync_info.offset,
                    last_synced_final_token_payment_id: Some(last_synced_final_token_payment_id),
                })
                .await?;
        }

        Ok(())
    }
}
