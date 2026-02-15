use std::rc::Rc;

use wasm_bindgen::prelude::*;

use crate::{
    error::WasmResult,
    event::{EventListener, WasmEventListener, WasmFilteredEventListener},
};

/// Sub-object for event listener management.
///
/// Access via `wallet.events`.
#[wasm_bindgen(js_name = "EventsApi")]
pub struct EventsApi {
    pub(crate) sdk: Rc<breez_sdk_spark::BreezSdk>,
}

#[wasm_bindgen(js_class = "EventsApi")]
impl EventsApi {
    /// Add an event listener.
    ///
    /// Returns a listener ID that can be passed to `remove()`.
    pub async fn add(&self, listener: EventListener) -> String {
        self.sdk
            .add_event_listener(Box::new(WasmEventListener { listener }))
            .await
    }

    /// Remove a previously registered event listener by ID.
    ///
    /// Returns `true` if the listener was found and removed.
    pub async fn remove(&self, id: &str) -> bool {
        self.sdk.remove_event_listener(id).await
    }

    /// Subscribe to a specific event type with a typed callback.
    ///
    /// Returns a listener ID that can be passed to `remove()`.
    ///
    /// Supported event types: `"payment"`, `"paymentSucceeded"`,
    /// `"paymentPending"`, `"paymentFailed"`, `"synced"`.
    ///
    /// ```js
    /// const id = await wallet.events.on("payment", (event) => {
    ///   console.log("Payment event:", event);
    /// });
    /// ```
    pub async fn on(&self, event_type: &str, callback: js_sys::Function) -> WasmResult<String> {
        let filter: fn(&breez_sdk_spark::SdkEvent) -> bool = match event_type {
            "payment" => breez_sdk_spark::SdkEvent::is_payment,
            "paymentSucceeded" => |e| matches!(e, breez_sdk_spark::SdkEvent::PaymentSucceeded { .. }),
            "paymentPending" => |e| matches!(e, breez_sdk_spark::SdkEvent::PaymentPending { .. }),
            "paymentFailed" => |e| matches!(e, breez_sdk_spark::SdkEvent::PaymentFailed { .. }),
            "synced" => breez_sdk_spark::SdkEvent::is_synced,
            _ => {
                return Err(breez_sdk_spark::SdkError::InvalidInput(format!(
                    "Unknown event type: \"{event_type}\". Supported: payment, paymentSucceeded, paymentPending, paymentFailed, synced"
                ))
                .into());
            }
        };

        let listener = WasmFilteredEventListener { filter, callback };
        let id = self.sdk.add_event_listener(Box::new(listener)).await;
        Ok(id)
    }
}
