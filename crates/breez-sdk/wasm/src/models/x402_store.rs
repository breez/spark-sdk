use wasm_bindgen::{JsCast, prelude::*};
use wasm_bindgen_futures::{
    JsFuture,
    js_sys::{Error, Promise},
};

pub struct WasmX402ReplayStore {
    pub x402_replay_store: X402ReplayStore,
}

// Single-threaded WASM environment makes this safe.
unsafe impl Send for WasmX402ReplayStore {}
unsafe impl Sync for WasmX402ReplayStore {}

fn store_error(js_error: JsValue) -> breez_sdk_spark::X402StoreError {
    let message = js_error
        .dyn_ref::<Error>()
        .map(|e| String::from(e.message()))
        .or_else(|| js_error.as_string())
        .unwrap_or_else(|| "x402 replay store error occurred".to_string());
    breez_sdk_spark::X402StoreError::Generic(message)
}

#[macros::async_trait]
impl breez_sdk_spark::X402ReplayStore for WasmX402ReplayStore {
    async fn consume(
        &self,
        key: String,
        retain_until_secs: u64,
    ) -> Result<bool, breez_sdk_spark::X402StoreError> {
        let promise = self
            .x402_replay_store
            .consume(key, retain_until_secs)
            .map_err(store_error)?;
        let result = JsFuture::from(promise).await.map_err(store_error)?;
        result.as_bool().ok_or_else(|| {
            breez_sdk_spark::X402StoreError::Generic(
                "consume must resolve to a boolean".to_string(),
            )
        })
    }
}

#[wasm_bindgen(typescript_custom_section)]
const X402_REPLAY_STORE_INTERFACE: &'static str = r#"export interface X402ReplayStore {
    consume: (key: string, retainUntilSecs: bigint) => Promise<boolean>;
}"#;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "X402ReplayStore")]
    pub type X402ReplayStore;

    #[wasm_bindgen(structural, method, js_name = consume, catch)]
    pub fn consume(
        this: &X402ReplayStore,
        key: String,
        retain_until_secs: u64,
    ) -> Result<Promise, JsValue>;
}
