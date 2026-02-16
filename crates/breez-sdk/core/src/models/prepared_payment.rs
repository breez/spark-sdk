use crate::{
    LnurlPayRequest, Payment, SendPaymentRequest, SuccessActionProcessed,
    error::SdkError,
    models::{
        PayOptions, PrepareLnurlPayResponse, PrepareSendPaymentResponse, PreparedPaymentData,
        PreparedPaymentFee, SendPaymentMethod,
    },
};
use serde::Serialize;
use std::ops::Deref;
use std::sync::Arc;

/// The type of payment, determined from the prepared payment data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum PaymentIntentType {
    /// A Spark transfer (address or invoice).
    Spark,
    /// A Lightning payment (Bolt11 invoice or LNURL-Pay).
    Lightning,
    /// An on-chain Bitcoin address payment.
    Onchain,
}

/// A payment that has been prepared and is ready to be confirmed.
///
/// Created by [`BreezClient::prepare`](crate::BreezClient::prepare). Holds a reference
/// back to the SDK so that the caller can simply call [`confirm`](Self::confirm)
/// to execute the payment.
///
/// The generic parameter `S` is the smart-pointer type wrapping [`BreezClient`](crate::BreezClient):
/// - `Arc<BreezClient>` for native (`UniFFI`) bindings
/// - `Rc<BreezClient>` for WASM bindings
///
/// Most consumers will never see the generic parameter since the SDK's
/// `prepare()` method returns a concrete `PreparedPayment<Arc<BreezClient>>`.
pub struct PreparedPayment<S>
where
    S: Deref<Target = crate::BreezClient> + Clone,
{
    sdk: S,
    data: PreparedPaymentData,
}

impl<S> PreparedPayment<S>
where
    S: Deref<Target = crate::BreezClient> + Clone,
{
    /// Create a new `PreparedPayment` from an SDK reference and prepared data.
    pub fn new(sdk: S, data: PreparedPaymentData) -> Self {
        Self { sdk, data }
    }

    /// Decompose into the inner SDK reference and prepared data.
    /// Useful for re-wrapping with a different pointer type (e.g., `Rc` for WASM).
    pub fn into_parts(self) -> (S, PreparedPaymentData) {
        (self.sdk, self.data)
    }
}

// Manual Debug to avoid S: Debug bound
impl<S> std::fmt::Debug for PreparedPayment<S>
where
    S: Deref<Target = crate::BreezClient> + Clone,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedPayment")
            .field("data", &self.data)
            .finish_non_exhaustive()
    }
}

/// The result of confirming a prepared payment.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ConfirmPaymentResponse {
    pub payment: Payment,
    /// Set only for LNURL-Pay payments that have a success action.
    pub success_action: Option<SuccessActionProcessed>,
}

impl<S> PreparedPayment<S>
where
    S: Deref<Target = crate::BreezClient> + Clone,
{
    /// The type of payment (Spark, Lightning, or Onchain).
    ///
    /// Determined from the prepared payment data so that callers don't need to
    /// inspect the fee variant or cross-reference the original input type.
    pub fn payment_type(&self) -> PaymentIntentType {
        match &self.data {
            PreparedPaymentData::Standard(resp) => match &resp.payment_method {
                SendPaymentMethod::BitcoinAddress { .. } => PaymentIntentType::Onchain,
                SendPaymentMethod::Bolt11Invoice { .. } => PaymentIntentType::Lightning,
                SendPaymentMethod::SparkAddress { .. }
                | SendPaymentMethod::SparkInvoice { .. } => PaymentIntentType::Spark,
            },
            PreparedPaymentData::Lnurl(_) => PaymentIntentType::Lightning,
        }
    }

    /// The amount that will be sent, in satoshis.
    ///
    /// Returns `None` for token payments — use [`amount_token_units()`](Self::amount_token_units) instead.
    pub fn amount_sats(&self) -> Option<u64> {
        if self.token_identifier().is_some() {
            return None;
        }
        match &self.data {
            PreparedPaymentData::Standard(resp) => Some(resp.amount as u64),
            PreparedPaymentData::Lnurl(resp) => Some(resp.amount_sats),
        }
    }

    /// The amount that will be sent, in token base units.
    ///
    /// Returns `None` for non-token (Bitcoin) payments — use [`amount_sats()`](Self::amount_sats) instead.
    pub fn amount_token_units(&self) -> Option<u128> {
        if self.token_identifier().is_none() {
            return None;
        }
        match &self.data {
            PreparedPaymentData::Standard(resp) => Some(resp.amount),
            PreparedPaymentData::Lnurl(_) => None, // LNURL is never a token payment
        }
    }

    /// The amount that will be sent.
    /// Denominated in satoshis for Bitcoin payments, or token base units for token payments.
    #[deprecated(note = "Use `amount_sats()` or `amount_token_units()` instead.")]
    pub fn amount(&self) -> u128 {
        match &self.data {
            PreparedPaymentData::Standard(resp) => resp.amount,
            PreparedPaymentData::Lnurl(resp) => u128::from(resp.amount_sats),
        }
    }

    /// The fee breakdown for this payment.
    pub fn fee(&self) -> PreparedPaymentFee {
        match &self.data {
            PreparedPaymentData::Standard(resp) => {
                PreparedPaymentFee::from_send_payment_method(&resp.payment_method)
            }
            PreparedPaymentData::Lnurl(resp) => PreparedPaymentFee::from_lnurl_prepare(resp),
        }
    }

    /// The token identifier, if this is a token payment.
    pub fn token_identifier(&self) -> Option<&str> {
        match &self.data {
            PreparedPaymentData::Standard(resp) => resp.token_identifier.as_deref(),
            PreparedPaymentData::Lnurl(_) => None,
        }
    }

    /// Returns `true` if this is an LNURL-Pay / Lightning Address payment.
    pub fn is_lnurl(&self) -> bool {
        matches!(self.data, PreparedPaymentData::Lnurl(_))
    }

    /// Access the underlying standard prepare response, if applicable.
    pub fn standard_response(&self) -> Option<&PrepareSendPaymentResponse> {
        match &self.data {
            PreparedPaymentData::Standard(resp) => Some(resp),
            PreparedPaymentData::Lnurl(_) => None,
        }
    }

    /// Access the underlying LNURL prepare response, if applicable.
    pub fn lnurl_response(&self) -> Option<&PrepareLnurlPayResponse> {
        match &self.data {
            PreparedPaymentData::Standard(_) => None,
            PreparedPaymentData::Lnurl(resp) => Some(resp),
        }
    }

    /// Send the prepared payment.
    ///
    /// This is the single method callers need after `prepare()`:
    /// ```ignore
    /// let prepared = client.prepare("lnbc1...", None).await?;
    /// println!("Fee: {:?}", prepared.fee());
    /// let result = prepared.send(None).await?;
    /// ```
    #[allow(deprecated)] // Delegates to legacy methods internally
    pub async fn send(
        &self,
        options: Option<PayOptions>,
    ) -> Result<ConfirmPaymentResponse, SdkError> {
        let options = options.unwrap_or_default();

        match &self.data {
            PreparedPaymentData::Standard(prepare_response) => {
                let response = self
                    .sdk
                    .send_payment(SendPaymentRequest {
                        prepare_response: prepare_response.clone(),
                        options: options.send_options,
                        idempotency_key: options.idempotency_key,
                    })
                    .await?;
                Ok(ConfirmPaymentResponse {
                    payment: response.payment,
                    success_action: None,
                })
            }
            PreparedPaymentData::Lnurl(prepare_response) => {
                let response = self
                    .sdk
                    .lnurl_pay(LnurlPayRequest {
                        prepare_response: prepare_response.clone(),
                        idempotency_key: options.idempotency_key,
                    })
                    .await?;
                Ok(ConfirmPaymentResponse {
                    payment: response.payment,
                    success_action: response.success_action,
                })
            }
        }
    }

}

// ---------------------------------------------------------------------------
// UniFFI-compatible concrete handle type
// ---------------------------------------------------------------------------

/// Concrete handle for `PreparedPayment` used by UniFFI language bindings
/// (Kotlin, Swift, Python, C#, Go).
///
/// UniFFI cannot export generic types, so this wraps
/// `PreparedPayment<Arc<BreezClient>>` with a concrete Object type.
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct PreparedPaymentHandle {
    inner: PreparedPayment<Arc<crate::BreezClient>>,
}

impl PreparedPaymentHandle {
    /// Wrap a generic `PreparedPayment` into a UniFFI-compatible handle.
    pub fn new(inner: PreparedPayment<Arc<crate::BreezClient>>) -> Self {
        Self { inner }
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
#[allow(deprecated)]
impl PreparedPaymentHandle {
    /// The type of payment (Spark, Lightning, or Onchain).
    pub fn payment_type(&self) -> PaymentIntentType {
        self.inner.payment_type()
    }

    /// The amount that will be sent, in satoshis.
    /// Returns `None` for token payments.
    pub fn amount_sats(&self) -> Option<u64> {
        self.inner.amount_sats()
    }

    /// The amount that will be sent, in token base units.
    /// Returns `None` for non-token (Bitcoin) payments.
    pub fn amount_token_units(&self) -> Option<u128> {
        self.inner.amount_token_units()
    }

    /// The amount that will be sent.
    #[deprecated(note = "Use `amount_sats()` or `amount_token_units()` instead.")]
    pub fn amount(&self) -> u128 {
        self.inner.amount()
    }

    /// The fee breakdown for this payment.
    pub fn fee(&self) -> PreparedPaymentFee {
        self.inner.fee()
    }

    /// The token identifier, if this is a token payment.
    pub fn token_identifier(&self) -> Option<String> {
        self.inner.token_identifier().map(String::from)
    }

    /// Whether this is an LNURL-Pay / Lightning Address payment.
    pub fn is_lnurl(&self) -> bool {
        self.inner.is_lnurl()
    }

    /// Send the prepared payment.
    pub async fn send(
        &self,
        options: Option<PayOptions>,
    ) -> Result<ConfirmPaymentResponse, SdkError> {
        self.inner.send(options).await
    }

}
