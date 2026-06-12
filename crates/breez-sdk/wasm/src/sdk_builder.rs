use std::{rc::Rc, sync::Arc};

use crate::{
    error::{WasmError, WasmResult},
    logger::{Logger, WASM_LOGGER},
    models::{
        Config, Credentials, Seed,
        chain_service::{BitcoinChainService, ChainApiType, WasmBitcoinChainService},
        fiat_service::{FiatService, WasmFiatService},
        payment_observer::{PaymentObserver, WasmPaymentObserver},
        rest_client::{RestClient, WasmRestClient},
        session_store::WasmSessionStore,
    },
    persist::{
        Storage, WasmStorage,
        pool::{
            JsPool, create_mysql_pool, create_mysql_session_store_with_pool,
            create_mysql_storage_with_pool, create_mysql_token_store_with_pool,
            create_mysql_tree_store_with_pool, create_postgres_pool,
            create_postgres_session_store_with_pool, create_postgres_storage_with_pool,
            create_postgres_token_store_with_pool, create_postgres_tree_store_with_pool,
        },
    },
    sdk::BreezSdk,
    sdk_context::{SharedMysqlPool, SharedPostgresPool, WasmSdkContext},
    token_store::WasmTokenStore,
    tree_store::WasmTreeStore,
};
use bitcoin::secp256k1::PublicKey;
use breez_sdk_spark::{PrebuiltBackend, SessionStoreAdapter, StorageBackend, identity_public_key};
use platform_utils::tokio::sync::OnceCell;
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
    /// Whether the SDK should run schema migrations on startup. Set to
    /// `false` when the embedding service owns and migrates the database
    /// schema. Defaults to `true`.
    #[serde(default = "default_run_migration")]
    pub run_migration: bool,
}

fn default_run_migration() -> bool {
    true
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
        run_migration: true,
    }
}

/// Controls whether MySQL migrations create database-enforced foreign keys.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    tsify_next::Tsify,
)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub enum MysqlForeignKeyMode {
    /// Create foreign-key constraints in the managed schema.
    #[default]
    Enforced,
    /// Omit foreign-key constraints from the managed schema.
    Disabled,
}

/// Configuration for MySQL storage connection pool. Targets MySQL 8.0+.
#[derive(Clone, serde::Serialize, serde::Deserialize, tsify_next::Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
#[serde(rename_all = "camelCase")]
pub struct MysqlStorageConfig {
    /// MySQL connection URL (e.g. `mysql://user:pass@host:3306/dbname`).
    pub connection_string: String,
    /// Maximum number of connections in the pool.
    pub max_pool_size: u32,
    /// Timeout in seconds for establishing a new connection (0 = no timeout).
    pub create_timeout_secs: u32,
    /// Timeout in seconds before recycling an idle connection.
    pub recycle_timeout_secs: u32,
    /// Whether the SDK should run schema migrations on startup. Set to
    /// `false` when the embedding service owns and migrates the database
    /// schema. Defaults to `true`.
    #[serde(default = "default_run_migration")]
    pub run_migration: bool,
    /// Whether migrations should create database-enforced foreign keys.
    ///
    /// Use `Disabled` for environments that manage relationships in
    /// application code and require schema changes without foreign-key
    /// constraints.
    #[serde(default)]
    pub foreign_key_mode: MysqlForeignKeyMode,
}

/// Creates a default MySQL storage configuration with sensible defaults.
///
/// Default values:
/// - `maxPoolSize`: 10
/// - `createTimeoutSecs`: 0 (no timeout)
/// - `recycleTimeoutSecs`: 10
/// - `foreignKeyMode`: `Enforced`
#[wasm_bindgen(js_name = "defaultMysqlStorageConfig")]
pub fn default_mysql_storage_config(connection_string: &str) -> MysqlStorageConfig {
    MysqlStorageConfig {
        connection_string: connection_string.to_string(),
        max_pool_size: 10,
        create_timeout_secs: 0,
        recycle_timeout_secs: 10,
        run_migration: true,
        foreign_key_mode: MysqlForeignKeyMode::Enforced,
    }
}

/// The built-in storage backend a [`SdkBuilder`] uses.
enum WasmStorageConfigKind {
    /// File-based storage rooted at `storage_dir` (IndexedDB in the browser,
    /// SQLite under Node.js).
    Default { storage_dir: String },
    /// `PostgreSQL`-backed storage.
    Postgres { config: PostgresStorageConfig },
    /// `MySQL`-backed storage.
    Mysql { config: MysqlStorageConfig },
}

/// Selects one of the SDK's built-in storage backends.
///
/// Construct it via `defaultStorage`, `postgresStorage` or `mysqlStorage` and
/// pass it to `SdkBuilder.withStorageBackend`.
#[wasm_bindgen]
pub struct WasmStorageConfig {
    kind: WasmStorageConfigKind,
}

/// File-based storage rooted at `storageDir` — IndexedDB in the browser,
/// SQLite under Node.js.
#[wasm_bindgen(js_name = "defaultStorage")]
#[must_use]
pub fn default_storage_config(storage_dir: String) -> WasmStorageConfig {
    WasmStorageConfig {
        kind: WasmStorageConfigKind::Default { storage_dir },
    }
}

/// `PostgreSQL`-backed storage built from `config`.
#[wasm_bindgen(js_name = "postgresStorage")]
#[must_use]
pub fn postgres_storage(config: PostgresStorageConfig) -> WasmStorageConfig {
    WasmStorageConfig {
        kind: WasmStorageConfigKind::Postgres { config },
    }
}

/// `MySQL`-backed storage built from `config`.
#[wasm_bindgen(js_name = "mysqlStorage")]
#[must_use]
pub fn mysql_storage(config: MysqlStorageConfig) -> WasmStorageConfig {
    WasmStorageConfig {
        kind: WasmStorageConfigKind::Mysql { config },
    }
}

#[wasm_bindgen]
pub struct SdkBuilder {
    builder: breez_sdk_spark::SdkBuilder,
    network: breez_sdk_spark::Network,
    seed: breez_sdk_spark::Seed,
    /// Storage backend selected via `withDefaultStorage` / `withStorageBackend`.
    storage_config: Option<WasmStorageConfig>,
    storage: Option<Storage>,
    /// JS Postgres pool supplied via `withSharedContext(ctx_with_pool)`.
    context_postgres_pool: Option<SharedPostgresPool>,
    /// JS MySQL pool supplied via `withSharedContext(ctx_with_pool)`.
    context_mysql_pool: Option<SharedMysqlPool>,
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
            storage_config: None,
            storage: None,
            context_postgres_pool: None,
            context_mysql_pool: None,
            account_number: None,
        }
    }

    #[wasm_bindgen(js_name = "newWithSigner")]
    pub fn new_with_signer(
        config: Config,
        breez_signer: crate::signer::JsExternalBreezSigner,
        spark_signer: crate::signer::JsExternalSparkSigner,
    ) -> Self {
        use crate::signer::{WasmExternalBreezSigner, WasmExternalSparkSigner};
        use std::sync::Arc;

        let config_core: breez_sdk_spark::Config = config.into();
        let signer_adapter: Arc<dyn breez_sdk_spark::signer::ExternalBreezSigner> =
            Arc::new(WasmExternalBreezSigner::new(breez_signer));
        let spark_signer_adapter: Arc<dyn breez_sdk_spark::signer::ExternalSparkSigner> =
            Arc::new(WasmExternalSparkSigner::new(spark_signer));

        Self {
            network: config_core.network,
            seed: breez_sdk_spark::Seed::Entropy(vec![]), // Placeholder, won't be used
            builder: breez_sdk_spark::SdkBuilder::new_with_signer(
                config_core,
                signer_adapter,
                spark_signer_adapter,
            ),
            storage_config: None,
            storage: None,
            context_postgres_pool: None,
            context_mysql_pool: None,
            account_number: None,
        }
    }

    #[wasm_bindgen(js_name = "withDefaultStorage")]
    pub async fn with_default_storage(mut self, storage_dir: String) -> WasmResult<Self> {
        self.storage_config = Some(default_storage_config(storage_dir));
        Ok(self)
    }

    #[wasm_bindgen(js_name = "withStorage")]
    pub fn with_storage(mut self, storage: Storage) -> Self {
        self.storage = Some(storage);
        self
    }

    /// Sets one of the SDK's built-in storage backends.
    ///
    /// Construct the [`WasmStorageConfig`] via `defaultStorage`,
    /// `postgresStorage` or `mysqlStorage`.
    #[wasm_bindgen(js_name = "withStorageBackend")]
    pub fn with_storage_backend(mut self, config: WasmStorageConfig) -> Self {
        self.storage_config = Some(config);
        self
    }

    /// **Deprecated.** Use `withStorageBackend(postgresStorage(config))`.
    #[wasm_bindgen(js_name = "withPostgresBackend")]
    #[allow(clippy::unnecessary_wraps)]
    pub fn with_postgres_backend(self, config: PostgresStorageConfig) -> WasmResult<Self> {
        Ok(self.with_storage_backend(postgres_storage(config)))
    }

    /// **Deprecated.** Use `withStorageBackend(mysqlStorage(config))`.
    #[wasm_bindgen(js_name = "withMysqlBackend")]
    #[allow(clippy::unnecessary_wraps)]
    pub fn with_mysql_backend(self, config: MysqlStorageConfig) -> WasmResult<Self> {
        Ok(self.with_storage_backend(mysql_storage(config)))
    }

    /// Threads a shared [`WasmSdkContext`] into the builder.
    ///
    /// Construct the context once via `newSharedSdkContext` and pass the same
    /// handle to every `SdkBuilder` whose SDKs should share its resources
    /// (operator gRPC channels, SSP HTTP client, database pool).
    #[wasm_bindgen(js_name = "withSharedContext")]
    pub fn with_shared_context(mut self, context: &WasmSdkContext) -> Self {
        self.builder = self.builder.with_shared_context(context.inner.clone());
        self.context_postgres_pool = context.postgres_pool.clone();
        self.context_mysql_pool = context.mysql_pool.clone();
        self
    }

    #[wasm_bindgen(js_name = "withAccountNumber")]
    pub fn with_account_number(mut self, account_number: u32) -> Self {
        self.account_number = Some(account_number);
        self.builder = self.builder.with_account_number(account_number);
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

    #[wasm_bindgen(js_name = "build")]
    pub async fn build(mut self) -> WasmResult<BreezSdk> {
        // Derive the tenant identity from the seed. The JS-side stores use it
        // to scope every read/write by `user_id`.
        let identity_bytes = identity_public_key(
            &self.seed.to_bytes()?,
            self.network.into(),
            self.account_number,
        )
        .map_err(WasmError::new)?
        .serialize();

        let custom_storage = match (
            self.storage_config,
            self.storage,
            self.context_postgres_pool,
            self.context_mysql_pool,
        ) {
            (Some(config), None, None, None) => {
                resolve_storage_config(config, &identity_bytes, &self.network).await?
            }
            (None, Some(storage), None, None) => Arc::new(PrebuiltBackend::new(
                Arc::new(WasmStorage { storage }),
                None,
                None,
                None,
            )),
            (None, None, Some((pool_rc, run_migration, migrated)), None) => {
                build_postgres_storage_shared(&pool_rc, &identity_bytes, run_migration, &migrated)
                    .await?
            }
            (None, None, None, Some((pool_rc, run_migration, foreign_key_mode, migrated))) => {
                build_mysql_storage_shared(
                    &pool_rc,
                    &identity_bytes,
                    foreign_key_mode,
                    run_migration,
                    &migrated,
                )
                .await?
            }
            _ => {
                return Err(WasmError::new(
                    "Exactly one storage source must be set: a storage backend (default, PostgreSQL or MySQL), a custom storage, or a shared context carrying a database pool",
                ));
            }
        };

        self.builder = self.builder.with_storage_backend(custom_storage);
        let sdk = self.builder.build().await?;
        Ok(BreezSdk { sdk: Rc::new(sdk) })
    }
}

/// Resolves a [`WasmStorageConfig`] into a [`StorageBackend`], opening a fresh
/// database pool for the PostgreSQL / MySQL backends.
async fn resolve_storage_config(
    config: WasmStorageConfig,
    identity: &[u8],
    network: &breez_sdk_spark::Network,
) -> WasmResult<Arc<dyn StorageBackend>> {
    match config.kind {
        WasmStorageConfigKind::Default { storage_dir } => {
            let identity_pub_key = PublicKey::from_slice(identity).map_err(WasmError::new)?;
            let storage = Arc::new(WasmStorage {
                storage: default_storage(&storage_dir, network, &identity_pub_key).await?,
            });
            Ok(Arc::new(PrebuiltBackend::new(storage, None, None, None)))
        }
        WasmStorageConfigKind::Postgres { config } => {
            let run_migration = config.run_migration;
            let pool = create_postgres_pool(config)?;
            build_postgres_storage(&pool, identity, run_migration).await
        }
        WasmStorageConfigKind::Mysql { config } => {
            let run_migration = config.run_migration;
            let foreign_key_mode = config.foreign_key_mode;
            let pool = create_mysql_pool(config)?;
            build_mysql_storage(&pool, identity, foreign_key_mode, run_migration).await
        }
    }
}

/// Builds the Postgres stores over a context-shared `pool`, running schema
/// migrations at most once for that pool. Mirrors the native `PostgresBackend`:
/// migrations are global per database (not per tenant) and take a global lock,
/// so running them on every per-tenant build serializes builds and starves the
/// pool. The `OnceCell` runs them once; concurrent first-callers await the same
/// run, and a failure isn't cached so a later build retries.
async fn build_postgres_storage_shared(
    pool: &Rc<JsPool>,
    identity: &[u8],
    run_migration: bool,
    migrated: &Rc<OnceCell<()>>,
) -> WasmResult<Arc<dyn StorageBackend>> {
    if run_migration {
        migrated
            .get_or_try_init(|| async {
                build_postgres_storage(pool, identity, true)
                    .await
                    .map(|_| ())
            })
            .await?;
    }
    // Migrations handled once above; build the per-tenant stores migration-free.
    build_postgres_storage(pool, identity, false).await
}

/// Builds the MySQL stores over a context-shared `pool`, running schema
/// migrations at most once for that pool. See [`build_postgres_storage_shared`].
async fn build_mysql_storage_shared(
    pool: &Rc<JsPool>,
    identity: &[u8],
    foreign_key_mode: MysqlForeignKeyMode,
    run_migration: bool,
    migrated: &Rc<OnceCell<()>>,
) -> WasmResult<Arc<dyn StorageBackend>> {
    if run_migration {
        migrated
            .get_or_try_init(|| async {
                build_mysql_storage(pool, identity, foreign_key_mode, true)
                    .await
                    .map(|_| ())
            })
            .await?;
    }
    // Migrations handled once above; build the per-tenant stores migration-free.
    build_mysql_storage(pool, identity, foreign_key_mode, false).await
}

/// Builds the four PostgreSQL-backed JS stores over `pool`.
async fn build_postgres_storage(
    pool: &JsPool,
    identity: &[u8],
    run_migration: bool,
) -> WasmResult<Arc<dyn StorageBackend>> {
    let logger_ref = get_wasm_logger_ref();
    let storage = Arc::new(WasmStorage {
        storage: create_postgres_storage_with_pool(pool, identity, logger_ref, run_migration)
            .await?,
    });
    let tree_store_js =
        create_postgres_tree_store_with_pool(pool, identity, logger_ref, run_migration).await?;
    let token_store_js =
        create_postgres_token_store_with_pool(pool, identity, logger_ref, run_migration).await?;
    let session_store_js =
        create_postgres_session_store_with_pool(pool, identity, logger_ref, run_migration).await?;
    Ok(Arc::new(PrebuiltBackend::new(
        storage,
        Some(Arc::new(WasmTreeStore::new(tree_store_js))),
        Some(Arc::new(WasmTokenStore::new(token_store_js))),
        Some(Arc::new(SessionStoreAdapter::new(Arc::new(
            WasmSessionStore {
                session_store: session_store_js,
            },
        )))),
    )))
}

/// Builds the four MySQL-backed JS stores over `pool`.
async fn build_mysql_storage(
    pool: &JsPool,
    identity: &[u8],
    foreign_key_mode: MysqlForeignKeyMode,
    run_migration: bool,
) -> WasmResult<Arc<dyn StorageBackend>> {
    let logger_ref = get_wasm_logger_ref();
    let storage = Arc::new(WasmStorage {
        storage: create_mysql_storage_with_pool(pool, identity, logger_ref, run_migration).await?,
    });
    let tree_store_js = create_mysql_tree_store_with_pool(
        pool,
        identity,
        foreign_key_mode,
        logger_ref,
        run_migration,
    )
    .await?;
    let token_store_js = create_mysql_token_store_with_pool(
        pool,
        identity,
        foreign_key_mode,
        logger_ref,
        run_migration,
    )
    .await?;
    let session_store_js =
        create_mysql_session_store_with_pool(pool, identity, logger_ref, run_migration).await?;
    Ok(Arc::new(PrebuiltBackend::new(
        storage,
        Some(Arc::new(WasmTreeStore::new(tree_store_js))),
        Some(Arc::new(WasmTokenStore::new(token_store_js))),
        Some(Arc::new(SessionStoreAdapter::new(Arc::new(
            WasmSessionStore {
                session_store: session_store_js,
            },
        )))),
    )))
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
    #[wasm_bindgen(js_name = "createDefaultStorage", catch)]
    async fn create_default_storage(
        data_dir: &str,
        logger: Option<&Logger>,
    ) -> Result<crate::persist::Storage, JsValue>;
}
