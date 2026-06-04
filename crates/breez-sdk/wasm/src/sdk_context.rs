//! WASM wrapper around [`breez_sdk_spark::SdkContext`].
//!
//! For Postgres, the context carries an `Arc<dyn StorageBackend>` built
//! from `breez_sdk_spark::postgres_storage`. That backend is the same
//! type the native SDK uses; on wasm it routes through `spark-postgres`
//! and `pg-wasm` to node-postgres. The backend's internal `OnceCell`
//! migration guard is what every SDK built from this context shares,
//! mirroring the native multi-tenant flow.
//!
//! For MySQL, the context still holds a JS-side pool tuple — that
//! backend has not yet been migrated.

use std::rc::Rc;
use std::sync::Arc;

#[cfg(feature = "postgres")]
use breez_sdk_spark::StorageBackend;
use platform_utils::tokio::sync::OnceCell;
use wasm_bindgen::prelude::*;

#[cfg(feature = "postgres")]
use crate::error::WasmError;
use crate::{
    error::WasmResult,
    models::Network,
    persist::pool::{JsPool, create_mysql_pool},
    sdk_builder::{MysqlForeignKeyMode, MysqlStorageConfig},
};
#[cfg(feature = "postgres")]
use crate::sdk_builder::PostgresStorageConfig;

/// A context-shared MySQL pool: the JS pool, its `run_migration` flag, the
/// foreign-key mode the stores were configured with, and a once-guard that
/// limits schema migrations to a single run per pool.
///
/// The guard exists because every SDK built from the context reuses the same
/// pool and (pre-fix) re-ran the four stores' migrations on each build, each
/// taking a *global* migration lock while holding a connection — serializing
/// every build across every tenant. Shared via `Rc` so it stays the same guard
/// after the context's pool is cloned into each `SdkBuilder`.
pub(crate) type SharedMysqlPool = (Rc<JsPool>, bool, MysqlForeignKeyMode, Rc<OnceCell<()>>);

/// Process-shared resources backing one or more `BreezSdk` instances on WASM.
///
/// Construct once via `newSharedSdkContext` and pass the handle to every
/// `SdkBuilder` whose SDKs should share its operator gRPC channels, SSP HTTP
/// client, and (optionally) database connection pool.
#[wasm_bindgen]
pub struct WasmSdkContext {
    pub(crate) inner: Arc<breez_sdk_spark::SdkContext>,
    /// Shared Postgres backend (a `PostgresBackend` produced by
    /// `breez_sdk_spark::postgres_storage`). Encapsulates the pool plus
    /// the once-cell that gates schema migrations across all SDKs built
    /// from this context. Only present when the wasm SDK is built with
    /// `feature = "postgres"`.
    #[cfg(feature = "postgres")]
    pub(crate) postgres_backend: Option<Arc<dyn StorageBackend>>,
    pub(crate) mysql_pool: Option<SharedMysqlPool>,
}

/// Settings for `newSharedSdkContext`. `network` is required; all other
/// fields are optional.
#[derive(Clone, serde::Serialize, serde::Deserialize, tsify_next::Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
#[serde(rename_all = "camelCase")]
pub struct WasmSdkContextConfig {
    /// Network the shared resources target. Used to gate the partner JWT
    /// header provider — only constructed on Mainnet.
    pub network: Network,

    /// Breez API key. When set together with `network == Mainnet`, the
    /// context constructs a shared partner JWT header provider that all
    /// SDKs built from this context will attach to their SO requests.
    #[tsify(optional)]
    pub api_key: Option<String>,

    /// Number of gRPC connections per Spark operator. `None` (or `Some(1)`)
    /// keeps a single connection per operator (right for most deployments);
    /// `Some(n)` opens `n` channels per operator and balances requests.
    #[tsify(optional)]
    pub connections_per_operator: Option<u32>,

    /// PostgreSQL backend configuration. When set, SDKs constructed with
    /// this context store their data in PostgreSQL via the shared pool.
    /// Only honoured when the wasm SDK is built with `feature = "postgres"`.
    #[cfg(feature = "postgres")]
    #[tsify(optional)]
    pub postgres_config: Option<PostgresStorageConfig>,

    /// MySQL backend configuration. When set, SDKs constructed with this
    /// context store their data in MySQL via the shared pool.
    #[tsify(optional)]
    pub mysql_config: Option<MysqlStorageConfig>,
}

/// Constructs a [`WasmSdkContext`] from a `WasmSdkContextConfig`.
#[wasm_bindgen(js_name = "newSharedSdkContext")]
pub async fn new_shared_sdk_context(config: WasmSdkContextConfig) -> WasmResult<WasmSdkContext> {
    // WASM storage is JS-backed and threaded through `WasmSdkContext` below, so
    // the core context carries no storage.
    let inner = breez_sdk_spark::new_shared_sdk_context(breez_sdk_spark::SdkContextConfig {
        network: config.network.into(),
        api_key: config.api_key,
        connections_per_operator: config.connections_per_operator,
        storage: None,
    })
    .await?;

    #[cfg(feature = "postgres")]
    let postgres_backend = match config.postgres_config {
        Some(cfg) => Some(
            breez_sdk_spark::postgres_storage(cfg.into()).map_err(WasmError::new)?,
        ),
        None => None,
    };

    let mysql_pool = match config.mysql_config {
        Some(cfg) => {
            let run_migration = cfg.run_migration;
            let foreign_key_mode = cfg.foreign_key_mode;
            Some((
                Rc::new(create_mysql_pool(cfg)?),
                run_migration,
                foreign_key_mode,
                Rc::new(OnceCell::new()),
            ))
        }
        None => None,
    };

    Ok(WasmSdkContext {
        inner,
        #[cfg(feature = "postgres")]
        postgres_backend,
        mysql_pool,
    })
}
