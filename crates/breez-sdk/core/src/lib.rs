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
mod sync;
mod utils;

pub use chain::{
    BitcoinChainService, ChainServiceError, RecommendedFees, TxStatus, Utxo,
    rest_client::{ChainApiType, RestClientChainService},
};
pub use common::{fiat::*, models::*, rest::*, sync_storage};
pub use error::{DepositClaimError, SdkError};
pub use events::{EventEmitter, EventListener, SdkEvent};
pub use issuer::*;
pub use models::*;
pub use persist::{
    PaymentMetadata, SetLnurlMetadataItem, Storage, StorageError, UpdateDepositPayload,
    path::default_storage_path,
};
pub use sdk::{BreezSdk, default_config, init_logging, parse_input};
pub use sdk_builder::SdkBuilder;
pub use spark_wallet::KeySet;

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub use {persist::sqlite::SqliteStorage, sdk::connect};

#[cfg(feature = "test-utils")]
pub use persist::tests as storage_tests;

#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!();

#[allow(clippy::doc_markdown)]
pub(crate) mod built_info {
    // The file has been placed there by the build script.
    include!(concat!(env!("OUT_DIR"), "/built.rs"));
}
