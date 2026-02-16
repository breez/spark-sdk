use wasm_bindgen::prelude::*;

use crate::{
    error::WasmResult,
    models::{Config, FiatCurrency, Rate},
};

/// A standalone fiat data API that doesn't require a wallet connection.
///
/// Use this to fetch fiat rates and currencies without needing to
/// initialize a wallet.
///
/// ```js
/// import { Fiat, defaultConfig } from '@breeztech/breez-sdk-spark';
/// const fiat = new Fiat(defaultConfig('mainnet'));
/// const rates = await fiat.rates();         // → Rate[]
/// const currencies = await fiat.currencies(); // → FiatCurrency[]
/// ```
#[wasm_bindgen(js_name = "Fiat")]
pub struct Fiat {
    inner: breez_sdk_spark::FiatApi,
}

#[wasm_bindgen(js_class = "Fiat")]
impl Fiat {
    /// Create a new standalone Fiat API from a config.
    #[wasm_bindgen(constructor)]
    pub fn new(config: Config) -> WasmResult<Fiat> {
        let core_config: breez_sdk_spark::Config = config.into();
        let inner = breez_sdk_spark::FiatApi::new(&core_config)?;
        Ok(Fiat { inner })
    }

    /// List the latest fiat exchange rates.
    ///
    /// Returns `Rate[]` directly.
    pub async fn rates(&self) -> WasmResult<Vec<Rate>> {
        let rates = self.inner.rates().await?;
        Ok(rates.into_iter().map(Into::into).collect())
    }

    /// List fiat currencies for which there is a known exchange rate.
    ///
    /// Returns `FiatCurrency[]` directly.
    pub async fn currencies(&self) -> WasmResult<Vec<FiatCurrency>> {
        let currencies = self.inner.currencies().await?;
        Ok(currencies.into_iter().map(Into::into).collect())
    }
}
