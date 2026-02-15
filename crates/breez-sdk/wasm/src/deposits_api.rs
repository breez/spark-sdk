use std::rc::Rc;

use wasm_bindgen::prelude::*;

use crate::{
    error::WasmResult,
    models::{
        ClaimDepositRequest, ClaimDepositResponse, DepositInfo, RefundDepositRequest,
        RefundDepositResponse,
    },
};

/// Sub-object for deposit operations.
///
/// Access via `wallet.deposits`.
#[wasm_bindgen(js_name = "DepositsApi")]
pub struct DepositsApi {
    pub(crate) sdk: Rc<breez_sdk_spark::BreezSdk>,
}

#[wasm_bindgen(js_class = "DepositsApi")]
impl DepositsApi {
    /// List unclaimed deposits.
    ///
    /// Returns `DepositInfo[]` directly (no wrapper object, no empty request param).
    #[wasm_bindgen(js_name = "listUnclaimed")]
    pub async fn list_unclaimed(&self) -> WasmResult<Vec<DepositInfo>> {
        let request = breez_sdk_spark::ListUnclaimedDepositsRequest {};
        let response = self.sdk.list_unclaimed_deposits(request).await?;
        Ok(response.deposits.into_iter().map(Into::into).collect())
    }

    /// Claim a deposit.
    pub async fn claim(
        &self,
        request: ClaimDepositRequest,
    ) -> WasmResult<ClaimDepositResponse> {
        Ok(self.sdk.claim_deposit(request.into()).await?.into())
    }

    /// Refund a deposit.
    pub async fn refund(
        &self,
        request: RefundDepositRequest,
    ) -> WasmResult<RefundDepositResponse> {
        Ok(self.sdk.refund_deposit(request.into()).await?.into())
    }
}
