use breez_sdk_spark::ServiceConnectivityError;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, js_sys::Promise};

use crate::models::{FiatCurrency, Rate, error::js_error_to_service_connectivity_error};

pub struct WasmFiatService {
    pub inner: FiatService,
}

// This assumes that we'll always be running in a single thread (true for Wasm environments)
unsafe impl Send for WasmFiatService {}
unsafe impl Sync for WasmFiatService {}

#[macros::async_trait]
impl breez_sdk_spark::FiatService for WasmFiatService {
    async fn fetch_fiat_currencies(
        &self,
    ) -> Result<Vec<breez_sdk_spark::FiatCurrency>, ServiceConnectivityError> {
        let promise = self
            .inner
            .fetch_fiat_currencies()
            .map_err(js_error_to_service_connectivity_error)?;
        let future = JsFuture::from(promise);
        let result = future
            .await
            .map_err(js_error_to_service_connectivity_error)?;
        let fiat_currencies: Vec<FiatCurrency> = serde_wasm_bindgen::from_value(result)
            .map_err(|e| ServiceConnectivityError::Other(e.to_string()))?;
        Ok(fiat_currencies.into_iter().map(|p| p.into()).collect())
    }

    async fn fetch_fiat_rates(
        &self,
    ) -> Result<Vec<breez_sdk_spark::Rate>, ServiceConnectivityError> {
        let promise = self
            .inner
            .fetch_fiat_rates()
            .map_err(js_error_to_service_connectivity_error)?;
        let future = JsFuture::from(promise);
        let result = future
            .await
            .map_err(js_error_to_service_connectivity_error)?;
        let rates: Vec<Rate> = serde_wasm_bindgen::from_value(result)
            .map_err(|e| ServiceConnectivityError::Other(e.to_string()))?;
        Ok(rates.into_iter().map(|p| p.into()).collect())
    }
}

#[wasm_bindgen(typescript_custom_section)]
const EVENT_INTERFACE: &'static str = r#"export interface FiatService {
    fetchFiatCurrencies(): Promise<FiatCurrency[]>;
    fetchFiatRates(): Promise<Rate[]>;
}"#;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "FiatService")]
    pub type FiatService;

    #[wasm_bindgen(structural, method, js_name = "fetchFiatCurrencies", catch)]
    pub fn fetch_fiat_currencies(this: &FiatService) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "fetchFiatRates", catch)]
    pub fn fetch_fiat_rates(this: &FiatService) -> Result<Promise, JsValue>;
}
