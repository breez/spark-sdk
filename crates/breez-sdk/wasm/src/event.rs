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

#[wasm_bindgen(typescript_custom_section)]
const EVENT_INTERFACE: &'static str = r#"export interface EventListener {
    onEvent: (e: SdkEvent) => void;
}"#;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "EventListener")]
    pub type EventListener;

    #[wasm_bindgen(structural, method, js_name = onEvent)]
    pub fn on_event(this: &EventListener, e: SdkEvent);
}
