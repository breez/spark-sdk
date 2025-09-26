use std::{collections::HashMap, sync::Arc};

use spark_wallet::{ListTokenTransactionsRequest, SparkWallet, SspUserRequest, TokenInputs};
use tracing::{error, info};

use crate::{
    Config, Payment, PaymentDetails, PaymentMethod, PaymentStatus, SdkError, Storage,
    adaptors::sparkscan::payments_from_address_transaction_and_ssp_request,
    persist::ObjectCacheRepository, sync::SyncService,
};

const PAYMENT_SYNC_BATCH_SIZE: u64 = 25;
const PAYMENT_SYNC_TAIL_MAX_PAGES: u64 = 5;

struct AddressTransactionsWithSspUserRequests {
    address_transactions: Vec<sparkscan::types::AddressTransaction>,
    ssp_user_requests: HashMap<String, SspUserRequest>,
}

pub struct SparkscanSyncService {
    config: Config,
    spark_wallet: Arc<SparkWallet>,
    storage: Arc<dyn Storage>,
}

#[macros::async_trait]
impl SyncService for SparkscanSyncService {
    async fn sync_payments(&self) -> Result<(), SdkError> {
        self.sync_pending_payments().await?;
        self.sync_payments_head_to_storage().await?;
        self.sync_payments_tail_to_storage().await
    }
}

impl SparkscanSyncService {
    pub fn new(config: Config, spark_wallet: Arc<SparkWallet>, storage: Arc<dyn Storage>) -> Self {
        Self {
            config,
            spark_wallet,
            storage,
        }
    }

    async fn fetch_address_transactions_with_ssp_user_requests(
        &self,
        legacy_spark_address: &str,
        offset: u64,
    ) -> Result<AddressTransactionsWithSspUserRequests, SdkError> {
        let response = sparkscan::Client::new(&self.config.sparkscan_api_url)
            .get_address_transactions_v1_address_address_transactions_get()
            .network(sparkscan::types::Network::from(self.config.network))
            .address(legacy_spark_address)
            .offset(offset)
            .limit(PAYMENT_SYNC_BATCH_SIZE)
            .send()
            .await?;
        let address_transactions = response.data.clone();
        let ssp_transfer_types = [
            sparkscan::types::AddressTransactionType::BitcoinDeposit,
            sparkscan::types::AddressTransactionType::BitcoinWithdrawal,
            sparkscan::types::AddressTransactionType::LightningPayment,
        ];
        let ssp_user_requests = self
            .spark_wallet
            .query_ssp_user_requests(
                address_transactions
                    .iter()
                    .filter(|tx| ssp_transfer_types.contains(&tx.type_))
                    .map(|tx| tx.id.clone())
                    .collect(),
            )
            .await?;

        Ok(AddressTransactionsWithSspUserRequests {
            address_transactions,
            ssp_user_requests,
        })
    }

    /// Syncs pending payments so that we have their latest status
    /// Uses the Spark SDK API to get the latest status of the payments
    async fn sync_pending_payments(&self) -> Result<(), SdkError> {
        // TODO: implement pending payment syncing using sparkscan API (including live updates)
        // Advantages:
        // - No need to maintain payment adapter code for both models
        // - Can use live updates from sparkscan API
        // Why it can't be done now:
        // - Sparkscan needs one of the following:
        //   - Batch transaction querying by id
        //   - Sorting by updated_at timestamp in address transactions query (simpler)

        let pending_payments = self
            .storage
            .list_payments(None, None, Some(PaymentStatus::Pending))
            .await?;

        let (pending_token_payments, pending_bitcoin_payments): (Vec<_>, Vec<_>) = pending_payments
            .iter()
            .partition(|p| p.method == PaymentMethod::Token);

        info!(
            "Syncing pending bitcoin payments: {}",
            pending_bitcoin_payments.len()
        );
        self.sync_pending_bitcoin_payments(&pending_bitcoin_payments)
            .await?;
        info!(
            "Syncing pending token payments: {}",
            pending_token_payments.len()
        );
        self.sync_pending_token_payments(&pending_token_payments)
            .await?;

        Ok(())
    }

    async fn sync_pending_bitcoin_payments(
        &self,
        pending_bitcoin_payments: &[&Payment],
    ) -> Result<(), SdkError> {
        if pending_bitcoin_payments.is_empty() {
            return Ok(());
        }

        let transfer_ids: Vec<_> = pending_bitcoin_payments
            .iter()
            .map(|p| p.id.clone())
            .collect();

        let transfers = self
            .spark_wallet
            .list_transfers(None, Some(transfer_ids.clone()))
            .await?;

        for transfer in transfers {
            let payment = Payment::try_from(transfer)?;
            info!("Inserting previously pending bitcoin payment: {payment:?}");
            self.storage.insert_payment(payment).await?;
        }

        Ok(())
    }

    async fn sync_pending_token_payments(
        &self,
        pending_token_payments: &[&Payment],
    ) -> Result<(), SdkError> {
        if pending_token_payments.is_empty() {
            return Ok(());
        }

        let hash_pending_token_payments_map = pending_token_payments.iter().try_fold(
            std::collections::HashMap::new(),
            |mut acc: std::collections::HashMap<&_, Vec<_>>, payment| {
                let details = payment
                    .details
                    .as_ref()
                    .ok_or_else(|| SdkError::Generic("Payment details missing".to_string()))?;

                if let PaymentDetails::Token { tx_hash, .. } = details {
                    acc.entry(tx_hash).or_default().push(payment);
                    Ok(acc)
                } else {
                    Err(SdkError::Generic(
                        "Payment is not a token payment".to_string(),
                    ))
                }
            },
        )?;

        let token_transactions = self
            .spark_wallet
            .list_token_transactions(ListTokenTransactionsRequest {
                token_transaction_hashes: hash_pending_token_payments_map
                    .keys()
                    .map(|k| (*k).to_string())
                    .collect(),
                ..Default::default()
            })
            .await?;

        for token_transaction in token_transactions {
            let is_transfer_transaction =
                matches!(token_transaction.inputs, TokenInputs::Transfer(..));
            let payment_status = PaymentStatus::from_token_transaction_status(
                token_transaction.status,
                is_transfer_transaction,
            );
            if payment_status != PaymentStatus::Pending {
                let payments_to_update = hash_pending_token_payments_map
                    .get(&token_transaction.hash)
                    .ok_or(SdkError::Generic("Payment not found".to_string()))?;
                for payment in payments_to_update {
                    // For now, updating the status is enough
                    let mut updated_payment = (**payment).clone();
                    updated_payment.status = payment_status;
                    info!("Inserting previously pending token payment: {updated_payment:?}");
                    self.storage.insert_payment(updated_payment).await?;
                }
            }
        }

        Ok(())
    }

    /// Synchronizes payments from the Spark network to persistent storage using the Sparkscan API.
    ///
    /// When syncing from head, it will start from the `next_head_offset` and page until it finds the
    /// `last_synced_payment_id`. If `max_pages` is reached or we get an error response from the Sparkscan
    /// API, it will update the `next_head_offset` for the next sync. This allows us to gradually sync the
    /// head up to the `last_synced_payment_id` in multiple sync cycles.
    async fn sync_payments_head_to_storage(&self) -> Result<(), SdkError> {
        info!("Syncing payments head to storage");
        let object_repository = ObjectCacheRepository::new(self.storage.clone());
        let cached_sync_info = object_repository
            .fetch_sparkscan_sync_info()
            .await?
            .unwrap_or_default();
        // TODO: use new spark address format once sparkscan supports it
        let legacy_spark_address = self
            .spark_wallet
            .get_spark_address()?
            .to_string_with_hrp_legacy();
        let last_synced_id = cached_sync_info.last_synced_payment_id;
        let (max_pages, mut head_synced) = if last_synced_id.is_some() {
            (u64::MAX, false)
        } else {
            info!("No last synced payment id found syncing from head, setting max_pages to 1");
            // There is no cached last synced payment id, limit the number of pages when syncing
            // from head. Then let the rest of the payments be synced by the tail sync.
            // Set `head_synced` to true to store the first payment as the last synced payment id.
            (1, true)
        };
        // Sync from the next head offset in case we didn't finish syncing the head last time
        let mut next_offset = cached_sync_info.next_head_offset;
        let mut payments_to_sync = Vec::new();
        'page_loop: for page in 1..=max_pages {
            info!(
                "Fetching address transactions, offset = {next_offset}, page = {page}/{max_pages}"
            );
            let Ok(AddressTransactionsWithSspUserRequests {
                address_transactions,
                ssp_user_requests,
            }) = self
                .fetch_address_transactions_with_ssp_user_requests(
                    &legacy_spark_address,
                    next_offset,
                )
                .await
            else {
                error!("Failed to fetch address transactions, stopping sync");
                break 'page_loop;
            };

            info!(
                "Processing address transactions, offset = {next_offset}, transactions = {}",
                address_transactions.len()
            );
            // Process transactions in this batch
            for transaction in &address_transactions {
                // Create payment records
                let payments = payments_from_address_transaction_and_ssp_request(
                    transaction,
                    ssp_user_requests.get(&transaction.id),
                    &legacy_spark_address,
                )?;

                for payment in payments {
                    if last_synced_id.as_ref().is_some_and(|id| payment.id == *id) {
                        info!(
                            "Last synced payment id found ({last_synced_id:?}), stopping sync and proceeding to insert {} payments",
                            payments_to_sync.len()
                        );
                        head_synced = true;
                        break 'page_loop;
                    }
                    payments_to_sync.push(payment);
                }
            }

            // Check if we have more transfers to fetch
            next_offset = next_offset.saturating_add(u64::try_from(address_transactions.len())?);
            if (address_transactions.len() as u64) < PAYMENT_SYNC_BATCH_SIZE {
                head_synced = true;
                break 'page_loop;
            }
        }

        // Insert what synced payments we have into storage from oldest to newest
        payments_to_sync.sort_by_key(|p| p.timestamp);
        for payment in payments_to_sync {
            self.storage.insert_payment(payment.clone()).await?;
            info!("Inserted payment: {payment:?}");
            let (last_synced_payment_id, next_head_offset) = if head_synced {
                (Some(payment.id.clone()), 0)
            } else {
                (None, next_offset)
            };
            object_repository
                .merge_sparkscan_sync_info(last_synced_payment_id, Some(next_head_offset), None)
                .await?;
        }

        Ok(())
    }

    /// Synchronizes payments from the Spark network to persistent storage using the Sparkscan API.
    ///
    /// When syncing from tail, it will start from the count of completed payments in storage and page for
    /// a maximum of `PAYMENT_SYNC_TAIL_MAX_PAGES` for one sync cycle. Once it reaches the end page,
    /// it will update `tail_synced` in the sync info and the tail will no longer be synced in future cycles.
    async fn sync_payments_tail_to_storage(&self) -> Result<(), SdkError> {
        info!("Syncing payments tail to storage");
        let object_repository = ObjectCacheRepository::new(self.storage.clone());
        let cached_sync_info = object_repository
            .fetch_sparkscan_sync_info()
            .await?
            .unwrap_or_default();
        if cached_sync_info.tail_synced {
            info!("Payments tail already synced, skipping");
            return Ok(());
        }
        // TODO: use new spark address format once sparkscan supports it
        let legacy_spark_address = self
            .spark_wallet
            .get_spark_address()?
            .to_string_with_hrp_legacy();
        let mut next_offset = self
            .storage
            .list_payments(None, None, Some(PaymentStatus::Completed))
            .await?
            .len() as u64;
        let mut tail_synced = false;

        for page in 1..=PAYMENT_SYNC_TAIL_MAX_PAGES {
            info!(
                "Fetching address transactions, offset = {next_offset}, page = {page}/{PAYMENT_SYNC_TAIL_MAX_PAGES}"
            );
            let Ok(AddressTransactionsWithSspUserRequests {
                address_transactions,
                ssp_user_requests,
            }) = self
                .fetch_address_transactions_with_ssp_user_requests(
                    &legacy_spark_address,
                    next_offset,
                )
                .await
            else {
                error!("Failed to fetch address transactions, stopping sync");
                break;
            };

            info!(
                "Processing address transactions, offset = {next_offset}, transactions = {}",
                address_transactions.len()
            );
            // Process transactions in this batch
            for transaction in &address_transactions {
                let payments = payments_from_address_transaction_and_ssp_request(
                    transaction,
                    ssp_user_requests.get(&transaction.id),
                    &legacy_spark_address,
                )?;
                // Insert payments
                for payment in payments {
                    self.storage.insert_payment(payment).await?;
                }
            }

            // Check if we have more transfers to fetch
            next_offset = next_offset.saturating_add(u64::try_from(address_transactions.len())?);
            if (address_transactions.len() as u64) < PAYMENT_SYNC_BATCH_SIZE {
                tail_synced = true;
                break;
            }
        }

        object_repository
            .merge_sparkscan_sync_info(None, None, Some(tail_synced))
            .await?;

        Ok(())
    }
}
