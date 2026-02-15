use std::rc::Rc;

use wasm_bindgen::prelude::*;

use crate::{
    error::WasmResult,
    models::{LnurlAuthRequestDetails, LnurlCallbackStatus, LnurlWithdrawRequest, LnurlWithdrawResponse},
};

/// Sub-object for LNURL operations.
///
/// Access via `wallet.lnurl`.
///
/// Note: LNURL-Pay goes through `wallet.createPayment()` (unified payment flow),
/// not through this sub-object. This contains auth and withdraw only.
#[wasm_bindgen(js_name = "LnurlApi")]
pub struct LnurlApi {
    pub(crate) sdk: Rc<breez_sdk_spark::BreezSdk>,
}

#[wasm_bindgen(js_class = "LnurlApi")]
impl LnurlApi {
    /// Authenticate with an LNURL-Auth service.
    pub async fn auth(
        &self,
        request_data: LnurlAuthRequestDetails,
    ) -> WasmResult<LnurlCallbackStatus> {
        Ok(self.sdk.lnurl_auth(request_data.into()).await?.into())
    }

    /// Withdraw funds via LNURL-Withdraw.
    pub async fn withdraw(
        &self,
        request: LnurlWithdrawRequest,
    ) -> WasmResult<LnurlWithdrawResponse> {
        Ok(self.sdk.lnurl_withdraw(request.into()).await?.into())
    }
}
