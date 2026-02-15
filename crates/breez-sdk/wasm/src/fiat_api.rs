use std::rc::Rc;

use wasm_bindgen::prelude::*;

use crate::{
    error::WasmResult,
    models::{FiatCurrency, Rate, chain_service::RecommendedFees},
};

/// Sub-object for fiat data (exchange rates, currencies, recommended fees).
///
/// Access via `wallet.fiat`.
///
/// ```js
/// const rates = await wallet.fiat.rates();         // → Rate[]
/// const currencies = await wallet.fiat.currencies(); // → FiatCurrency[]
/// const fees = await wallet.fiat.recommendedFees();
/// ```
#[wasm_bindgen(js_name = "FiatApi")]
pub struct FiatApi {
    pub(crate) sdk: Rc<breez_sdk_spark::BreezSdk>,
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

    /// Get the recommended BTC fees.
    #[wasm_bindgen(js_name = "recommendedFees")]
    pub async fn recommended_fees(&self) -> WasmResult<RecommendedFees> {
        Ok(self.sdk.recommended_fees().await?.into())
    }
}
