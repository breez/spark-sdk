pub mod app;
#[cfg(feature = "uniffi")]
pub mod bindings;
mod chain;
mod common;
mod error;
mod events;
mod fiat_api;
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
pub use common::{fiat::*, models::*, rest::*, sync_storage};
pub use fiat_api::FiatApi;
pub use error::{DepositClaimError, SdkError, SignerError};
pub use events::{
    EventEmitter, EventListener, FilteredEventListener, LeafOptimizationEvent, OptimizationEvent,
    SdkEvent,
};
pub use issuer::*;
pub use models::*;
pub use persist::{
    PaymentMetadata, SetLnurlMetadataItem, Storage, StorageError, UpdateDepositPayload,
    path::default_storage_path,
};
pub use app::Breez;
#[allow(deprecated)] // Re-export deprecated items for backward compatibility
pub use sdk::{
    BreezClient, BreezSdk, Wallet, default_config, get_spark_status, init_logging, parse_input,
    verify_message,
};
pub use sdk::sub_objects::{
    DepositsApi, EventsApi, FiatCurrencyApi, LightningAddressApi, LnurlApi, MessageApi,
    OptimizationApi, PaymentsApi, SettingsApi, TokensApi,
};
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
#[allow(deprecated)] // Re-export deprecated items for backward compatibility
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
