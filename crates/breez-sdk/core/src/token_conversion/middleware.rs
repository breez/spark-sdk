//! Middleware that suppresses events for conversion child payments.
//!
//! Conversion operations (stable balance, ongoing sends) create child payments
//! (send sats→Flashnet, receive tokens). These child events are internal plumbing
//! and should not reach external listeners. Internal listeners (like
//! `ClientSyncListener`) bypass middleware and still see them.

use tracing::info;

use crate::events::{EventMiddleware, SdkEvent};

pub struct TokenConversionMiddleware;

#[macros::async_trait]
impl EventMiddleware for TokenConversionMiddleware {
    async fn process(&self, event: SdkEvent) -> Option<SdkEvent> {
        match &event {
            SdkEvent::PaymentSucceeded { payment }
            | SdkEvent::PaymentPending { payment }
            | SdkEvent::PaymentFailed { payment }
            | SdkEvent::PaymentMetadataUpdated { payment }
                if payment.is_conversion_child() =>
            {
                info!(
                    "Suppressing {} event for conversion child payment {}",
                    event, payment.id
                );
                None
            }
            _ => Some(event),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ConversionInfo, ConversionStatus, Payment, PaymentDetails, PaymentMethod, PaymentStatus,
        PaymentType,
    };
    use macros::async_test_all;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    fn spark_payment(conversion_info: Option<ConversionInfo>) -> Payment {
        Payment {
            id: "p1".to_string(),
            payment_type: PaymentType::Receive,
            status: PaymentStatus::Completed,
            amount: 1_000,
            fees: 0,
            timestamp: 100,
            method: PaymentMethod::Spark,
            details: Some(PaymentDetails::Spark {
                invoice_details: None,
                htlc_details: None,
                conversion_info,
            }),
            conversion_details: None,
        }
    }

    fn amm_info() -> ConversionInfo {
        ConversionInfo::Amm {
            pool_id: "pool_1".to_string(),
            conversion_id: "conv_1".to_string(),
            status: ConversionStatus::Completed,
            fee: None,
            purpose: None,
            amount_adjustment: None,
            degradation: None,
        }
    }

    #[async_test_all]
    async fn metadata_update_of_a_conversion_child_is_suppressed() {
        let event = SdkEvent::PaymentMetadataUpdated {
            payment: spark_payment(Some(amm_info())),
        };

        assert!(TokenConversionMiddleware.process(event).await.is_none());
    }

    #[async_test_all]
    async fn metadata_update_of_a_regular_payment_passes_through() {
        let event = SdkEvent::PaymentMetadataUpdated {
            payment: spark_payment(None),
        };

        assert!(TokenConversionMiddleware.process(event).await.is_some());
    }
}
