//! Middleware that suppresses events for conversion child payments.
//!
//! Conversion operations (stable balance, ongoing sends) create child payments
//! (send sats→Flashnet, receive tokens). These child events are internal plumbing
//! and should not reach external listeners. Internal listeners (like `wait_for_payment`)
//! bypass middleware and still see them.

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
