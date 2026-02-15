use std::rc::Rc;

use wasm_bindgen::prelude::*;

use crate::{
    error::WasmResult,
    models::{SignMessageRequest, SignMessageResponse},
};

/// Sub-object for message signing operations.
///
/// Access via `wallet.message`.
///
/// Note: `verify()` (checkMessage) is available as a standalone module-level
/// function since it doesn't require wallet keys.
#[wasm_bindgen(js_name = "MessageApi")]
pub struct MessageApi {
    pub(crate) sdk: Rc<breez_sdk_spark::BreezSdk>,
}

#[wasm_bindgen(js_class = "MessageApi")]
impl MessageApi {
    /// Sign a message with the wallet's identity key.
    pub async fn sign(
        &self,
        request: SignMessageRequest,
    ) -> WasmResult<SignMessageResponse> {
        Ok(self.sdk.sign_message(request.into()).await?.into())
    }
}
