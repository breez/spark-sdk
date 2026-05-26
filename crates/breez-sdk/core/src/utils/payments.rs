use std::sync::Arc;

use tracing::{error, info, warn};

use crate::{
    EventEmitter, Payment, Storage, error::SdkError, events::SdkEvent,
    models::conversion_steps_from_payments,
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

/// Emit a payment event based on whether storage indicated the update should
/// produce one (new payment inserted or status transitioned).
pub(crate) async fn emit_payment_update(
    storage: &Arc<dyn Storage>,
    event_emitter: &EventEmitter,
    payment: Payment,
    should_emit: bool,
) -> bool {
    if should_emit {
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
