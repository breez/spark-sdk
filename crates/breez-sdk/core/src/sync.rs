use std::sync::Arc;

use spark_wallet::{ListTokenTransactionsRequest, Order, PagingFilter, SparkWallet};
use tracing::{error, info};

use crate::{
    Payment, PaymentStatus, SdkError, Storage,
    persist::{CachedSyncInfo, ObjectCacheRepository},
    utils::token::token_transaction_to_payments,
};

const PAYMENT_SYNC_BATCH_SIZE: u64 = 50;

pub(crate) struct SparkSyncService {
    spark_wallet: Arc<SparkWallet>,
    storage: Arc<dyn Storage>,
}

impl SparkSyncService {
    pub fn new(spark_wallet: Arc<SparkWallet>, storage: Arc<dyn Storage>) -> Self {
        Self {
            spark_wallet,
            storage,
        }
    }

    pub async fn sync_payments(&self) -> Result<(), SdkError> {
        let object_repository = ObjectCacheRepository::new(self.storage.clone());
        self.sync_bitcoin_payments_to_storage(&object_repository)
            .await?;
        self.sync_token_payments_to_storage(&object_repository)
            .await
    }

    async fn sync_bitcoin_payments_to_storage(
        &self,
        object_repository: &ObjectCacheRepository,
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
                .list_transfers(Some(filter.clone()))
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
                // Insert payment into storage
                if let Err(err) = self.storage.insert_payment(payment.clone()).await {
                    error!("Failed to insert payment: {err:?}");
                }
                if payment.status == PaymentStatus::Pending {
                    pending_payments = pending_payments.saturating_add(1);
                }
                info!("Inserted payment: {payment:?}");
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

    #[allow(clippy::too_many_lines)]
    async fn sync_token_payments_to_storage(
        &self,
        object_repository: &ObjectCacheRepository,
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
                        None,
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
                    paging: Some(PagingFilter::new(None, Some(PAYMENT_SYNC_BATCH_SIZE), None)),
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
            info!("Inserting token payment: {payment:?}");
            if let Err(e) = self.storage.insert_payment(payment.clone()).await {
                error!("Failed to insert token payment: {e:?}");
            }
        }

        // We have synced all token transactions or found the last synced payment id.
        // If there was a failure to fetch transactions or no transactions exist,
        // we won't update the last synced token payment id
        if !has_more
            && let Some(last_synced_final_token_payment_id) = payments_to_sync
                .into_iter()
                .filter(|p| p.status.is_final())
                .next_back()
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
