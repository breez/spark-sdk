use std::{rc::Rc, sync::Arc};

use crate::{
    error::WasmResult,
    logger::{Logger, WASM_LOGGER},
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
    network: breez_sdk_spark::Network,
    seed: breez_sdk_spark::Seed,
}

#[wasm_bindgen]
impl SdkBuilder {
    #[wasm_bindgen(js_name = "new")]
    pub fn new(config: Config, seed: Seed) -> Self {
        let config: breez_sdk_spark::Config = config.into();
        let seed: breez_sdk_spark::Seed = seed.into();

        Self {
            network: config.network,
            seed: seed.clone(),
            builder: breez_sdk_spark::SdkBuilder::new(config, seed),
        }
    }

    #[wasm_bindgen(js_name = "withDefaultStorage")]
    pub async fn with_default_storage(mut self, storage_dir: String) -> WasmResult<Self> {
        let storage = Arc::new(WasmStorage {
            storage: default_storage(&storage_dir, &self.network, &self.seed).await?,
        });
        self.builder = self.builder.with_storage(storage.clone());
        self.builder = self.builder.with_real_time_sync_storage(storage);
        Ok(self)
    }

    #[wasm_bindgen(js_name = "withStorage")]
    pub fn with_storage(mut self, storage: Storage) -> Self {
        let storage_arc = Arc::new(WasmStorage { storage });
        self.builder = self.builder.with_storage(storage_arc.clone());
        self.builder = self.builder.with_real_time_sync_storage(storage_arc);
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

    #[wasm_bindgen(js_name = "build")]
    pub async fn build(self) -> WasmResult<BreezSdk> {
        let sdk = self.builder.build().await?;
        Ok(BreezSdk { sdk: Rc::new(sdk) })
    }
}

async fn default_storage(
    data_dir: &str,
    network: &breez_sdk_spark::Network,
    seed: &breez_sdk_spark::Seed,
) -> WasmResult<Storage> {
    let db_path = breez_sdk_spark::default_storage_path(data_dir, network, seed)?;
    // SAFETY: In WASM, thread-local storage is stable and the logger reference
    // will remain valid for the duration of this async function call.
    // The WASM environment is single-threaded, so there's no risk of the
    // logger being moved or deallocated during the async operation.
    let logger_ref = unsafe {
        WASM_LOGGER.with_borrow(|logger| {
            logger
                .as_ref()
                .map(|l| std::mem::transmute::<&Logger, &'static Logger>(l))
        })
    };
    Ok(create_default_storage(db_path.to_string_lossy().as_ref(), logger_ref).await?)
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = "createDefaultStorage", catch)]
    async fn create_default_storage(
        data_dir: &str,
        logger: Option<&Logger>,
    ) -> Result<crate::persist::Storage, JsValue>;
}
