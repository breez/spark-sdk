#[cfg(feature = "uniffi")]
pub mod bindings;
mod chain;
mod common;
mod error;
mod events;
mod issuer;
mod jwt_header_provider;
mod lnurl;
mod logger;
mod models;
#[cfg(feature = "passkey")]
pub mod passkey;
mod persist;
mod realtime_sync;
mod sdk;
mod sdk_builder;
mod sdk_context;
mod session_manager;
pub mod signer;
mod stable_balance;
mod sync;
pub mod token_conversion;
mod utils;

pub use chain::{
    BitcoinChainService, ChainServiceError, RecommendedFees, TxStatus, Utxo,
    new_rest_chain_service,
    rest_client::{ChainApiType, RestClientChainService},
};
pub use common::rest::{RestClient, RestResponse};
pub use common::{fiat::*, models::*, sync_storage};
pub use error::{DepositClaimError, SdkError, SignerError};
pub use events::{EventEmitter, EventListener, OptimizationEvent, SdkEvent};
pub use issuer::*;
pub use models::*;
pub use persist::{
    PaymentMetadata, SetLnurlMetadataItem, Storage, StorageError, StorageListPaymentsRequest,
    StoragePaymentDetailsFilter, UpdateDepositPayload, path::default_storage_path,
};
pub use sdk::{
    BreezSdk, default_config, default_server_config, get_spark_status, init_logging, parse_input,
};
pub use sdk_builder::SdkBuilder;
pub use sdk_context::{SdkContext, SdkContextConfig, new_shared_sdk_context};
pub use session_manager::{Session, SessionManager, SessionManagerError};
pub use spark_wallet::{
    CombinedHeaderProvider, HeaderProvider, HeaderProviderError, KeySet, PublicKey,
};

#[cfg(all(
    feature = "postgres",
    not(all(target_family = "wasm", target_os = "unknown"))
))]
pub use persist::postgres::{
    PoolQueueMode, PostgresConnectionPool, PostgresStorageConfig, create_postgres_connection_pool,
    default_postgres_storage_config,
};

#[cfg(all(
    feature = "mysql",
    not(all(target_family = "wasm", target_os = "unknown"))
))]
pub use persist::mysql::{
    MysqlConnectionPool, MysqlForeignKeyMode, MysqlStorageConfig, create_mysql_connection_pool,
    default_mysql_storage_config,
};

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub use {
    persist::sqlite::SqliteStorage,
    sdk::{connect, connect_with_signer},
};

pub use sdk::default_external_signer;

#[cfg(feature = "test-utils")]
pub use persist::tests as storage_tests;

#[cfg(feature = "test-utils")]
pub use spark_wallet::tree_store_tests;

#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!();

#[allow(clippy::doc_markdown)]
pub(crate) mod built_info {
    // The file has been placed there by the build script.
    include!(concat!(env!("OUT_DIR"), "/built.rs"));
}

pub(crate) fn default_user_agent() -> String {
    format!(
        "{}/{}",
        crate::built_info::PKG_NAME,
        crate::built_info::GIT_VERSION.unwrap_or(crate::built_info::PKG_VERSION),
    )
}
