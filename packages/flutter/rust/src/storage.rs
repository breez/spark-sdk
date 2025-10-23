use breez_sdk_common::sync::storage::SyncStorage;
use breez_sdk_spark::Storage;
pub use breez_sdk_spark::StorageImplementations;
use flutter_rust_bridge::frb;
use std::sync::Arc;

// Mirror StorageImplementations in this separate module so Dart can access the fields
// By putting it here instead of models.rs, flutter_rust_bridge won't generate
// `use crate::models::*` in frb_generated.rs, avoiding conflicts with BreezSdk/SdkBuilder ambiguity.
#[frb(mirror(StorageImplementations))]
pub struct _StorageImplementations {
    pub storage: Arc<dyn Storage>,
    pub sync_storage: Arc<dyn SyncStorage>,
}
