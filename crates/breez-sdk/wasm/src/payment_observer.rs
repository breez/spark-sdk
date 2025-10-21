use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, js_sys::Promise};

use crate::models::ProvisionalPayment;

pub struct WasmPaymentObserver {
    pub payment_observer: PaymentObserver,
}

// This assumes that we'll always be running in a single thread (true for Wasm environments)
unsafe impl Send for WasmPaymentObserver {}
unsafe impl Sync for WasmPaymentObserver {}

#[macros::async_trait]
impl breez_sdk_spark::PaymentObserver for WasmPaymentObserver {
    async fn before_send(
        &self,
        payments: Vec<breez_sdk_spark::ProvisionalPayment>,
    ) -> Result<(), breez_sdk_spark::PaymentObserverError> {
        let promise = self
            .payment_observer
            .before_send(payments.into_iter().map(ProvisionalPayment::from).collect())
            .map_err(js_error_to_payment_observer_error)?;
        let future = JsFuture::from(promise);
        future.await.map_err(js_error_to_payment_observer_error)?;
        Ok(())
    }
}

#[wasm_bindgen(typescript_custom_section)]
const EVENT_INTERFACE: &'static str = r#"export interface PaymentObserver {
    beforeSend: (payments: ProvisionalPayment[]) => Promise<void>;
}"#;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "PaymentObserver")]
    pub type PaymentObserver;

    #[wasm_bindgen(structural, method, js_name = beforeSend, catch)]
    pub fn before_send(
        this: &PaymentObserver,
        payments: Vec<ProvisionalPayment>,
    ) -> Result<Promise, JsValue>;
}

fn js_error_to_payment_observer_error(js_error: JsValue) -> breez_sdk_spark::PaymentObserverError {
    let error_message = js_error
        .as_string()
        .unwrap_or_else(|| "Payment observer error occurred".to_string());
    breez_sdk_spark::PaymentObserverError::Generic(error_message)
}
