//! WASM wrapper around [`breez_sdk_spark::SdkContext`] plus the JS-side
//! pools (postgres / mysql) that WASM uses in place of the native Rust pools.

use std::rc::Rc;
use std::sync::Arc;

use wasm_bindgen::prelude::*;

use crate::{
    error::WasmResult,
    persist::pool::{JsPool, create_mysql_pool, create_postgres_pool},
    sdk_builder::{MysqlForeignKeyMode, MysqlStorageConfig, PostgresStorageConfig},
};

/// Process-shared resources backing one or more `BreezSdk` instances on WASM.
///
/// Construct once via `newSharedSdkContext` and pass the handle to every
/// `SdkBuilder` whose SDKs should share its operator gRPC channels, SSP HTTP
/// client, and (optionally) database connection pool.
#[wasm_bindgen]
pub struct WasmSdkContext {
    pub(crate) inner: Arc<breez_sdk_spark::SdkContext>,
    pub(crate) postgres_pool: Option<(Rc<JsPool>, bool)>,
    pub(crate) mysql_pool: Option<(Rc<JsPool>, bool, MysqlForeignKeyMode)>,
}

/// Settings for `newSharedSdkContext`. Fields are optional with sensible defaults.
#[derive(Default, Clone, serde::Serialize, serde::Deserialize, tsify_next::Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
#[serde(rename_all = "camelCase")]
pub struct WasmSdkContextConfig {
    /// Number of gRPC connections per Spark operator. `None` (or `Some(1)`)
    /// keeps a single connection per operator (right for most deployments);
    /// `Some(n)` opens `n` channels per operator and balances requests.
    #[tsify(optional)]
    pub connections_per_operator: Option<u32>,

    /// PostgreSQL backend configuration. When set, SDKs constructed with
    /// this context store their data in PostgreSQL via the shared pool.
    #[tsify(optional)]
    pub postgres_config: Option<PostgresStorageConfig>,

    /// MySQL backend configuration. When set, SDKs constructed with this
    /// context store their data in MySQL via the shared pool.
    #[tsify(optional)]
    pub mysql_config: Option<MysqlStorageConfig>,
}

/// Constructs a [`WasmSdkContext`] from a `WasmSdkContextConfig`.
#[wasm_bindgen(js_name = "newSharedSdkContext")]
pub fn new_shared_sdk_context(config: WasmSdkContextConfig) -> WasmResult<WasmSdkContext> {
    let inner = breez_sdk_spark::new_shared_sdk_context(breez_sdk_spark::SdkContextConfig {
        connections_per_operator: config.connections_per_operator,
    })?;

    let postgres_pool = match config.postgres_config {
        Some(cfg) => {
            let run_migration = cfg.run_migration;
            Some((Rc::new(create_postgres_pool(cfg)?), run_migration))
        }
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
            ))
        }
        None => None,
    };

    Ok(WasmSdkContext {
        inner,
        postgres_pool,
        mysql_pool,
    })
}
