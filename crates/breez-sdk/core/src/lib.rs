#[cfg(feature = "uniffi")]
pub mod bindings;
mod chain;
mod common;
mod error;
mod events;
mod issuer;
mod lnurl;
mod logger;
mod models;
mod nostr;
mod persist;
mod realtime_sync;
mod sdk;
mod sdk_builder;
pub mod signer;
mod sync;
pub mod token_conversion;
mod utils;

pub use chain::{
    BitcoinChainService, ChainServiceError, RecommendedFees, TxStatus, Utxo,
    rest_client::{ChainApiType, RestClientChainService},
};
pub use common::rest::{RestClient, RestResponse};
pub use common::{fiat::*, models::*, sync_storage};
pub use error::{DepositClaimError, SdkError, SignerError};
pub use events::{EventEmitter, EventListener, OptimizationEvent, SdkEvent};
pub use issuer::*;
pub use models::*;
pub use persist::{
    PaymentMetadata, SetLnurlMetadataItem, Storage, StorageError, UpdateDepositPayload,
    path::default_storage_path,
};
pub use sdk::{BreezSdk, default_config, get_spark_status, init_logging, parse_input};
pub use sdk_builder::SdkBuilder;
pub use spark_wallet::KeySet;

#[cfg(all(
    feature = "postgres",
    not(all(target_family = "wasm", target_os = "unknown"))
))]
pub use persist::postgres::{
    PoolQueueMode, PostgresStorageConfig, create_postgres_storage, default_postgres_storage_config,
};

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub use {
    persist::sqlite::SqliteStorage,
    sdk::{connect, connect_with_signer},
};

pub use sdk::default_external_signer;

#[cfg(feature = "test-utils")]
pub use persist::tests as storage_tests;

#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!();

#[allow(clippy::doc_markdown)]
pub(crate) mod built_info {
    // The file has been placed there by the build script.
    include!(concat!(env!("OUT_DIR"), "/built.rs"));
}
