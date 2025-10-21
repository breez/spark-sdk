use std::{rc::Rc, sync::Arc};

use crate::{
    error::WasmResult,
    models::{
        Config, Credentials, KeySetType, Seed,
        chain_service::{BitcoinChainService, WasmBitcoinChainService},
        fiat_service::{FiatService, WasmFiatService},
        payment_observer::{PaymentObserver, WasmPaymentObserver},
        rest_client::{RestClient, WasmRestClient},
    },
    persist::{Storage, WasmStorage},
    sdk::BreezSdk,
};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct SdkBuilder {
    builder: breez_sdk_spark::SdkBuilder,
    storage: Arc<WasmStorage>,
}

#[wasm_bindgen]
impl SdkBuilder {
    #[wasm_bindgen(js_name = "new")]
    pub fn new(config: Config, seed: Seed, storage: Storage) -> WasmResult<Self> {
        let storage = Arc::new(WasmStorage { storage });
        Ok(Self {
            builder: breez_sdk_spark::SdkBuilder::new(config.into(), seed.into(), storage.clone()),
            storage,
        })
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

    #[wasm_bindgen(js_name = "withChainService")]
    pub fn with_chain_service(mut self, chain_service: BitcoinChainService) -> Self {
        self.builder = self
            .builder
            .with_chain_service(Arc::new(WasmBitcoinChainService {
                inner: chain_service,
            }));
        self
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

    #[wasm_bindgen(js_name = "withFiatService")]
    pub fn with_fiat_service(mut self, fiat_service: FiatService) -> Self {
        self.builder = self.builder.with_fiat_service(Arc::new(WasmFiatService {
            inner: fiat_service,
        }));
        self
    }

    #[wasm_bindgen(js_name = "withLnurlClient")]
    pub fn with_lnurl_client(mut self, lnurl_client: RestClient) -> Self {
        self.builder = self.builder.with_lnurl_client(Arc::new(WasmRestClient {
            inner: lnurl_client,
        }));
        self
    }

    #[wasm_bindgen(js_name = "withPaymentObserver")]
    pub fn with_payment_observer(mut self, payment_observer: PaymentObserver) -> Self {
        self.builder = self
            .builder
            .with_payment_observer(Arc::new(WasmPaymentObserver { payment_observer }));
        self
    }

    #[wasm_bindgen(js_name = "withRealTimeSync")]
    pub fn with_real_time_sync(mut self, url: String) -> Self {
        self.builder = self.builder.with_real_time_sync(url, self.storage.clone());
        self
    }

    #[wasm_bindgen(js_name = "build")]
    pub async fn build(self) -> WasmResult<BreezSdk> {
        let sdk = self.builder.build().await?;
        Ok(BreezSdk { sdk: Rc::new(sdk) })
    }
}
