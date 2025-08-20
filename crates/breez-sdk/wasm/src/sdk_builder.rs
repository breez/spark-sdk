use std::rc::Rc;

use breez_sdk_core::default_storage;
use wasm_bindgen::prelude::*;

use crate::{
    error::WasmResult,
    models::{Config, Credentials},
    sdk::BreezSdk,
};

#[wasm_bindgen]
pub struct SdkBuilder {
    builder: breez_sdk_core::SdkBuilder,
}

#[wasm_bindgen]
impl SdkBuilder {
    #[wasm_bindgen(js_name = "new")]
    pub fn new(config: Config, mnemonic: String, data_dir: String) -> WasmResult<Self> {
        let storage = default_storage(data_dir)?;
        Ok(Self {
            builder: breez_sdk_core::SdkBuilder::new(config.into(), mnemonic, storage),
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

    #[wasm_bindgen(js_name = "build")]
    pub async fn build(self) -> WasmResult<BreezSdk> {
        let sdk = self.builder.build().await?;
        Ok(BreezSdk { sdk: Rc::new(sdk) })
    }
}
