use breez_sdk_core::Storage;

use crate::error::WasmResult;

pub(crate) fn default_storage(data_dir: &str) -> WasmResult<Box<dyn Storage>> {
    Ok(Box::new(IndexedDbStorage {}))
}

pub(crate) struct IndexedDbStorage {}
