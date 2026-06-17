#[cfg(feature = "uniffi")]
pub mod bindings;
mod chain;
mod common;
mod cross_chain;
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
mod session_store;
pub mod signer;
mod stable_balance;
mod sync;
pub mod token_conversion;
#[cfg(feature = "turnkey")]
pub mod turnkey;
mod utils;

pub use chain::{
    BitcoinChainService, ChainServiceError, RecommendedFees, TxStatus, Utxo,
    new_rest_chain_service,
    rest_client::{ChainApiType, RestClientChainService},
};
pub use common::rest::{RestClient, RestResponse};
pub use common::{fiat::*, models::*, sync_storage};
pub use cross_chain::{
    CrossChainFeeMode, CrossChainProvider, CrossChainProviderContext, CrossChainRouteFilter,
    CrossChainRoutePair, SourceAsset,
};
pub use error::{DepositClaimError, SdkError, SignerError};
pub use events::{AutoOptimizationEvent, EventEmitter, EventListener, SdkEvent};
pub use issuer::*;
pub use logger::DEFAULT_FILTER;
pub use models::*;
pub use persist::{
    ConversionFilter, PaymentMetadata, SetLnurlMetadataItem, Storage, StorageError,
    StorageListPaymentsRequest, StoragePaymentDetailsFilter, UpdateDepositPayload,
    backend::{PrebuiltBackend, ResolvedStores, StorageBackend, custom_storage},
    path::default_storage_path,
};
pub use sdk::{
    BreezSdk, default_config, default_server_config, get_spark_status, init_logging, parse_input,
};
pub use sdk_builder::SdkBuilder;
pub use sdk_context::{SdkContext, SdkContextConfig, new_shared_sdk_context};
pub use session_store::{Session, SessionStore, SessionStoreAdapter, SessionStoreError};
pub use spark_wallet::{
    CombinedHeaderProvider, HeaderProvider, HeaderProviderError, PublicKey, account_master_key,
    identity_master_key, identity_public_key,
};

#[cfg(feature = "postgres")]
pub use persist::{
    backend::postgres_storage,
    postgres::{PoolQueueMode, PostgresStorageConfig, default_postgres_storage_config},
};

#[cfg(feature = "mysql")]
pub use persist::{
    backend::mysql_storage,
    mysql::{MysqlForeignKeyMode, MysqlStorageConfig, default_mysql_storage_config},
};

#[cfg(feature = "sqlite")]
pub use {
    persist::{backend::default_storage, sqlite::SqliteStorage},
    sdk::{connect, connect_with_signer},
};

pub use sdk::{ExternalSigners, default_external_signers};

#[cfg(feature = "test-utils")]
pub use persist::tests as storage_tests;

#[cfg(feature = "test-utils")]
pub use spark_wallet::tree_store_tests;

#[cfg(feature = "test-utils")]
pub use spark_wallet::token_store_tests;

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
