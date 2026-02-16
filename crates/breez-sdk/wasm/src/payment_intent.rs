use std::rc::Rc;

use wasm_bindgen::prelude::*;

use crate::{
    error::WasmResult,
    models::{ConfirmPaymentResponse, PayOptions, PaymentIntentType, PreparedPaymentFee},
};

/// A prepared payment, ready to be reviewed and sent.
///
/// Created by `BreezClient.preparePayment()`. Inspect `paymentType`, `amountSats`/`amountTokenUnits`,
/// and `fee` to preview the payment, then call `send()` to execute it.
///
/// This is the WASM equivalent of `PreparedPayment` in the core SDK.
/// The name follows the Stripe convention: a `PaymentIntent` represents
/// a payment that hasn't been executed yet.
#[wasm_bindgen(js_name = "PaymentIntent")]
pub struct PaymentIntent {
    pub(crate) inner: breez_sdk_spark::PreparedPayment<Rc<breez_sdk_spark::BreezClient>>,
}

#[wasm_bindgen(js_class = "PaymentIntent")]
#[allow(deprecated)]
impl PaymentIntent {
    /// The type of payment: `'spark'`, `'lightning'`, or `'onchain'`.
    #[wasm_bindgen(getter, js_name = "paymentType")]
    pub fn payment_type(&self) -> PaymentIntentType {
        self.inner.payment_type().into()
    }

    /// The amount in satoshis, if this is a Bitcoin payment.
    /// Returns `undefined` for token payments.
    #[wasm_bindgen(getter, js_name = "amountSats")]
    pub fn amount_sats(&self) -> Option<u64> {
        self.inner.amount_sats()
    }

    /// The amount in token base units, if this is a token payment.
    /// Returns `undefined` for Bitcoin payments.
    #[wasm_bindgen(getter, js_name = "amountTokenUnits")]
    pub fn amount_token_units(&self) -> Option<String> {
        self.inner.amount_token_units().map(|v| v.to_string())
    }

    /// @deprecated Use `amountSats` or `amountTokenUnits` instead.
    #[wasm_bindgen(getter)]
    pub fn amount(&self) -> String {
        self.inner.amount().to_string()
    }

    /// The fee breakdown for this payment.
    #[wasm_bindgen(getter)]
    pub fn fee(&self) -> PreparedPaymentFee {
        self.inner.fee().into()
    }

    /// The fee in sats (convenience accessor).
    ///
    /// For on-chain payments, returns the medium-speed tier fee.
    /// Returns 0 for token payments — use `feeTokenUnits` instead.
    #[wasm_bindgen(getter, js_name = "feeSats")]
    pub fn fee_sats(&self) -> u64 {
        self.inner.fee().fee_sats()
    }

    /// The fee in token base units, if this is a token payment.
    /// Returns `undefined` for non-token payments.
    #[wasm_bindgen(getter, js_name = "feeTokenUnits")]
    pub fn fee_token_units(&self) -> Option<String> {
        self.inner.fee().fee_token_units().map(|v| v.to_string())
    }

    /// The token identifier, if this is a token payment.
    #[wasm_bindgen(getter, js_name = "tokenIdentifier")]
    pub fn token_identifier(&self) -> Option<String> {
        self.inner.token_identifier().map(String::from)
    }

    /// Whether this is an LNURL-Pay / Lightning Address payment.
    #[wasm_bindgen(getter, js_name = "isLnurl")]
    pub fn is_lnurl(&self) -> bool {
        self.inner.is_lnurl()
    }

    /// Execute the payment.
    #[wasm_bindgen]
    pub async fn send(&self, options: Option<PayOptions>) -> WasmResult<ConfirmPaymentResponse> {
        let options = options.map(Into::into);
        Ok(self.inner.send(options).await?.into())
    }

}

