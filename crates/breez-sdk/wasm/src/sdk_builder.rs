use std::{rc::Rc, sync::Arc};

use crate::{
    error::{WasmError, WasmResult},
    logger::{Logger, WASM_LOGGER},
    models::{
        Config, Credentials, Seed,
        chain_service::{BitcoinChainService, ChainApiType, WasmBitcoinChainService},
        connection_manager::ConnectionManager,
        fiat_service::{FiatService, WasmFiatService},
        payment_observer::{PaymentObserver, WasmPaymentObserver},
        rest_client::{RestClient, WasmRestClient},
    },
    persist::{Storage, WasmStorage},
    sdk::BreezSdk,
    token_store::{TokenStoreJs, WasmTokenStore},
    tree_store::{TreeStoreJs, WasmTreeStore},
};
use bitcoin::secp256k1::PublicKey;
use breez_sdk_spark::KeySet;
use wasm_bindgen::prelude::*;

/// Configuration for PostgreSQL storage connection pool.
#[derive(Clone, serde::Serialize, serde::Deserialize, tsify_next::Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
#[serde(rename_all = "camelCase")]
pub struct PostgresStorageConfig {
    /// PostgreSQL connection string (URI format).
    pub connection_string: String,
    /// Maximum number of connections in the pool.
    pub max_pool_size: u32,
    /// Timeout in seconds for establishing a new connection (0 = no timeout).
    pub create_timeout_secs: u32,
    /// Timeout in seconds before recycling an idle connection.
    pub recycle_timeout_secs: u32,
}

/// Creates a default PostgreSQL storage configuration with sensible defaults.
///
/// Default values (from pg.Pool):
/// - `maxPoolSize`: 10
/// - `createTimeoutSecs`: 0 (no timeout)
/// - `recycleTimeoutSecs`: 10 (10 seconds idle before disconnect)
#[wasm_bindgen(js_name = "defaultPostgresStorageConfig")]
pub fn default_postgres_storage_config(connection_string: &str) -> PostgresStorageConfig {
    PostgresStorageConfig {
        connection_string: connection_string.to_string(),
        max_pool_size: 10,
        create_timeout_secs: 0,
        recycle_timeout_secs: 10,
    }
}

#[wasm_bindgen]
pub struct SdkBuilder {
    builder: breez_sdk_spark::SdkBuilder,
    network: breez_sdk_spark::Network,
    seed: breez_sdk_spark::Seed,
    default_storage_dir: Option<String>,
    storage: Option<Storage>,
    postgres_backend_config: Option<PostgresStorageConfig>,
    key_set_type: breez_sdk_spark::KeySetType,
    use_address_index: bool,
    account_number: Option<u32>,
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
            default_storage_dir: None,
            storage: None,
            postgres_backend_config: None,
            key_set_type: breez_sdk_spark::KeySetType::Default,
            use_address_index: false,
            account_number: None,
        }
    }

    #[wasm_bindgen(js_name = "newWithSigner")]
    pub fn new_with_signer(config: Config, signer: crate::signer::JsExternalSigner) -> Self {
        use crate::signer::WasmExternalSigner;
        use std::sync::Arc;

        let config_core: breez_sdk_spark::Config = config.into();
        let signer_adapter: Arc<dyn breez_sdk_spark::signer::ExternalSigner> =
            Arc::new(WasmExternalSigner::new(signer));

        Self {
            network: config_core.network,
            seed: breez_sdk_spark::Seed::Entropy(vec![]), // Placeholder, won't be used
            builder: breez_sdk_spark::SdkBuilder::new_with_signer(config_core, signer_adapter),
            default_storage_dir: None,
            storage: None,
            postgres_backend_config: None,
            key_set_type: breez_sdk_spark::KeySetType::Default,
            use_address_index: false,
            account_number: None,
        }
    }

    #[wasm_bindgen(js_name = "withDefaultStorage")]
    pub async fn with_default_storage(mut self, storage_dir: String) -> WasmResult<Self> {
        self.default_storage_dir = Some(storage_dir);
        Ok(self)
    }

    #[wasm_bindgen(js_name = "withStorage")]
    pub fn with_storage(mut self, storage: Storage) -> Self {
        self.storage = Some(storage);
        self
    }

    #[wasm_bindgen(js_name = "withPostgresBackend")]
    pub fn with_postgres_backend(mut self, config: PostgresStorageConfig) -> Self {
        self.postgres_backend_config = Some(config);
        self
    }

    #[wasm_bindgen(js_name = "withKeySet")]
    pub fn with_key_set(mut self, config: crate::models::KeySetConfig) -> Self {
        self.key_set_type = config.key_set_type.clone().into();
        self.use_address_index = config.use_address_index;
        self.account_number = config.account_number;
        let core_config = breez_sdk_spark::KeySetConfig {
            key_set_type: config.key_set_type.into(),
            use_address_index: config.use_address_index,
            account_number: config.account_number,
        };
        self.builder = self.builder.with_key_set(core_config);
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
        api_type: ChainApiType,
        credentials: Option<Credentials>,
    ) -> Self {
        self.builder = self.builder.with_rest_chain_service(
            url,
            api_type.into(),
            credentials.map(|c| c.into()),
        );
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

    #[wasm_bindgen(js_name = "withConnectionManager")]
    pub fn with_connection_manager(mut self, connection_manager: ConnectionManager) -> Self {
        self.builder = self
            .builder
            .with_connection_manager(connection_manager.inner);
        self
    }

    #[wasm_bindgen(js_name = "build")]
    pub async fn build(mut self) -> WasmResult<BreezSdk> {
        match (
            self.default_storage_dir,
            self.storage,
            &self.postgres_backend_config,
        ) {
            (Some(storage_dir), None, None) => {
                // Create key set to get identity_pub_key for WASM-compatible storage
                let key_set = KeySet::new(
                    &self.seed.to_bytes()?,
                    self.network.into(),
                    self.key_set_type.into(),
                    self.use_address_index,
                    self.account_number,
                )
                .map_err(WasmError::new)?;

                let identity_pub_key = key_set.identity_key_pair.public_key();

                let storage = Arc::new(WasmStorage {
                    storage: default_storage(&storage_dir, &self.network, &identity_pub_key)
                        .await?,
                });
                self.builder = self.builder.with_storage(storage);
            }
            (None, Some(storage), None) => {
                let storage_arc = Arc::new(WasmStorage { storage });
                self.builder = self.builder.with_storage(storage_arc);
            }
            (None, None, Some(config)) => {
                let logger_ref = get_wasm_logger_ref();

                // Create a single shared pool for all postgres stores
                let pool = create_postgres_pool(config.clone())?;

                let storage = Arc::new(WasmStorage {
                    storage: create_postgres_storage_with_pool(&pool, logger_ref).await?,
                });
                self.builder = self.builder.with_storage(storage);

                let tree_store_js = create_postgres_tree_store_with_pool(&pool, logger_ref).await?;
                let tree_store = Arc::new(WasmTreeStore::new(tree_store_js));
                self.builder = self.builder.with_tree_store(tree_store);

                let token_store_js =
                    create_postgres_token_store_with_pool(&pool, logger_ref).await?;
                let token_store = Arc::new(WasmTokenStore::new(token_store_js));
                self.builder = self.builder.with_token_output_store(token_store);
            }
            _ => {
                return Err(WasmError::new(
                    "Exactly one of default storage directory, storage, or postgres config must be set",
                ));
            }
        }

        let sdk = self.builder.build().await?;
        Ok(BreezSdk { sdk: Rc::new(sdk) })
    }
}

/// Returns a `'static` reference to the thread-local WASM logger.
///
/// # Safety
///
/// In WASM, thread-local storage is stable and the logger reference will remain
/// valid for the duration of any async function call. The WASM environment is
/// single-threaded, so there's no risk of the logger being moved or deallocated.
fn get_wasm_logger_ref() -> Option<&'static Logger> {
    unsafe {
        WASM_LOGGER.with_borrow(|logger| {
            logger
                .as_ref()
                .map(|l| std::mem::transmute::<&Logger, &'static Logger>(l))
        })
    }
}

async fn default_storage(
    data_dir: &str,
    network: &breez_sdk_spark::Network,
    identity_pub_key: &PublicKey,
) -> WasmResult<Storage> {
    let db_path = breez_sdk_spark::default_storage_path(data_dir, network, identity_pub_key)?;
    let logger_ref = get_wasm_logger_ref();
    Ok(create_default_storage(db_path.to_string_lossy().as_ref(), logger_ref).await?)
}

#[wasm_bindgen]
extern "C" {
    /// JS type representing a `pg.Pool` instance.
    type JsPool;

    #[wasm_bindgen(js_name = "createDefaultStorage", catch)]
    async fn create_default_storage(
        data_dir: &str,
        logger: Option<&Logger>,
    ) -> Result<crate::persist::Storage, JsValue>;

    #[wasm_bindgen(js_name = "createPostgresPool", catch)]
    fn create_postgres_pool(config: PostgresStorageConfig) -> Result<JsPool, JsValue>;

    #[wasm_bindgen(js_name = "createPostgresStorageWithPool", catch)]
    async fn create_postgres_storage_with_pool(
        pool: &JsPool,
        logger: Option<&Logger>,
    ) -> Result<crate::persist::Storage, JsValue>;

    #[wasm_bindgen(js_name = "createPostgresTreeStoreWithPool", catch)]
    async fn create_postgres_tree_store_with_pool(
        pool: &JsPool,
        logger: Option<&Logger>,
    ) -> Result<TreeStoreJs, JsValue>;

    #[wasm_bindgen(js_name = "createPostgresTokenStoreWithPool", catch)]
    async fn create_postgres_token_store_with_pool(
        pool: &JsPool,
        logger: Option<&Logger>,
    ) -> Result<TokenStoreJs, JsValue>;
}
