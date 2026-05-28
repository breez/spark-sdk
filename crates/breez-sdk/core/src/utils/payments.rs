use std::str::FromStr;
use std::sync::Arc;

use platform_utils::time::Instant;
use spark_wallet::{
    ListTransfersRequest, SparkWallet, TokenTransaction, TransferId, TransferStatus, WalletTransfer,
};
use tracing::{debug, error, info, warn};

use crate::{
    EventEmitter, Payment, PaymentStatus, Storage,
    error::SdkError,
    events::SdkEvent,
    models::conversion_steps_from_payments,
    persist::{CachedAccountInfo, ObjectCacheRepository},
    sync::SparkSyncService,
    utils::token::token_transaction_to_payments,
};

/// Insert a payment through the storage status guard and emit when requested
/// and when the persisted status advances.
pub(crate) async fn record_payment_update(
    storage: &Arc<dyn Storage>,
    event_emitter: &EventEmitter,
    payment: Payment,
    emit_event: bool,
) -> bool {
    let should_emit = match storage.apply_payment_update(payment.clone()).await {
        Ok(should_emit) => should_emit,
        Err(err) => {
            error!("Failed to apply payment update {}: {err:?}", payment.id);
            return false;
        }
    };

    if emit_event && should_emit {
        get_payment_and_emit_event(storage, event_emitter, payment).await;
        true
    } else {
        false
    }
}

/// Gets the payment from storage to include already stored metadata and conversion details.
/// Emits the appropriate event based on its status. Falls back to the provided
/// payment if the storage lookup fails.
pub(crate) async fn get_payment_and_emit_event(
    storage: &Arc<dyn Storage>,
    event_emitter: &EventEmitter,
    payment: Payment,
) {
    let payment =
        match get_payment_with_conversion_details(payment.id.clone(), Arc::clone(storage)).await {
            Ok(payment) => payment,
            Err(e) => {
                warn!("Failed to fetch payment from storage: {e:?}");
                payment
            }
        };
    info!("Emitting payment event: {payment:?}");
    event_emitter.emit(&SdkEvent::from_payment(payment)).await;
}

/// Process an already-fetched Spark transfer, claiming it locally if
/// it is awaiting our key tweak.
///
/// Returns `None` when the transfer is at a status we cannot yet finalise
/// from (e.g. still pending on the operator) — callers can then choose to
/// poll again, or to skip.
async fn process_spark_transfer_to_payment(
    spark_wallet: &SparkWallet,
    wallet_transfer: WalletTransfer,
) -> Result<Option<Payment>, SdkError> {
    let payment: Payment = match wallet_transfer.status {
        // Already terminal — convert as-is.
        TransferStatus::Completed => wallet_transfer.try_into()?,
        // Claimable — pull the leaves into the local tree-store and promote
        // the status before converting.
        TransferStatus::SenderKeyTweaked => {
            debug!(
                "process_spark_transfer_to_payment({}): claiming",
                wallet_transfer.id
            );
            spark_wallet.process_transfer(&wallet_transfer).await?;
            let mut claimed = wallet_transfer;
            claimed.status = TransferStatus::Completed;
            claimed.try_into()?
        }
        // Terminal-failed — convert without claiming so callers see the
        // `Failed` payment.
        TransferStatus::Expired | TransferStatus::Returned => {
            debug!(
                "process_spark_transfer_to_payment({}): terminal-failed ({})",
                wallet_transfer.id, wallet_transfer.status
            );
            wallet_transfer.try_into()?
        }
        _ => return Ok(None),
    };
    Ok(Some(payment))
}

/// Process an already-fetched token transaction, updating the local token-output
/// store on terminal status.
///
/// Returns `None` when the matching payment isn't found in the transaction's
/// outputs or is still pending.
async fn process_token_transaction_to_payment(
    spark_wallet: &SparkWallet,
    storage: Arc<dyn Storage>,
    token_transaction: &TokenTransaction,
    tx_inputs_are_ours: bool,
    payment_id: &str,
) -> Result<Option<Payment>, SdkError> {
    let object_repository = ObjectCacheRepository::new(storage);
    let payments = token_transaction_to_payments(
        spark_wallet,
        &object_repository,
        token_transaction,
        tx_inputs_are_ours,
    )
    .await?;
    let Some(payment) = payments.into_iter().find(|p| p.id == payment_id) else {
        debug!(
            "process_token_transaction_to_payment({}): no output matches payment_id {payment_id}",
            token_transaction.hash
        );
        return Ok(None);
    };
    if payment.status == PaymentStatus::Pending {
        return Ok(None);
    }
    spark_wallet
        .process_token_transaction(token_transaction)
        .await?;
    Ok(Some(payment))
}

/// Fetch a payment by its id (Spark `transfer_id` or token `{hash}:{vout}`)
/// and process it.
///
/// Returns `Ok(None)` if the underlying transfer/token transaction isn't
/// yet visible on operators, or isn't at a status we can produce a terminal
/// `Payment` for.
pub(crate) async fn fetch_and_process_payment(
    spark_wallet: &SparkWallet,
    storage: Arc<dyn Storage>,
    payment_id: &str,
    tx_inputs_are_ours: bool,
) -> Result<Option<Payment>, SdkError> {
    if let Ok(transfer_id) = TransferId::from_str(payment_id) {
        let mut resp = spark_wallet
            .list_transfers(ListTransfersRequest {
                transfer_ids: vec![transfer_id],
                paging: None,
            })
            .await?;
        let Some(wallet_transfer) = resp.items.pop() else {
            debug!("fetch_and_process_payment({payment_id}): not yet visible on operators");
            return Ok(None);
        };
        debug!(
            "fetch_and_process_payment({payment_id}): spark transfer status={}",
            wallet_transfer.status
        );
        return process_spark_transfer_to_payment(spark_wallet, wallet_transfer).await;
    }

    let Some((tx_hash, _vout)) = payment_id.split_once(':') else {
        return Err(SdkError::Generic(format!(
            "Unrecognized payment_id format: {payment_id}"
        )));
    };
    let token_transactions = spark_wallet
        .get_token_transactions_by_hashes(vec![tx_hash.to_string()])
        .await?;
    let Some(token_transaction) = token_transactions.first() else {
        debug!("fetch_and_process_payment({payment_id}): not yet visible on operators");
        return Ok(None);
    };
    debug!(
        "fetch_and_process_payment({payment_id}): token tx status={:?}",
        token_transaction.status
    );
    process_token_transaction_to_payment(
        spark_wallet,
        storage,
        token_transaction,
        tx_inputs_are_ours,
        payment_id,
    )
    .await
}

/// Apply any cached metadata, refresh balances, then persist the payment
/// through the storage status guard (`record_payment_update`) and emit a
/// status event if storage reports the persisted status advanced. Balances
/// are refreshed before emitting so clients querying state in response to
/// the event observe the new balance. Returns whether an event was emitted.
pub(crate) async fn insert_payment_with_metadata(
    spark_wallet: Arc<SparkWallet>,
    storage: Arc<dyn Storage>,
    event_emitter: Arc<EventEmitter>,
    payment: Payment,
) -> bool {
    let sync_service =
        SparkSyncService::new(spark_wallet.clone(), storage.clone(), event_emitter.clone());
    if let Err(e) = sync_service.apply_payment_metadata(&payment).await {
        error!(
            "insert_payment_with_metadata({}): failed to apply payment metadata: {e:?}",
            payment.id
        );
    }

    if let Err(e) = update_balances(spark_wallet, storage.clone()).await {
        error!("insert_payment_with_metadata: failed to update balances: {e:?}");
    }

    record_payment_update(&storage, event_emitter.as_ref(), payment, true).await
}

/// Refresh the locally-cached balance snapshot (sats + token balances) from
/// the wallet and persist it to storage.
pub(crate) async fn update_balances(
    spark_wallet: Arc<SparkWallet>,
    storage: Arc<dyn Storage>,
) -> Result<(), SdkError> {
    let total_start = Instant::now();

    let t = Instant::now();
    let balance_sats = spark_wallet.get_balance().await?;
    let get_balance_dt = t.elapsed();

    let t = Instant::now();
    let token_balances_raw = spark_wallet.get_token_balances().await?;
    let get_token_balances_dt = t.elapsed();
    let token_balances_count = token_balances_raw.len();
    let token_balances = token_balances_raw
        .into_iter()
        .map(|(k, v)| (k, v.into()))
        .collect();

    let object_repository = ObjectCacheRepository::new(storage.clone());

    let t = Instant::now();
    object_repository
        .save_account_info(&CachedAccountInfo {
            balance_sats,
            token_balances,
        })
        .await?;
    let save_dt = t.elapsed();

    let identity_public_key = spark_wallet.get_identity_public_key();
    info!(
        "Balance updated successfully {} for identity {} (total: {:?}, get_balance: {:?}, get_token_balances[{}]: {:?}, save_account_info: {:?})",
        balance_sats,
        identity_public_key,
        total_start.elapsed(),
        get_balance_dt,
        token_balances_count,
        get_token_balances_dt,
        save_dt
    );
    Ok(())
}

/// Gets a payment from storage by ID to include already stored payment metadata
/// and then enriches it with conversion steps by looking up related child payments.
///
/// Only fetches child payments when `conversion_details` is already set (from persisted
/// metadata), preserving the persisted status while merging in the from/to steps.
pub async fn get_payment_with_conversion_details(
    id: String,
    storage: Arc<dyn Storage>,
) -> Result<Payment, SdkError> {
    let mut payment = storage.get_payment_by_id(id).await?;

    if payment.conversion_details.is_some() {
        let related_payments_map = storage
            .get_payments_by_parent_ids(vec![payment.id.clone()])
            .await?;

        if let Some(related_payments) = related_payments_map.get(&payment.id) {
            match conversion_steps_from_payments(related_payments) {
                Ok((from, to)) => {
                    if let Some(ref mut cd) = payment.conversion_details {
                        cd.from = from;
                        cd.to = to;
                    }
                }
                Err(e) => {
                    warn!("Failed to build conversion steps: {e}");
                }
            }
        }
    }

    Ok(payment)
}
