use breez_sdk_common::lnurl::{
    error::LnurlError,
    pay::{CallbackResponse, ValidatedCallbackResponse, validate_lnurl_pay},
};
use tracing::info;

use crate::{
    ConversionEstimate, ConversionType, FeePolicy, InputType, LnurlPayInfo, LnurlPayRequest,
    LnurlPayResponse, PrepareLnurlPayRequest, PrepareLnurlPayResponse, SendPaymentMethod,
    error::SdkError,
    events::SdkEvent,
    models::{PrepareSendPaymentResponse, SendPaymentRequest},
    persist::PaymentMetadata,
    sdk::{
        BreezSdk,
        helpers::process_success_action,
        payments::{conversion, send, validation},
    },
};

/// Validates an LNURL pay request and returns the (possibly upgraded) fee policy.
///
/// Synchronous, input-only checks. Async checks (LNURL min/max sendable, post-conversion
/// dust, converter availability) stay in the prepare flow.
fn validate_request(request: &PrepareLnurlPayRequest) -> Result<(), SdkError> {
    validation::validate_amount(Some(request.amount))?;
    validation::validate_fee_policy_for_conversion(
        request.fee_policy,
        request.conversion_options.as_ref(),
    )?;

    // Token-denominated LNURL pay (`token_identifier` set) requires
    // `FeesIncluded`: the conversion output is the only sat budget, so fees
    // must come out of it.
    if request.token_identifier.is_some() && request.fee_policy != Some(FeePolicy::FeesIncluded) {
        return Err(SdkError::InvalidInput(
            "Token conversion with token_identifier requires FeesIncluded fee policy".to_string(),
        ));
    }

    Ok(())
}

pub(super) async fn prepare(
    sdk: &BreezSdk,
    request: PrepareLnurlPayRequest,
) -> Result<PrepareLnurlPayResponse, SdkError> {
    validate_request(&request)?;
    let fee_policy = request.fee_policy.unwrap_or_default();

    // Only run the token-conversion estimator when a ToBitcoin conversion is
    // actually configured. Otherwise the request is plain sats and the user's
    // amount passes through with no estimate attached.
    let (estimated_sats, conversion_estimate) = match request.conversion_options.as_ref() {
        Some(opts) if matches!(opts.conversion_type, ConversionType::ToBitcoin { .. }) => {
            conversion::estimate_sats_from_token_conversion(
                sdk,
                opts,
                request.token_identifier.as_ref(),
                request.amount,
                fee_policy,
            )
            .await?
        }
        _ => (request.amount, None),
    };

    // If the user is denominating in tokens (`token_identifier` set), the
    // conversion must be available — otherwise the request.amount (in token
    // units) would be silently treated as sats by the sats branch below.
    if request.token_identifier.is_some() && conversion_estimate.is_none() {
        return Err(SdkError::InvalidInput(
            "Token conversion is not available for the requested token and amount".to_string(),
        ));
    }
    // `amount` switches from token base units to sats post-conversion.
    // `fee_policy` is already validated; for token-denominated requests it's
    // guaranteed `FeesIncluded` by `validate_request`.
    let amount = estimated_sats;

    // FeesIncluded uses the double-query approach
    if fee_policy == FeePolicy::FeesIncluded {
        let amount_sats: u64 = amount
            .try_into()
            .map_err(|_| SdkError::InvalidInput("Amount too large for LNURL".to_string()))?;
        return prepare_fees_included(sdk, request, amount_sats, conversion_estimate).await;
    }

    // Regular send (no FeesIncluded, no conversion)
    let amount_sats: u64 = amount
        .try_into()
        .map_err(|_| SdkError::InvalidInput("Amount too large for LNURL".to_string()))?;

    let success_data =
        query_lnurl_invoice(sdk, &request, amount_sats.saturating_mul(1_000)).await?;

    let prepare_response = sdk
        .prepare_send_payment(crate::PrepareSendPaymentRequest {
            payment_request: crate::PaymentRequest::Input {
                input: success_data.pr,
            },
            amount: Some(u128::from(amount_sats)),
            token_identifier: request.token_identifier.clone(),
            conversion_options: request.conversion_options.clone(),
            fee_policy: None,
        })
        .await?;

    let SendPaymentMethod::Bolt11Invoice {
        invoice_details,
        lightning_fee_sats,
        ..
    } = prepare_response.payment_method
    else {
        return Err(SdkError::Generic(
            "Expected Bolt11Invoice payment method".to_string(),
        ));
    };

    Ok(PrepareLnurlPayResponse {
        amount_sats,
        comment: request.comment,
        pay_request: request.pay_request,
        invoice_details,
        fee_sats: lightning_fee_sats,
        success_action: success_data.success_action.map(From::from),
        conversion_estimate: prepare_response.conversion_estimate,
        fee_policy,
    })
}

/// Prepares an LNURL pay `FeesIncluded` operation using a double-query approach.
///
/// This method:
/// 1. Validates amount doesn't exceed LNURL `max_sendable`
/// 2. First query: gets invoice for full amount to estimate fees
/// 3. Calculates actual send amount (amount - estimated fee)
/// 4. Second query: gets invoice for actual amount
/// 5. Returns the prepare response with the second invoice
async fn prepare_fees_included(
    sdk: &BreezSdk,
    request: PrepareLnurlPayRequest,
    amount_sats: u64,
    conversion_estimate: Option<ConversionEstimate>,
) -> Result<PrepareLnurlPayResponse, SdkError> {
    if amount_sats == 0 {
        return Err(SdkError::InvalidInput(
            "Amount must be greater than 0".to_string(),
        ));
    }

    // 1. Validate amount is within LNURL limits
    let min_sendable_sats = request.pay_request.min_sendable.div_ceil(1000);
    let max_sendable_sats = request.pay_request.max_sendable / 1000;

    if amount_sats < min_sendable_sats {
        return Err(SdkError::InvalidInput(format!(
            "Amount ({amount_sats} sats) is below LNURL minimum ({min_sendable_sats} sats)"
        )));
    }

    if amount_sats > max_sendable_sats {
        return Err(SdkError::InvalidInput(format!(
            "Amount ({amount_sats} sats) exceeds LNURL maximum ({max_sendable_sats} sats)"
        )));
    }

    // 2. First query: get invoice for full amount to estimate fees.
    // Note: we don't intend to pay this invoice, it's only for fee estimation.
    let first_data = query_lnurl_invoice(sdk, &request, amount_sats.saturating_mul(1_000)).await?;

    // 3. Get fee estimate for first invoice
    let first_fee = sdk
        .spark_wallet
        .fetch_lightning_send_fee_estimate(&first_data.pr, None)
        .await?;

    // 4. Calculate actual send amount (amount - fee)
    let actual_amount = amount_sats.saturating_sub(first_fee);

    // Validate against LNURL minimum
    if actual_amount < min_sendable_sats {
        return Err(SdkError::InvalidInput(format!(
            "Amount after fees ({actual_amount} sats) is below LNURL minimum ({min_sendable_sats} sats)"
        )));
    }

    // 5. Second query: get invoice for actual amount (back-to-back, no delay).
    let success_data =
        query_lnurl_invoice(sdk, &request, actual_amount.saturating_mul(1_000)).await?;

    // 6. Get actual fee for the smaller invoice
    let actual_fee = sdk
        .spark_wallet
        .fetch_lightning_send_fee_estimate(&success_data.pr, None)
        .await?;

    // If fee increased between queries, fail (user must retry)
    if actual_fee > first_fee {
        return Err(SdkError::Generic(
            "Fee increased between queries. Please retry.".to_string(),
        ));
    }

    // Parse the invoice to get details
    let parsed = sdk.parse(&success_data.pr).await?;
    let InputType::Bolt11Invoice(invoice_details) = parsed else {
        return Err(SdkError::Generic(
            "Expected Bolt11 invoice from LNURL".to_string(),
        ));
    };

    info!(
        "LNURL FeesIncluded prepared: amount={amount_sats}, receiver_amount={actual_amount}, fee={first_fee}"
    );

    Ok(PrepareLnurlPayResponse {
        amount_sats,
        comment: request.comment,
        pay_request: request.pay_request,
        invoice_details,
        fee_sats: first_fee,
        success_action: success_data.success_action.map(From::from),
        conversion_estimate,
        fee_policy: FeePolicy::FeesIncluded,
    })
}

#[allow(clippy::too_many_lines)]
pub(super) async fn send(
    sdk: &BreezSdk,
    request: LnurlPayRequest,
) -> Result<LnurlPayResponse, SdkError> {
    sdk.maybe_ensure_spark_private_mode_initialized().await?;

    let is_fees_included = request.prepare_response.fee_policy == FeePolicy::FeesIncluded;

    // For FeesIncluded, extract amount from the invoice (set during prepare)
    let receiver_amount_sats: u64 = if is_fees_included {
        request
            .prepare_response
            .invoice_details
            .amount_msat
            .ok_or_else(|| SdkError::Generic("Missing invoice amount".to_string()))?
            / 1000
    } else {
        request.prepare_response.amount_sats
    };

    // Calculate amount override for FeesIncluded operations
    let amount_override = if is_fees_included {
        // Re-estimate current fee for the invoice
        let current_fee = sdk
            .spark_wallet
            .fetch_lightning_send_fee_estimate(
                &request.prepare_response.invoice_details.invoice.bolt11,
                None,
            )
            .await?;

        // fees_included_fee = first_fee (from prepare), which is the total we need to pay in fees
        let fees_included_fee = request.prepare_response.fee_sats;

        // Reconcile the prepare-time fee against the re-estimated fee, overpaying
        // by the difference to respect the prepared amount.
        let overpayment = crate::utils::fees::fee_overpayment(fees_included_fee, current_fee)?;

        if overpayment > 0 {
            tracing::info!(
                overpayment_sats = overpayment,
                fees_included_fee_sats = fees_included_fee,
                current_fee_sats = current_fee,
                "FeesIncluded fee overpayment applied"
            );
        }
        Some(receiver_amount_sats.saturating_add(overpayment))
    } else {
        None
    };

    // For conversions, use FeesIncluded so the send path deducts fees from
    // the post-conversion balance. For non-conversion FeesIncluded, the LNURL
    // flow handles fees via invoice sizing and amount_override.
    let has_conversion = request.prepare_response.conversion_estimate.is_some();
    let internal_fee_policy = if is_fees_included && has_conversion {
        FeePolicy::FeesIncluded
    } else {
        FeePolicy::FeesExcluded
    };

    let mut payment = Box::pin(send::orchestrate_send(
        sdk,
        SendPaymentRequest {
            prepare_response: PrepareSendPaymentResponse {
                payment_method: SendPaymentMethod::Bolt11Invoice {
                    invoice_details: request.prepare_response.invoice_details,
                    spark_transfer_fee_sats: None,
                    lightning_fee_sats: request.prepare_response.fee_sats,
                },
                // For conversions, use the prepare's total amount (before fee
                // deduction) so the sats_change logic in complete_conversion_and_send
                // correctly computes the post-conversion amount override.
                // For non-conversions, use the invoice amount.
                amount: if has_conversion {
                    u128::from(request.prepare_response.amount_sats)
                } else {
                    u128::from(receiver_amount_sats)
                },
                // LNURL always sends sats — token_identifier is None on the
                // internal PrepareSendPaymentResponse even when a conversion
                // is present (the token info is in conversion_estimate).
                token_identifier: None,
                conversion_estimate: request.prepare_response.conversion_estimate,
                fee_policy: internal_fee_policy,
            },
            options: None,
            idempotency_key: request.idempotency_key,
        },
        true,
        // For conversions, don't pass amount_override — let
        // complete_conversion_and_send compute it from sats_change.
        // For non-conversions, use the LNURL-computed override.
        if has_conversion {
            None
        } else {
            amount_override
        },
    ))
    .await?
    .payment;

    let success_action = process_success_action(
        &payment,
        request
            .prepare_response
            .success_action
            .clone()
            .map(Into::into)
            .as_ref(),
    )?;

    let lnurl_info = LnurlPayInfo {
        ln_address: request.prepare_response.pay_request.address,
        comment: request.prepare_response.comment,
        domain: Some(request.prepare_response.pay_request.domain),
        metadata: Some(request.prepare_response.pay_request.metadata_str),
        processed_success_action: success_action.clone().map(From::from),
        raw_success_action: request.prepare_response.success_action,
    };
    let lnurl_description = lnurl_info.extract_description();

    match &mut payment.details {
        Some(crate::PaymentDetails::Lightning {
            lnurl_pay_info,
            description,
            ..
        }) => {
            *lnurl_pay_info = Some(lnurl_info.clone());
            description.clone_from(&lnurl_description);
        }
        // When the LNURL server includes a Spark routing hint, the payment
        // is routed via Spark transfer. The Spark variant doesn't carry
        // lnurl fields, so we just persist the metadata separately below.
        Some(crate::PaymentDetails::Spark { .. }) => {}
        _ => {
            return Err(SdkError::Generic(
                "Expected Lightning or Spark payment details".to_string(),
            ));
        }
    }

    sdk.storage
        .insert_payment_metadata(
            payment.id.clone(),
            PaymentMetadata {
                lnurl_pay_info: Some(lnurl_info),
                lnurl_description,
                ..Default::default()
            },
        )
        .await?;

    // Emit the payment with metadata already included
    sdk.event_emitter
        .emit(&SdkEvent::from_payment(payment.clone()))
        .await;
    Ok(LnurlPayResponse {
        payment,
        success_action: success_action.map(From::from),
    })
}

/// Calls the LNURL pay endpoint for the given `amount_msat` and unwraps the
/// success branch into a `CallbackResponse`, mapping `EndpointError` into an
/// `SdkError`.
async fn query_lnurl_invoice(
    sdk: &BreezSdk,
    request: &PrepareLnurlPayRequest,
    amount_msat: u64,
) -> Result<CallbackResponse, SdkError> {
    let response = validate_lnurl_pay(
        sdk.lnurl_client.as_ref(),
        amount_msat,
        &request.comment,
        &request.pay_request.clone().into(),
        sdk.config.network.into(),
        request.validate_success_action_url,
    )
    .await?;
    match response {
        ValidatedCallbackResponse::EndpointError { data } => {
            Err(LnurlError::EndpointError(data.reason).into())
        }
        ValidatedCallbackResponse::EndpointSuccess { data } => Ok(data),
    }
}

#[cfg(test)]
mod tests {
    use super::validate_request;
    use crate::{
        ConversionOptions, ConversionType, FeePolicy, LnurlPayRequestDetails,
        PrepareLnurlPayRequest, error::SdkError,
    };
    use macros::test_all;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    fn pay_request_details() -> LnurlPayRequestDetails {
        LnurlPayRequestDetails {
            callback: "https://example.com/callback".to_string(),
            min_sendable: 1_000,
            max_sendable: 100_000_000_000,
            metadata_str: "[]".to_string(),
            comment_allowed: 0,
            domain: "example.com".to_string(),
            url: "https://example.com".to_string(),
            address: None,
            allows_nostr: None,
            nostr_pubkey: None,
        }
    }

    fn request_with(
        amount: u128,
        token_identifier: Option<&str>,
        fee_policy: Option<FeePolicy>,
    ) -> PrepareLnurlPayRequest {
        PrepareLnurlPayRequest {
            amount,
            pay_request: pay_request_details(),
            comment: None,
            validate_success_action_url: None,
            token_identifier: token_identifier.map(String::from),
            conversion_options: None,
            fee_policy,
        }
    }

    fn to_bitcoin_options() -> ConversionOptions {
        ConversionOptions {
            conversion_type: ConversionType::ToBitcoin {
                from_token_identifier: "token123".to_string(),
            },
            max_slippage_bps: None,
            completion_timeout_secs: None,
        }
    }

    fn from_bitcoin_options() -> ConversionOptions {
        ConversionOptions {
            conversion_type: ConversionType::FromBitcoin,
            max_slippage_bps: None,
            completion_timeout_secs: None,
        }
    }

    // ---- Amount ----

    #[test_all]
    fn test_validate_lnurl_pay_amount_zero_rejected() {
        let request = request_with(0, None, None);
        let result = validate_request(&request);
        assert!(result.is_err());
        if let Err(SdkError::InvalidInput(msg)) = result {
            assert!(msg.contains("must be greater than 0"));
        } else {
            panic!("Expected InvalidInput error");
        }
    }

    #[test_all]
    fn test_validate_lnurl_pay_positive_amount_ok() {
        assert!(validate_request(&request_with(1_000, None, None)).is_ok());
    }

    // ---- FeesIncluded + FromBitcoin (shared rule) ----

    #[test_all]
    fn test_validate_lnurl_pay_fees_included_with_from_bitcoin_rejected() {
        let mut request = request_with(1_000, None, Some(FeePolicy::FeesIncluded));
        request.conversion_options = Some(from_bitcoin_options());
        let result = validate_request(&request);
        assert!(result.is_err());
        if let Err(SdkError::InvalidInput(msg)) = result {
            assert!(msg.contains("FeesIncluded cannot be combined with FromBitcoin"));
        } else {
            panic!("Expected InvalidInput error");
        }
    }

    #[test_all]
    fn test_validate_lnurl_pay_fees_included_with_to_bitcoin_ok() {
        let mut request = request_with(1_000, Some("token123"), Some(FeePolicy::FeesIncluded));
        request.conversion_options = Some(to_bitcoin_options());
        assert!(validate_request(&request).is_ok());
    }

    // ---- Token-denominated requires FeesIncluded ----

    #[test_all]
    fn test_validate_lnurl_pay_token_identifier_with_fees_excluded_rejected() {
        let request = request_with(1_000, Some("token123"), Some(FeePolicy::FeesExcluded));
        let result = validate_request(&request);
        assert!(result.is_err());
        if let Err(SdkError::InvalidInput(msg)) = result {
            assert!(msg.contains("requires FeesIncluded"));
        } else {
            panic!("Expected InvalidInput error");
        }
    }

    #[test_all]
    fn test_validate_lnurl_pay_token_identifier_with_default_fee_policy_rejected() {
        // None defaults to FeesExcluded, which is rejected for token-denominated.
        let request = request_with(1_000, Some("token123"), None);
        let result = validate_request(&request);
        assert!(result.is_err());
        if let Err(SdkError::InvalidInput(msg)) = result {
            assert!(msg.contains("requires FeesIncluded"));
        } else {
            panic!("Expected InvalidInput error");
        }
    }

    #[test_all]
    fn test_validate_lnurl_pay_token_identifier_with_fees_included_ok() {
        let request = request_with(1_000, Some("token123"), Some(FeePolicy::FeesIncluded));
        assert!(validate_request(&request).is_ok());
    }

    #[test_all]
    fn test_validate_lnurl_pay_no_token_identifier_no_fee_policy_ok() {
        // Plain LNURL pay (no token, no fee policy) — must work.
        assert!(validate_request(&request_with(1_000, None, None)).is_ok());
    }
}
