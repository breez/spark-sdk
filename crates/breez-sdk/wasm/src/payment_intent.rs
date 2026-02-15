use std::rc::Rc;

use wasm_bindgen::prelude::*;

use crate::{
    error::WasmResult,
    models::{ConfirmPaymentResponse, PayOptions, PaymentIntentType, PreparedPaymentFee},
};

/// An intent to send a payment, ready to be reviewed and confirmed.
///
/// Created by `Wallet.createPayment()`. Inspect `paymentType`, `amount`,
/// and `fee` to preview the payment, then call `confirm()` to execute it.
///
/// This is the WASM equivalent of `PreparedPayment` in the core SDK.
/// The name follows the Stripe convention: a `PaymentIntent` represents
/// a payment that hasn't been executed yet.
#[wasm_bindgen(js_name = "PaymentIntent")]
pub struct PaymentIntent {
    pub(crate) inner: breez_sdk_spark::PreparedPayment<Rc<breez_sdk_spark::BreezSdk>>,
}

#[wasm_bindgen(js_class = "PaymentIntent")]
impl PaymentIntent {
    /// The type of payment: `'spark'`, `'lightning'`, or `'onchain'`.
    #[wasm_bindgen(getter, js_name = "paymentType")]
    pub fn payment_type(&self) -> PaymentIntentType {
        self.inner.payment_type().into()
    }

    /// The amount that will be sent.
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
    #[wasm_bindgen(getter, js_name = "feeSats")]
    pub fn fee_sats(&self) -> u64 {
        self.inner.fee().fee_sats()
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

    /// Confirm and execute the payment.
    #[wasm_bindgen]
    pub async fn confirm(&self, options: Option<PayOptions>) -> WasmResult<ConfirmPaymentResponse> {
        let options = options.map(Into::into);
        Ok(self.inner.confirm(options).await?.into())
    }
}

// Keep backward-compat type alias so existing imports still work
#[wasm_bindgen(typescript_custom_section)]
const PREPARED_PAYMENT_ALIAS: &str = r#"
/** @deprecated Use `PaymentIntent` instead. */
export type PreparedPayment = PaymentIntent;"#;
