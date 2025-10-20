use std::{rc::Rc, sync::Arc};

use crate::{
    error::WasmResult,
    models::{Config, Credentials, KeySetType, Seed},
    payment_observer::{PaymentObserver, WasmPaymentObserver},
    persist::{Storage, WasmStorage},
    sdk::BreezSdk,
};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct SdkBuilder {
    builder: breez_sdk_spark::SdkBuilder,
}

#[wasm_bindgen]
impl SdkBuilder {
    #[wasm_bindgen(js_name = "new")]
    pub fn new(config: Config, seed: Seed, storage: Storage) -> WasmResult<Self> {
        Ok(Self {
            builder: breez_sdk_spark::SdkBuilder::new(
                config.into(),
                seed.into(),
                Arc::new(WasmStorage { storage }),
            ),
        })
    }

    #[wasm_bindgen(js_name = "withRestChainService")]
    pub fn with_rest_chain_service(
        mut self,
        url: String,
        credentials: Option<Credentials>,
    ) -> Self {
        self.builder = self
            .builder
            .with_rest_chain_service(url, credentials.map(|c| c.into()));
        self
    }

    #[wasm_bindgen(js_name = "withKeySet")]
    pub fn with_key_set(
        mut self,
        key_set_type: KeySetType,
        use_address_index: bool,
        account_number: Option<u32>,
    ) -> Self {
        self.builder =
            self.builder
                .with_key_set(key_set_type.into(), use_address_index, account_number);
        self
    }

    #[wasm_bindgen(js_name = "withPaymentObserver")]
    pub fn with_payment_observer(mut self, payment_observer: PaymentObserver) -> Self {
        self.builder = self
            .builder
            .with_payment_observer(Arc::new(WasmPaymentObserver { payment_observer }));
        self
    }

    #[wasm_bindgen(js_name = "build")]
    pub async fn build(self) -> WasmResult<BreezSdk> {
        let sdk = self.builder.build().await?;
        Ok(BreezSdk { sdk: Rc::new(sdk) })
    }
}
