use std::rc::Rc;

use wasm_bindgen::prelude::*;

use crate::{
    error::WasmResult,
    models::{GetPaymentResponse, Payment},
};

/// Sub-object for payment queries.
///
/// Access via `client.payments`.
#[wasm_bindgen(js_name = "PaymentsApi")]
pub struct PaymentsApi {
    pub(crate) sdk: Rc<breez_sdk_spark::BreezClient>,
}

#[wasm_bindgen(js_class = "PaymentsApi")]
impl PaymentsApi {
    /// List payments with optional pagination.
    ///
    /// Returns `Payment[]` directly (no wrapper object).
    pub async fn list(
        &self,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> WasmResult<Vec<Payment>> {
        let request = breez_sdk_spark::ListPaymentsRequest {
            limit,
            offset,
            ..Default::default()
        };
        let response = self.sdk.list_payments(request).await?;
        Ok(response.payments.into_iter().map(Into::into).collect())
    }

    /// Get a single payment by ID.
    ///
    /// Returns `Payment | null`.
    pub async fn get(&self, id: &str) -> WasmResult<GetPaymentResponse> {
        let request = breez_sdk_spark::GetPaymentRequest {
            payment_id: id.to_string(),
        };
        Ok(self.sdk.get_payment(request).await?.into())
    }
}
