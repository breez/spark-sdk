use std::rc::Rc;

use wasm_bindgen::prelude::wasm_bindgen;

use crate::{
    error::WasmResult,
    models::x402::{
        CreateX402ChallengeRequest, CreateX402ChallengeResponse, PayX402Request, PayX402Response,
        SettleX402Request, SettleX402Response,
    },
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

    #[wasm_bindgen(js_name = "createChallenge")]
    pub async fn create_challenge(
        &self,
        request: CreateX402ChallengeRequest,
    ) -> WasmResult<CreateX402ChallengeResponse> {
        Ok(self.x402.create_challenge(request.into()).await?.into())
    }

    #[wasm_bindgen(js_name = "settle")]
    pub async fn settle(&self, request: SettleX402Request) -> WasmResult<SettleX402Response> {
        Ok(self.x402.settle(request.into()).await?.into())
    }
}
