use std::rc::Rc;

use wasm_bindgen::prelude::*;

use crate::{error::WasmResult, models::OptimizationProgress};

/// Sub-object for leaf optimization operations.
///
/// Access via `wallet.optimization`.
#[wasm_bindgen(js_name = "OptimizationApi")]
pub struct OptimizationApi {
    pub(crate) sdk: Rc<breez_sdk_spark::BreezSdk>,
}

#[wasm_bindgen(js_class = "OptimizationApi")]
impl OptimizationApi {
    /// Start leaf optimization in the background.
    pub fn start(&self) {
        self.sdk.start_leaf_optimization();
    }

    /// Cancel a running leaf optimization.
    pub async fn cancel(&self) -> WasmResult<()> {
        Ok(self.sdk.cancel_leaf_optimization().await?)
    }

    /// Current optimization progress (sync getter).
    #[wasm_bindgen(getter)]
    pub fn progress(&self) -> OptimizationProgress {
        self.sdk.get_leaf_optimization_progress().into()
    }
}
