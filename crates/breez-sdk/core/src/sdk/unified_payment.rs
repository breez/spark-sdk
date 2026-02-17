use std::sync::Arc;

use tracing::warn;

use crate::{
    InputType, PrepareLnurlPayRequest, PrepareSendPaymentRequest, ReceivePaymentMethod,
    ReceivePaymentRequest,
    error::SdkError,
    models::{
        PaymentDestination, PrepareOptions, PreparedPaymentData, ReceiveOptions,
        ReceivePaymentType, ReceiveResult,
        prepared_payment::{PreparedPayment, PreparedPaymentHandle},
    },
};

use super::BreezClient;

/// Extract a raw payment-request string from a parsed [`InputType`] for use
/// with the legacy `prepare_send_payment()` API.
fn payment_request_from_input(input: &InputType) -> Option<String> {
    match input {
        InputType::SparkAddress(d) => Some(d.address.clone()),
        InputType::SparkInvoice(d) => Some(d.invoice.clone()),
        InputType::Bolt11Invoice(d) => Some(d.invoice.bolt11.clone()),
        InputType::BitcoinAddress(d) => Some(d.address.clone()),
        InputType::Bip21(d) => Some(d.uri.clone()),
        _ => None,
    }
}

// Internal implementation: returns PreparedPayment<S> which is generic and cannot be
// exported through UniFFI. Public API is `prepare_payment()` (UniFFI) and
// `preparePayment()` (WASM, which calls this internally).
#[allow(deprecated)] // New unified API delegates to legacy methods internally
impl BreezClient {
    /// Internal implementation of `prepare_payment()`.
    ///
    /// Returns a generic `PreparedPayment<Arc<BreezClient>>` that is then
    /// wrapped by `prepare_payment()` (UniFFI) or `preparePayment()` (WASM).
    ///
    /// Not part of the public API — use [`prepare_payment()`](Self::prepare_payment) instead.
    #[doc(hidden)]
    pub async fn prepare_from_destination(
        &self,
        destination: PaymentDestination,
        options: Option<PrepareOptions>,
    ) -> Result<PreparedPayment<Arc<BreezClient>>, SdkError> {
        let options = options.unwrap_or_default();

        // Resolve destination: parse if raw string, use as-is if already parsed.
        //
        // For LNURL-Pay / Lightning Address destinations, only the `Parsed` variant
        // is accepted. If a raw string resolves to LNURL, we reject with a helpful
        // error. This enforces the LUD-06 wallet flow: parse first → show metadata
        // to user → let user select amount → then call preparePayment with the
        // parsed InputType.
        //
        // We check post-parse rather than with a local heuristic because some
        // formats (like `user@domain`) may resolve to either LNURL-Pay or Bolt12
        // in the future — only the actual parse result can tell us which.
        let (parsed, was_raw) = match destination {
            PaymentDestination::Raw { destination } => {
                (self.parse(&destination).await?, true)
            }
            PaymentDestination::Parsed { input } => (input, false),
        };

        // Reject raw LNURL-Pay / Lightning Address strings (LUD-06 compliance).
        let is_lnurl = matches!(
            &parsed,
            InputType::LnurlPay(_) | InputType::LightningAddress(_)
        );
        if is_lnurl && was_raw {
            return Err(SdkError::InvalidInput(
                "LNURL-Pay and Lightning Address destinations must be parsed first using \
                 parse() / parseInput() before calling preparePayment(). This is required \
                 by the LNURL spec (LUD-06): the wallet must discover and display the \
                 service metadata (min/max sendable, description) to the user before \
                 selecting an amount. Pass the parsed InputType to preparePayment() instead \
                 of the raw string."
                    .to_string(),
            ));
        }

        // Warn if LNURL-specific options were passed for a non-LNURL destination.
        if !is_lnurl && options.lnurl.is_some() {
            warn!(
                "LnurlPayOptions provided but destination is not LNURL-Pay/Lightning Address — \
                 these options will be ignored."
            );
        }

        let data = match &parsed {
            // LNURL-Pay and Lightning Address → route through prepare_lnurl_pay
            InputType::LnurlPay(_) | InputType::LightningAddress(_) => {
                let pay_request_details = match &parsed {
                    InputType::LnurlPay(details) => details.clone(),
                    InputType::LightningAddress(la) => la.pay_request.clone(),
                    _ => unreachable!(),
                };

                if options.amount_token_units.is_some() {
                    return Err(SdkError::InvalidInput(
                        "LNURL-Pay/Lightning Address only supports amount_sats, not amount_token_units".to_string(),
                    ));
                }

                let amount_sats: u64 = options.amount_sats.ok_or(SdkError::InvalidInput(
                    "amount_sats is required for LNURL-Pay/Lightning Address".to_string(),
                ))?;

                let lnurl_opts = options.lnurl.unwrap_or_default();
                let response = self
                    .prepare_lnurl_pay(PrepareLnurlPayRequest {
                        amount_sats,
                        pay_request: pay_request_details,
                        comment: lnurl_opts.comment,
                        validate_success_action_url: lnurl_opts.validate_success_action_url,
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
                let payment_request = payment_request_from_input(&parsed).ok_or_else(|| {
                    SdkError::InvalidInput(
                        "Could not extract payment request from parsed input".to_string(),
                    )
                })?;
                let amount = options.unified_amount()?;
                let response = self
                    .prepare_send_payment(PrepareSendPaymentRequest {
                        payment_request,
                        amount,
                        token_identifier: options.token_identifier,
                        conversion_options: options.conversion_options,
                        fee_policy: options.fee_policy,
                    })
                    .await?;

                PreparedPaymentData::Standard(response)
            }

            // Bip21 URIs contain payment methods — pick the best one and prepare
            InputType::Bip21(bip21) => {
                let payment_request =
                    payment_request_from_input(&parsed).ok_or_else(|| {
                        SdkError::InvalidInput(
                            "Could not extract payment request from parsed input".to_string(),
                        )
                    })?;
                let amount = options
                    .unified_amount()?
                    .or(bip21.amount_sat.map(u128::from));
                let response = self
                    .prepare_send_payment(PrepareSendPaymentRequest {
                        payment_request,
                        amount,
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
                    "Destination type {:?} is not supported for prepare_payment(). \
                     Use lnurl_auth() or lnurl_withdraw() for those destination types.",
                    std::mem::discriminant(&parsed)
                )));
            }
        };

        // Create the Arc reference cheaply (all fields are already Arc-wrapped)
        let sdk_ref = Arc::new(self.clone());

        Ok(PreparedPayment::new(sdk_ref, data))
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
#[allow(clippy::needless_pass_by_value)]
#[allow(deprecated)] // receive() delegates to legacy receive_payment() internally
impl BreezClient {
    /// Prepare a payment to the given destination.
    ///
    /// Accepts a [`PaymentDestination`] — either a raw string (invoice, address)
    /// or an already-parsed [`InputType`] from a prior `parse()` call.
    ///
    /// **LNURL-Pay and Lightning Address** destinations **must** be parsed first
    /// using `parse()` / `parseInput()` and passed as `PaymentDestination::Parsed`.
    /// This is required by the LNURL spec ([LUD-06](https://github.com/lnurl/luds/blob/luds/06.md)):
    /// the wallet must discover and display the service metadata (min/max sendable,
    /// description, comment constraints) to the user before selecting an amount.
    /// Passing a raw LNURL/Lightning address string will return an error.
    ///
    /// For non-LNURL destinations (Bolt11 invoices, Bitcoin addresses, Spark
    /// addresses), either form is accepted — raw strings are parsed internally.
    ///
    /// Returns a [`PreparedPaymentHandle`] that can be inspected (amount, fee)
    /// and then confirmed with [`PreparedPaymentHandle::send`].
    pub async fn prepare_payment(
        &self,
        destination: PaymentDestination,
        options: Option<PrepareOptions>,
    ) -> Result<Arc<PreparedPaymentHandle>, SdkError> {
        let prepared = self.prepare_from_destination(destination, options).await?;
        Ok(Arc::new(PreparedPaymentHandle::new(prepared)))
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
        let has_token = options.token_identifier.is_some();

        let payment_method = match payment_type {
            ReceivePaymentType::Bolt11Invoice => {
                if options.amount_token_units.is_some() {
                    return Err(SdkError::InvalidInput(
                        "Bolt11Invoice receive only supports amount_sats, not amount_token_units"
                            .to_string(),
                    ));
                }
                ReceivePaymentMethod::Bolt11Invoice {
                    description: options.description.unwrap_or_default(),
                    amount_sats: options.amount_sats,
                    expiry_secs: options.expiry.map(|e| e.try_into().unwrap_or(u32::MAX)),
                }
            }
            ReceivePaymentType::BitcoinAddress => {
                if options.amount_token_units.is_some() {
                    return Err(SdkError::InvalidInput(
                        "BitcoinAddress receive only supports amount_sats, not amount_token_units"
                            .to_string(),
                    ));
                }
                ReceivePaymentMethod::BitcoinAddress
            }
            ReceivePaymentType::SparkAddress => {
                if options.amount_token_units.is_some() {
                    return Err(SdkError::InvalidInput(
                        "SparkAddress receive only supports amount_sats, not amount_token_units"
                            .to_string(),
                    ));
                }
                ReceivePaymentMethod::SparkAddress
            }
            ReceivePaymentType::SparkInvoice => {
                let amount = options.unified_amount()?;
                ReceivePaymentMethod::SparkInvoice {
                    amount,
                    token_identifier: options.token_identifier,
                    expiry_time: options.expiry,
                    description: options.description,
                    sender_public_key: options.sender_public_key,
                }
            }
        };

        let response = self
            .receive_payment(ReceivePaymentRequest { payment_method })
            .await?;

        // The legacy response.fee is dual-purpose (sats or token units).
        // Split into the appropriate field based on whether this is a token receive.
        let (fee_sats, fee_token_units) = if has_token {
            (0, Some(response.fee))
        } else {
            (response.fee as u64, None)
        };

        Ok(ReceiveResult {
            destination: response.payment_request,
            fee_sats,
            fee_token_units,
        })
    }
}
