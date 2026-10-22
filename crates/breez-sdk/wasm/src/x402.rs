use std::rc::Rc;

use wasm_bindgen::prelude::wasm_bindgen;

use crate::{
    error::WasmResult,
    models::x402::{PayX402Request, PayX402Response},
};

#[wasm_bindgen]
pub struct X402 {
    pub(crate) x402: Rc<breez_sdk_spark::X402>,
}

#[wasm_bindgen]
impl X402 {
    #[wasm_bindgen(js_name = "pay")]
    pub async fn pay(&self, request: PayX402Request) -> WasmResult<PayX402Response> {
        Ok(self.x402.pay(request.into()).await?.into())
    }
}
