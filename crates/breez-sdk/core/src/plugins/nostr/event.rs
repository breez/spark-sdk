use breez_nostr::sdk_services::SdkEventListener;

use crate::{EventListener, SdkEvent};

pub struct NostrEventListener {
    pub inner: Box<dyn SdkEventListener>,
}

#[macros::async_trait]
impl EventListener for NostrEventListener {
    async fn on_event(&self, event: SdkEvent) {
        match event {
            SdkEvent::PaymentPending { payment }
            | SdkEvent::PaymentFailed { payment }
            | SdkEvent::PaymentSucceeded { payment } => {
                let Ok(payment) = payment.try_into() else {
                    return;
                };
                self.inner.on_sdk_payment(&payment).await;
            }
            _ => {}
        }
    }
}
