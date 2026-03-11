//! Middleware that suppresses events for conversion child payments.
//!
//! Conversion operations (stable balance, ongoing sends) create child payments
//! (send sats→Flashnet, receive tokens). These child events are internal plumbing
//! and should not reach external listeners. Internal listeners (like `wait_for_payment`)
//! bypass middleware and still see them.

use tracing::info;

use crate::events::{EventMiddleware, SdkEvent};
use crate::models::{Payment, PaymentDetails};

pub struct TokenConversionMiddleware;

impl TokenConversionMiddleware {
    fn has_conversion_info(payment: &Payment) -> bool {
        matches!(
            &payment.details,
            Some(PaymentDetails::Spark {
                conversion_info: Some(_),
                ..
            }) | Some(PaymentDetails::Token {
                conversion_info: Some(_),
                ..
            })
        )
    }
}

#[macros::async_trait]
impl EventMiddleware for TokenConversionMiddleware {
    async fn process(&self, event: SdkEvent) -> Option<SdkEvent> {
        match &event {
            SdkEvent::PaymentSucceeded { payment }
            | SdkEvent::PaymentPending { payment }
            | SdkEvent::PaymentFailed { payment }
                if Self::has_conversion_info(payment) =>
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
