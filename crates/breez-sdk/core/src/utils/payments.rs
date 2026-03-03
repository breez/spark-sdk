use std::sync::Arc;

use tracing::{info, warn};

use crate::{EventEmitter, Payment, Storage, error::SdkError, events::SdkEvent};

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
/// and then enriches it with conversion details by looking up related payments.
pub async fn get_payment_with_conversion_details(
    id: String,
    storage: Arc<dyn Storage>,
) -> Result<Payment, SdkError> {
    let mut payment = storage.get_payment_by_id(id).await?;

    // Load related payments (single ID batch)
    let related_payments_map = storage
        .get_payments_by_parent_ids(vec![payment.id.clone()])
        .await?;

    if let Some(related_payments) = related_payments_map.get(&payment.id) {
        match related_payments.try_into() {
            Ok(conversion_details) => payment.conversion_details = Some(conversion_details),
            Err(e) => {
                warn!("Related payments not convertable to ConversionDetails: {e}");
            }
        }
    }

    Ok(payment)
}
