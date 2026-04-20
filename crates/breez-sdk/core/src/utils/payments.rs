use std::str::FromStr;
use std::sync::Arc;

use spark_wallet::SparkWallet;
use tracing::{debug, info, warn};

use crate::{
    ConversionInfo, EventEmitter, Payment, PaymentDetails, PaymentMetadata, Storage,
    error::SdkError, events::SdkEvent, models::conversion_steps_from_payments,
    persist::ObjectCacheRepository, utils::token::token_transaction_to_payments,
};

/// Extract `ConversionInfo` from whichever [`PaymentDetails`] variant carries
/// it. Cross-chain conversion info can sit on `Lightning` (Boltz hold-invoice
/// pays), `Spark`, or `Token` details — this helper hides the variant match
/// so callers can write a single destructure regardless of provider.
pub(crate) fn extract_conversion_info(details: Option<PaymentDetails>) -> Option<ConversionInfo> {
    match details? {
        PaymentDetails::Spark {
            conversion_info, ..
        }
        | PaymentDetails::Token {
            conversion_info, ..
        }
        | PaymentDetails::Lightning {
            conversion_info, ..
        } => conversion_info,
        _ => None,
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

/// Resolves a Spark transfer ID or token transaction hash to a payment ID.
///
/// If `identifier` is a valid [`TransferId`] it is returned directly (Spark
/// transfers use the transfer ID as the payment ID). Otherwise we look up
/// the token transaction by hash and extract the payment ID from it.
///
/// Used by both `FlashnetTokenConverter` (AMM conversions) and
/// `OrchestraService` (cross-chain sends) when attaching metadata to the
/// outgoing Spark payment.
pub(crate) async fn resolve_payment_id(
    identifier: &str,
    spark_wallet: &SparkWallet,
    storage: &Arc<dyn Storage>,
    tx_inputs_are_ours: bool,
) -> Result<String, SdkError> {
    use spark_wallet::TransferId;

    debug!("Resolving payment id for identifier: {identifier}");

    if let Ok(transfer_id) = TransferId::from_str(identifier) {
        return Ok(transfer_id.to_string());
    }

    // It's a token transaction hash — look up the transaction and derive the payment.
    let token_transactions = spark_wallet
        .get_token_transactions_by_hashes(vec![identifier.to_string()])
        .await
        .map_err(|e| SdkError::Generic(format!("Failed to get token transactions: {e}")))?;

    let token_transaction = token_transactions
        .first()
        .ok_or_else(|| SdkError::Generic("Token transaction not found".to_string()))?;

    let object_repository = ObjectCacheRepository::new(Arc::clone(storage));
    let payments = token_transaction_to_payments(
        spark_wallet,
        &object_repository,
        token_transaction,
        tx_inputs_are_ours,
    )
    .await?;

    payments
        .first()
        .map(|p| p.id.clone())
        .ok_or_else(|| SdkError::Generic("Payment id not found for token transaction".to_string()))
}

/// Inserts payment metadata by first resolving the identifier to a payment ID.
/// If the payment ID can't be resolved yet (async sync hasn't processed the
/// transfer), caches the metadata for later attachment.
///
/// Returns the resolved payment ID, or the raw identifier if it was cached.
pub(crate) async fn insert_or_cache_payment_metadata(
    identifier: &str,
    metadata: PaymentMetadata,
    spark_wallet: &SparkWallet,
    storage: &Arc<dyn Storage>,
    tx_inputs_are_ours: bool,
) -> Result<String, SdkError> {
    match resolve_payment_id(identifier, spark_wallet, storage, tx_inputs_are_ours).await {
        Ok(payment_id) => {
            debug!("Resolved payment id {payment_id} for identifier {identifier}");
            storage
                .insert_payment_metadata(payment_id.clone(), metadata)
                .await
                .map_err(|e| {
                    SdkError::Generic(format!("Failed to insert payment metadata: {e}"))
                })?;
            Ok(payment_id)
        }
        Err(e) => {
            debug!("Could not resolve payment id for {identifier}: {e}, caching metadata");
            let cache = ObjectCacheRepository::new(Arc::clone(storage));
            cache
                .save_payment_metadata(identifier, &metadata)
                .await
                .map_err(|e| SdkError::Generic(format!("Failed to cache payment metadata: {e}")))?;
            Ok(identifier.to_string())
        }
    }
}
