use std::sync::Arc;

use crate::{
    InputType, PrepareLnurlPayRequest, PrepareSendPaymentRequest, ReceivePaymentMethod,
    ReceivePaymentRequest,
    error::SdkError,
    models::{
        PayOptions, PrepareOptions, PreparedPaymentData, ReceiveOptions, ReceivePaymentType,
        ReceiveResult,
        prepared_payment::{ConfirmPaymentResponse, PreparedPayment, PreparedPaymentHandle},
    },
};

use super::BreezSdk;

// prepare() and pay() return PreparedPayment<S> which is generic and cannot be
// exported through UniFFI.  WASM bindings wrap these in crates/breez-sdk/wasm/src/sdk.rs.
// UniFFI bindings will be added in Phase 6 (language binding idiomaticity).
#[allow(deprecated)] // New unified API delegates to legacy methods internally
impl BreezSdk {
    /// Parse the destination and prepare a payment in one step.
    ///
    /// This is the main entry point for the unified payment API.
    /// It accepts any destination string (Spark address, Spark invoice,
    /// Bolt11 invoice, Bitcoin address, LNURL-Pay URL, Lightning address)
    /// and returns a `PreparedPayment` that can be inspected (amount, fee)
    /// and then confirmed.
    ///
    /// # Example (Rust)
    /// ```ignore
    /// let prepared = sdk.prepare("lnbc1...", None).await?;
    /// println!("Fee: {:?}", prepared.fee());
    /// let result = prepared.confirm(None).await?;
    /// ```
    pub async fn prepare(
        &self,
        destination: &str,
        options: Option<PrepareOptions>,
    ) -> Result<PreparedPayment<Arc<BreezSdk>>, SdkError> {
        let options = options.unwrap_or_default();
        let parsed = self.parse(destination).await?;

        let data = match &parsed {
            // LNURL-Pay and Lightning Address → route through prepare_lnurl_pay
            InputType::LnurlPay(_) | InputType::LightningAddress(_) => {
                let pay_request_details = match &parsed {
                    InputType::LnurlPay(details) => details.clone(),
                    InputType::LightningAddress(la) => la.pay_request.clone(),
                    _ => unreachable!(),
                };

                let amount_sats: u64 = options
                    .amount
                    .ok_or(SdkError::InvalidInput(
                        "Amount is required for LNURL-Pay/Lightning Address".to_string(),
                    ))?
                    .try_into()
                    .map_err(|_| SdkError::InvalidInput("Amount too large".to_string()))?;

                let response = self
                    .prepare_lnurl_pay(PrepareLnurlPayRequest {
                        amount_sats,
                        pay_request: pay_request_details,
                        comment: options.lnurl_comment,
                        validate_success_action_url: options.lnurl_validate_success_action_url,
                        conversion_options: options.conversion_options,
                        fee_policy: options.fee_policy,
                    })
                    .await?;

                PreparedPaymentData::Lnurl(response)
            }

            // All other sendable destinations → route through prepare_send_payment
            InputType::SparkAddress(_)
            | InputType::SparkInvoice(_)
            | InputType::Bolt11Invoice(_)
            | InputType::BitcoinAddress(_) => {
                let response = self
                    .prepare_send_payment(PrepareSendPaymentRequest {
                        payment_request: destination.to_string(),
                        amount: options.amount,
                        token_identifier: options.token_identifier,
                        conversion_options: options.conversion_options,
                        fee_policy: options.fee_policy,
                    })
                    .await?;

                PreparedPaymentData::Standard(response)
            }

            // Bip21 URIs contain payment methods — pick the best one and prepare
            InputType::Bip21(bip21) => {
                // Use the raw destination so prepare_send_payment can re-parse and extract
                // the best payment method from the BIP21 URI.
                let response = self
                    .prepare_send_payment(PrepareSendPaymentRequest {
                        payment_request: destination.to_string(),
                        amount: options.amount.or(bip21.amount_sat.map(u128::from)),
                        token_identifier: options.token_identifier,
                        conversion_options: options.conversion_options,
                        fee_policy: options.fee_policy,
                    })
                    .await?;

                PreparedPaymentData::Standard(response)
            }

            // Unsupported destinations
            _ => {
                return Err(SdkError::InvalidInput(format!(
                    "Destination type {:?} is not supported for prepare(). \
                     Use lnurl_auth() or lnurl_withdraw() for those destination types.",
                    std::mem::discriminant(&parsed)
                )));
            }
        };

        // Create the Arc reference cheaply (all fields are already Arc-wrapped)
        let sdk_ref = Arc::new(self.clone());

        Ok(PreparedPayment::new(sdk_ref, data))
    }

    /// Parse, prepare, and execute a payment in one step.
    ///
    /// This is the simplest way to send a payment — a one-liner for callers
    /// who don't need to preview fees before confirming.
    ///
    /// # Example (Rust)
    /// ```ignore
    /// let result = sdk.pay("lnbc1...", None, None).await?;
    /// ```
    pub async fn pay(
        &self,
        destination: &str,
        prepare_options: Option<PrepareOptions>,
        pay_options: Option<PayOptions>,
    ) -> Result<ConfirmPaymentResponse, SdkError> {
        let prepared = self.prepare(destination, prepare_options).await?;
        prepared.confirm(pay_options).await
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
#[allow(clippy::needless_pass_by_value)]
#[allow(deprecated)] // receive() delegates to legacy receive_payment() internally
impl BreezSdk {
    /// Parse the destination and prepare a payment in one step.
    ///
    /// Returns a [`PreparedPaymentHandle`] that can be inspected (amount, fee)
    /// and then confirmed with [`PreparedPaymentHandle::confirm`].
    ///
    /// This is the UniFFI-exported version of [`prepare`](Self::prepare).
    pub async fn prepare_payment(
        &self,
        destination: String,
        options: Option<PrepareOptions>,
    ) -> Result<Arc<PreparedPaymentHandle>, SdkError> {
        let prepared = self.prepare(&destination, options).await?;
        Ok(Arc::new(PreparedPaymentHandle::new(prepared)))
    }

    /// Parse, prepare, and execute a payment in one step.
    ///
    /// This is the UniFFI-exported version of [`pay`](Self::pay).
    pub async fn pay_to_destination(
        &self,
        destination: String,
        prepare_options: Option<PrepareOptions>,
        pay_options: Option<PayOptions>,
    ) -> Result<ConfirmPaymentResponse, SdkError> {
        self.pay(&destination, prepare_options, pay_options).await
    }

    /// Generate a payment request (invoice, address) to receive funds.
    ///
    /// This is a simplified version of `receive_payment` that uses a flat
    /// options struct instead of nested enum variants.
    ///
    /// # Example (Rust)
    /// ```ignore
    /// // Receive 1000 sats via Lightning
    /// let result = sdk.receive(ReceiveOptions {
    ///     amount: Some(1_000),
    ///     description: Some("Coffee".into()),
    ///     ..Default::default()
    /// }).await?;
    /// println!("Invoice: {}", result.destination);
    /// ```
    pub async fn receive(&self, options: ReceiveOptions) -> Result<ReceiveResult, SdkError> {
        let payment_type = options.payment_type.unwrap_or_default();

        let payment_method = match payment_type {
            ReceivePaymentType::Lightning => {
                let amount_sats: Option<u64> = options
                    .amount
                    .map(|a| {
                        a.try_into()
                            .map_err(|_| SdkError::InvalidInput("Amount too large".to_string()))
                    })
                    .transpose()?;
                ReceivePaymentMethod::Bolt11Invoice {
                    description: options.description.unwrap_or_default(),
                    amount_sats,
                    expiry_secs: options.expiry.map(|e| e.try_into().unwrap_or(u32::MAX)),
                }
            }
            ReceivePaymentType::Onchain => ReceivePaymentMethod::BitcoinAddress,
            ReceivePaymentType::SparkAddress => ReceivePaymentMethod::SparkAddress,
            ReceivePaymentType::SparkInvoice => ReceivePaymentMethod::SparkInvoice {
                amount: options.amount,
                token_identifier: options.token_identifier,
                expiry_time: options.expiry,
                description: options.description,
                sender_public_key: options.sender_public_key,
            },
        };

        let response = self
            .receive_payment(ReceivePaymentRequest { payment_method })
            .await?;

        Ok(ReceiveResult {
            destination: response.payment_request,
            fee: response.fee,
        })
    }
}
