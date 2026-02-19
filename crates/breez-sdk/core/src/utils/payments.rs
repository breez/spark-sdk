use std::sync::Arc;

use tracing::warn;

use crate::{Payment, Storage, error::SdkError};

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
