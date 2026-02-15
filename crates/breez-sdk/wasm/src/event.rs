use wasm_bindgen::prelude::*;

use crate::models::SdkEvent;

pub struct WasmEventListener {
    pub listener: EventListener,
}

// This assumes that we'll always be running in a single thread (true for Wasm environments)
unsafe impl Send for WasmEventListener {}
unsafe impl Sync for WasmEventListener {}

#[macros::async_trait]
impl breez_sdk_spark::EventListener for WasmEventListener {
    async fn on_event(&self, event: breez_sdk_spark::SdkEvent) {
        self.listener.on_event(event.into());
    }
}

/// A filtered event listener that checks a predicate before invoking a JS callback.
/// Used by the `on()` convenience method.
pub struct WasmFilteredEventListener {
    pub filter: fn(&breez_sdk_spark::SdkEvent) -> bool,
    pub callback: js_sys::Function,
}

// Safe in single-threaded WASM environments
unsafe impl Send for WasmFilteredEventListener {}
unsafe impl Sync for WasmFilteredEventListener {}

#[macros::async_trait]
impl breez_sdk_spark::EventListener for WasmFilteredEventListener {
    async fn on_event(&self, event: breez_sdk_spark::SdkEvent) {
        if (self.filter)(&event) {
            let wasm_event: SdkEvent = event.into();
            let _ = self.callback.call1(&wasm_bindgen::JsValue::NULL, &wasm_event.into());
        }
    }
}

#[wasm_bindgen(typescript_custom_section)]
const EVENT_INTERFACE: &'static str = r#"export interface EventListener {
    onEvent: (e: WalletEvent) => void;
}
/** @deprecated Use EventListener with WalletEvent instead. */
export interface SdkEventListener {
    onEvent: (e: SdkEvent) => void;
}"#;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "EventListener")]
    pub type EventListener;

    #[wasm_bindgen(structural, method, js_name = onEvent)]
    pub fn on_event(this: &EventListener, e: SdkEvent);
}
