use wasm_bindgen::prelude::*;

use crate::error::WasmResult;
use crate::logger::{Logger, WASM_LOGGER};
use crate::persist::Storage;

pub(crate) fn default_storage(data_dir: &str) -> WasmResult<Storage> {
    // Get the global logger if it's been set up
    Ok(WASM_LOGGER.with_borrow(|logger| create_js_sqlite_storage(data_dir, logger.as_ref()))?)
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = "createSqliteStorage", catch)]
    fn create_js_sqlite_storage(
        data_dir: &str,
        logger: Option<&Logger>,
    ) -> Result<crate::persist::Storage, JsValue>;
}
