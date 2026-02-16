use std::rc::Rc;

use wasm_bindgen::prelude::*;

use crate::{
    error::WasmResult,
    models::{FiatCurrency, Rate},
};

/// Sub-object for fiat data (exchange rates, currencies).
///
/// Access via `client.fiat`.
///
/// ```js
/// const rates = await client.fiat.rates();         // → Rate[]
/// const currencies = await client.fiat.currencies(); // → FiatCurrency[]
/// ```
#[wasm_bindgen(js_name = "FiatApi")]
pub struct FiatApi {
    pub(crate) sdk: Rc<breez_sdk_spark::BreezClient>,
}

#[wasm_bindgen(js_class = "FiatApi")]
impl FiatApi {
    /// List the latest fiat exchange rates.
    ///
    /// Returns `Rate[]` directly.
    pub async fn rates(&self) -> WasmResult<Vec<Rate>> {
        let rates = self.sdk.list_fiat_rates().await?.rates;
        Ok(rates.into_iter().map(Into::into).collect())
    }

    /// List fiat currencies for which there is a known exchange rate.
    ///
    /// Returns `FiatCurrency[]` directly.
    pub async fn currencies(&self) -> WasmResult<Vec<FiatCurrency>> {
        let currencies = self.sdk.list_fiat_currencies().await?.currencies;
        Ok(currencies.into_iter().map(Into::into).collect())
    }
}
