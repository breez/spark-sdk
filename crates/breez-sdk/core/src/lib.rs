#[cfg(feature = "uniffi")]
mod bindings;
mod chain;
mod error;
mod events;
mod logger;
mod models;
mod persist;
mod sdk;
mod sdk_builder;
mod utils;

#[cfg(feature = "uniffi")]
pub use bindings::*;
pub use breez_sdk_common::input::{InputType, ParseError, parse};
pub use chain::{BitcoinChainService, ChainServiceError, rest_client::RestClientChainService};
pub use error::{DepositClaimError, SdkError};
pub use events::{EventEmitter, EventListener, SdkEvent};
pub use models::*;
pub use persist::{PaymentMetadata, Storage, StorageError, UpdateDepositPayload};
pub use sdk::{BreezSdk, default_config, init_logging, parse_input};
#[cfg(not(feature = "uniffi"))]
pub use sdk_builder::SdkBuilder;

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub use {persist::sqlite::SqliteStorage, sdk::default_storage};

#[cfg(feature = "test-utils")]
pub use persist::tests as storage_tests;

#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!();
