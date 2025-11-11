#[cfg(feature = "uniffi")]
pub mod bindings;
mod chain;
mod common;
mod error;
mod events;
mod lnurl;
mod logger;
mod models;
mod persist;
mod realtime_sync;
mod sdk;
mod sdk_builder;
mod sync;
mod utils;

pub use chain::{
    BitcoinChainService, ChainServiceError, TxStatus, Utxo, rest_client::RestClientChainService,
};
pub use common::{fiat::*, models::*, rest::*, sync_storage};
pub use error::{DepositClaimError, SdkError};
pub use events::{EventEmitter, EventListener, SdkEvent};
pub use models::*;
pub use persist::{
    PaymentMetadata, Storage, StorageError, UpdateDepositPayload, path::default_storage_path,
};
pub use sdk::{BreezSdk, default_config, init_logging, parse_input};
pub use sdk_builder::SdkBuilder;

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub use {persist::sqlite::SqliteStorage, sdk::connect};

#[cfg(feature = "test-utils")]
pub use persist::tests as storage_tests;

#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!();
