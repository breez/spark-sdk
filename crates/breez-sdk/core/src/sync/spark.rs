use std::{sync::Arc, time::UNIX_EPOCH};

use spark_wallet::{ListTokenTransactionsRequest, Order, PagingFilter, SparkWallet, TokenMetadata};
use tracing::{error, info};

use crate::{
    Payment, PaymentDetails, PaymentMethod, PaymentStatus, PaymentType, SdkError, Storage,
    persist::{CachedSyncInfo, ObjectCacheRepository},
    sync::SyncService,
};

const PAYMENT_SYNC_BATCH_SIZE: u64 = 50;

pub struct SparkSyncService {
    spark_wallet: Arc<SparkWallet>,
    storage: Arc<dyn Storage>,
}

#[macros::async_trait]
impl SyncService for SparkSyncService {
    async fn sync_payments(&self) -> Result<(), SdkError> {
        self.sync_bitcoin_payments_to_storage().await?;
        self.sync_token_payments_to_storage().await
    }

    async fn sync_historical_payments(&self) -> Result<(), SdkError> {
        Ok(())
    }
}

impl SparkSyncService {
    pub fn new(spark_wallet: Arc<SparkWallet>, storage: Arc<dyn Storage>) -> Self {
        Self {
            spark_wallet,
            storage,
        }
    }

    async fn sync_bitcoin_payments_to_storage(&self) -> Result<(), SdkError> {
        // Get the last offset we processed from storage
        let object_repository = ObjectCacheRepository::new(self.storage.clone());
        let cached_sync_info = object_repository
            .fetch_sync_info()
            .await?
            .unwrap_or_default();
        let current_offset = cached_sync_info.offset;

        // We'll keep querying in batches until we have all transfers
        let mut next_offset = current_offset;
        let mut has_more = true;
        info!("Syncing payments to storage, offset = {next_offset}");
        let mut pending_payments: u64 = 0;
        while has_more {
            // Get batch of transfers starting from current offset
            let transfers_response = self
                .spark_wallet
                .list_transfers(
                    Some(PagingFilter::new(
                        Some(next_offset),
                        Some(PAYMENT_SYNC_BATCH_SIZE),
                        Some(Order::Ascending),
                    )),
                    None,
                )
                .await?;

            info!(
                "Syncing bitcoin payments to storage, offset = {next_offset}, transfers = {}",
                transfers_response.len()
            );
            // Process transfers in this batch
            for transfer in &transfers_response {
                // Create a payment record
                let payment: Payment = transfer.clone().try_into()?;
                // Insert payment into storage
                if let Err(err) = self.storage.insert_payment(payment.clone()).await {
                    error!("Failed to insert bitcoin payment: {err:?}");
                }
                if payment.status == PaymentStatus::Pending {
                    pending_payments = pending_payments.saturating_add(1);
                }
                info!("Inserted bitcoin payment: {payment:?}");
            }

            // Check if we have more transfers to fetch
            next_offset = next_offset.saturating_add(u64::try_from(transfers_response.len())?);
            // Update our last processed offset in the storage. We should remove pending payments
            // from the offset as they might be removed from the list later.
            let save_res = object_repository
                .save_sync_info(&CachedSyncInfo {
                    offset: next_offset.saturating_sub(pending_payments),
                })
                .await;
            if let Err(err) = save_res {
                error!("Failed to update last sync bitcoin offset: {err:?}");
            }
            has_more = transfers_response.len() as u64 == PAYMENT_SYNC_BATCH_SIZE;
        }

        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    async fn sync_token_payments_to_storage(&self) -> Result<(), SdkError> {
        info!("Syncing token payments to storage");
        let our_public_key = self.spark_wallet.get_identity_public_key();
        let mut next_offset = 0;
        let mut has_more = true;
        // We'll keep querying in pages until we already have a completed or failed payment stored
        // or we have fetched all transfers
        'page_loop: while has_more {
            // Get batch of token transactions starting from current offset
            let token_transactions = self
                .spark_wallet
                .list_token_transactions(ListTokenTransactionsRequest {
                    paging: Some(PagingFilter::new(
                        Some(next_offset),
                        Some(PAYMENT_SYNC_BATCH_SIZE),
                        None,
                    )),
                    ..Default::default()
                })
                .await?;
            if token_transactions.is_empty() {
                break 'page_loop;
            }

            // Get prev out hashes of first input of each token transaction
            // Assumes all inputs of a tx share the same owner public key
            let token_transactions_prevout_hashes = token_transactions
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
            let parent_transactions = self
                .spark_wallet
                .list_token_transactions(ListTokenTransactionsRequest {
                    paging: Some(PagingFilter::new(None, Some(PAYMENT_SYNC_BATCH_SIZE), None)),
                    owner_public_keys: Some(Vec::new()),
                    token_transaction_hashes: token_transactions_prevout_hashes,
                    ..Default::default()
                })
                .await?;

            info!(
                "Syncing token payments to storage, offset = {next_offset}, transactions = {}",
                token_transactions.len()
            );
            // Process transfers in this page
            for transaction in &token_transactions {
                let tx_inputs_are_ours = match &transaction.inputs {
                    spark_wallet::TokenInputs::Transfer(token_transfer_input) => {
                        let first_input = token_transfer_input.outputs_to_spend.first().ok_or(
                            SdkError::Generic("No input in token transfer input".to_string()),
                        )?;
                        let parent_transaction = parent_transactions
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
                    transaction,
                    tx_inputs_are_ours,
                )
                .await?;

                for payment in payments {
                    // Stop syncing if we encounter a finalized payment that we have already processed
                    if let Ok(Payment {
                        status: PaymentStatus::Completed | PaymentStatus::Failed,
                        ..
                    }) = self.storage.get_payment_by_id(payment.id.clone()).await
                    {
                        info!(
                            "Encountered already finalized payment {}, stopping sync",
                            payment.id
                        );
                        break 'page_loop;
                    }

                    // Insert payment into storage
                    info!("Inserting token payment: {payment:?}");
                    if let Err(err) = self.storage.insert_payment(payment).await {
                        error!("Failed to insert token payment: {err:?}");
                    }
                }
            }

            // Check if we have more transfers to fetch
            next_offset = next_offset.saturating_add(u64::try_from(token_transactions.len())?);
            has_more = token_transactions.len() as u64 == PAYMENT_SYNC_BATCH_SIZE;
        }

        Ok(())
    }

    /// Converts a token transaction to payments
    ///
    /// Each resulting payment corresponds to a potential group of outputs that share the same owner public key.
    /// The id of the payment is the id of the first output in the group.
    ///
    /// Assumptions:
    /// - All outputs of a token transaction share the same token identifier
    /// - All inputs of a token transaction share the same owner public key
    #[allow(clippy::too_many_lines)]
    async fn token_transaction_to_payments(
        &self,
        transaction: &spark_wallet::TokenTransaction,
        tx_inputs_are_ours: bool,
    ) -> Result<Vec<Payment>, SdkError> {
        // Get token metadata for the first output (assuming all outputs have the same token)
        let token_identifier = transaction
            .outputs
            .first()
            .ok_or(SdkError::Generic(
                "No outputs in token transaction".to_string(),
            ))?
            .token_identifier
            .as_ref();
        let metadata: TokenMetadata = self
            .spark_wallet
            .get_tokens_metadata(&[token_identifier])
            .await?
            .first()
            .ok_or(SdkError::Generic("Token metadata not found".to_string()))?
            .clone();

        let is_transfer_transaction =
            matches!(&transaction.inputs, spark_wallet::TokenInputs::Transfer(..));

        let timestamp = transaction
            .created_timestamp
            .duration_since(UNIX_EPOCH)
            .map_err(|_| {
                SdkError::Generic(
                    "Token transaction created timestamp is before UNIX_EPOCH".to_string(),
                )
            })?
            .as_secs();

        // Group outputs by owner public key
        let mut outputs_by_owner = std::collections::HashMap::new();
        for output in &transaction.outputs {
            outputs_by_owner
                .entry(output.owner_public_key)
                .or_insert_with(Vec::new)
                .push(output);
        }

        let mut payments = Vec::new();

        if tx_inputs_are_ours {
            // If inputs are ours, add an outgoing payment for each output group that is not ours
            for (owner_pubkey, outputs) in outputs_by_owner {
                if owner_pubkey != self.spark_wallet.get_identity_public_key() {
                    // This is an outgoing payment to another user
                    let total_amount = outputs
                        .iter()
                        .map(|output| {
                            let amount: u64 = output.token_amount.try_into().unwrap_or_default();
                            amount
                        })
                        .sum();

                    let id = outputs
                        .first()
                        .ok_or(SdkError::Generic("No outputs in output group".to_string()))?
                        .id
                        .clone();

                    let payment = Payment {
                        id,
                        payment_type: PaymentType::Send,
                        status: PaymentStatus::from_token_transaction_status(
                            transaction.status,
                            is_transfer_transaction,
                        ),
                        amount: total_amount,
                        fees: 0, // TODO: calculate actual fees when they start being charged
                        timestamp,
                        method: PaymentMethod::Token,
                        details: Some(PaymentDetails::Token {
                            metadata: metadata.clone().into(),
                            tx_hash: transaction.hash.clone(),
                        }),
                    };

                    payments.push(payment);
                }
                // Ignore outputs that belong to us (potential change outputs)
            }
        } else {
            // If inputs are not ours, add an incoming payment for our output group
            if let Some(our_outputs) =
                outputs_by_owner.get(&self.spark_wallet.get_identity_public_key())
            {
                let total_amount: u64 = our_outputs
                    .iter()
                    .map(|output| {
                        let amount: u64 = output.token_amount.try_into().unwrap_or_default();
                        amount
                    })
                    .sum();

                let id = our_outputs
                    .first()
                    .ok_or(SdkError::Generic(
                        "No outputs in our output group".to_string(),
                    ))?
                    .id
                    .clone();

                let payment = Payment {
                    id,
                    payment_type: PaymentType::Receive,
                    status: PaymentStatus::from_token_transaction_status(
                        transaction.status,
                        is_transfer_transaction,
                    ),
                    amount: total_amount,
                    fees: 0,
                    timestamp,
                    method: PaymentMethod::Token,
                    details: Some(PaymentDetails::Token {
                        metadata: metadata.into(),
                        tx_hash: transaction.hash.clone(),
                    }),
                };

                payments.push(payment);
            }
            // Ignore outputs that don't belong to us
        }

        Ok(payments)
    }
}
