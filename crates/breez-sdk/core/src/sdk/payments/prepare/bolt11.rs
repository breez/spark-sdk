use spark_wallet::SparkAddress;

use crate::{
    Bolt11InvoiceDetails, ConversionOptions, ConversionType, FeePolicy, SendPaymentMethod,
    error::SdkError,
    models::{PrepareSendPaymentRequest, PrepareSendPaymentResponse},
    sdk::BreezSdk,
    token_conversion::ConversionAmount,
};

use super::super::{conversion, validation};

/// Validates a Bolt11 invoice request.
fn validate_request(
    invoice_details: &Bolt11InvoiceDetails,
    request: &PrepareSendPaymentRequest,
) -> Result<(), SdkError> {
    validation::validate_amount(request.amount)?;
    validation::validate_fee_policy_for_conversion(
        request.fee_policy,
        request.conversion_options.as_ref(),
    )?;

    // FeesIncluded is only supported for amountless Bolt11 invoices
    if request.fee_policy == Some(FeePolicy::FeesIncluded) && invoice_details.amount_msat.is_some()
    {
        return Err(SdkError::InvalidInput(
            "FeesIncluded is not supported for invoices with a fixed amount".to_string(),
        ));
    }

    // Token identifier cannot be provided for Bolt11 invoices unless ToBitcoin conversion
    // is present (send-all-with-conversion from stable balance).
    if request.token_identifier.is_some()
        && !matches!(
            &request.conversion_options,
            Some(ConversionOptions {
                conversion_type: ConversionType::ToBitcoin { .. },
                ..
            })
        )
    {
        return Err(SdkError::InvalidInput(
            "Token identifier can't be provided for this payment request: non-spark address"
                .to_string(),
        ));
    }

    // Token-denominated payment to a fixed-amount Bolt11 invoice is ambiguous:
    // the user's converted sats may not match the invoice amount, causing
    // overpayment or send failure. Omit `amount` so the SDK can derive the
    // conversion from the invoice and fees instead.
    if invoice_details.amount_msat.is_some()
        && request.amount.is_some()
        && request.token_identifier.is_some()
    {
        return Err(SdkError::InvalidInput(
            "Token amount is not supported for invoices with a fixed amount".to_string(),
        ));
    }

    // Conversion from Bitcoin is not supported for Bolt11 invoices
    if matches!(
        &request.conversion_options,
        Some(ConversionOptions {
            conversion_type: ConversionType::FromBitcoin,
            ..
        })
    ) {
        return Err(SdkError::InvalidInput(
            "Conversion must be to Bitcoin for Bolt11 invoices".to_string(),
        ));
    }

    Ok(())
}

pub(super) async fn prepare(
    sdk: &BreezSdk,
    input: &str,
    request: &PrepareSendPaymentRequest,
    detailed_bolt11_invoice: &Bolt11InvoiceDetails,
    fee_policy: FeePolicy,
    token_identifier: Option<String>,
) -> Result<PrepareSendPaymentResponse, SdkError> {
    validate_request(detailed_bolt11_invoice, request)?;

    let spark_address: Option<SparkAddress> = sdk.spark_wallet.extract_spark_address(input)?;

    let spark_transfer_fee_sats = if spark_address.is_some() {
        Some(0)
    } else {
        None
    };

    if let Some(opts) = request.conversion_options.as_ref()
        && conversion::is_token_denominated(request.amount, Some(opts), token_identifier.as_ref())
    {
        return prepare_token_denominated(
            sdk,
            input,
            opts,
            request,
            detailed_bolt11_invoice,
            spark_transfer_fee_sats,
            token_identifier.as_ref(),
            fee_policy,
        )
        .await;
    }

    prepare_sats_denominated(
        sdk,
        input,
        request,
        detailed_bolt11_invoice,
        spark_transfer_fee_sats,
        token_identifier,
        fee_policy,
    )
    .await
}

/// Sats-denominated Bolt11 prepare: `request.amount` (or the invoice's `amount_msat`)
/// is in sats. Fetches the lightning fee for the user's amount, validates the
/// receiver covers fees for `FeesIncluded` amountless invoices, and attaches a
/// `MinAmountOut` conversion estimate for display when conversion options are set.
async fn prepare_sats_denominated(
    sdk: &BreezSdk,
    input: &str,
    request: &PrepareSendPaymentRequest,
    invoice: &Bolt11InvoiceDetails,
    spark_transfer_fee_sats: Option<u64>,
    token_identifier: Option<String>,
    fee_policy: FeePolicy,
) -> Result<PrepareSendPaymentResponse, SdkError> {
    let amount = request
        .amount
        .or(invoice
            .amount_msat
            .map(|msat| u128::from(msat).saturating_div(1000)))
        .ok_or(SdkError::InvalidInput("Amount is required".to_string()))?;

    // For FeesIncluded, estimate fee for user's full amount
    let lightning_fee_sats = sdk
        .spark_wallet
        .fetch_lightning_send_fee_estimate(input, Some(amount.try_into()?))
        .await?;

    // Validate receiver amount is positive for FeesIncluded
    if fee_policy == FeePolicy::FeesIncluded && invoice.amount_msat.is_none() {
        let amount_u64: u64 = amount.try_into()?;
        if amount_u64 <= lightning_fee_sats {
            return Err(SdkError::InvalidInput(
                "Amount too small to cover fees".to_string(),
            ));
        }
    }

    let conversion_estimate = conversion::estimate_conversion(
        sdk,
        request.conversion_options.as_ref(),
        token_identifier.as_ref(),
        ConversionAmount::MinAmountOut(amount.saturating_add(u128::from(lightning_fee_sats))),
    )
    .await?;

    Ok(PrepareSendPaymentResponse {
        payment_method: SendPaymentMethod::Bolt11Invoice {
            invoice_details: invoice.clone(),
            spark_transfer_fee_sats,
            lightning_fee_sats,
        },
        amount,
        token_identifier,
        conversion_estimate,
        fee_policy,
    })
}

/// Token-denominated Bolt11 prepare: `request.amount` is in token base units and
/// `conversion_options` is `ToBitcoin`. Estimates the conversion, fetches lightning
/// fees based on the estimated sats, and validates the conversion output covers
/// the invoice + fees.
///
/// Returns an explicit `InvalidInput` error when the converter can't validate the
/// requested conversion (rare — unsupported config / temporary outage). The
/// caller must not silently fall back to the sats-denominated path, since the
/// user's `amount` is in token units and would be misinterpreted as sats.
#[allow(clippy::too_many_arguments)]
async fn prepare_token_denominated(
    sdk: &BreezSdk,
    input: &str,
    conversion_options: &ConversionOptions,
    request: &PrepareSendPaymentRequest,
    invoice: &Bolt11InvoiceDetails,
    spark_transfer_fee_sats: Option<u64>,
    token_identifier: Option<&String>,
    fee_policy: FeePolicy,
) -> Result<PrepareSendPaymentResponse, SdkError> {
    // The is_token_denominated gate at the call site guarantees amount.is_some().
    let token_amount = request.amount.ok_or_else(|| {
        SdkError::Generic("prepare_token_denominated called without amount".to_string())
    })?;
    let (estimated_sats, conversion_estimate) = conversion::estimate_sats_from_token_conversion(
        sdk,
        conversion_options,
        token_identifier,
        token_amount,
        fee_policy,
    )
    .await?;
    if conversion_estimate.is_none() {
        return Err(SdkError::InvalidInput(
            "Token conversion is not available for the requested token and amount".to_string(),
        ));
    }

    let lightning_fee_sats = sdk
        .spark_wallet
        .fetch_lightning_send_fee_estimate(input, Some(estimated_sats.try_into()?))
        .await?;

    let total_u64: u64 = estimated_sats.try_into()?;
    // For fixed-amount invoices, the converted sats must cover invoice amount + fees.
    // For amountless invoices (send-all), just check fees are covered.
    let min_required = if let Some(amount_msat) = invoice.amount_msat {
        (amount_msat / 1000).saturating_add(lightning_fee_sats)
    } else {
        lightning_fee_sats
    };
    if total_u64 <= min_required {
        return Err(SdkError::InvalidInput(
            "Token conversion amount too small to cover invoice amount and fees".to_string(),
        ));
    }

    Ok(PrepareSendPaymentResponse {
        payment_method: SendPaymentMethod::Bolt11Invoice {
            invoice_details: invoice.clone(),
            spark_transfer_fee_sats,
            lightning_fee_sats,
        },
        amount: estimated_sats,
        // ToBitcoin conversion outputs sats — token_identifier is None
        token_identifier: None,
        conversion_estimate,
        fee_policy,
    })
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::*;
    use super::validate_request;
    use crate::{ConversionOptions, ConversionType, error::SdkError};
    use macros::test_all;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    // ---- Token identifier requires ToBitcoin conversion ----

    #[test_all]
    fn test_validate_bolt11_invoice_without_token_identifier() {
        let invoice = create_test_bolt11_invoice();
        let request = create_test_request();
        let result = validate_request(&invoice, &request);
        assert!(
            result.is_ok(),
            "Should succeed when token identifier is not provided"
        );
    }

    #[test_all]
    fn test_validate_bolt11_invoice_with_token_identifier() {
        let invoice = create_test_bolt11_invoice();
        let request = create_token_amount_request(1000, "token123");
        let result = validate_request(&invoice, &request);
        assert!(
            result.is_err(),
            "Should fail when token identifier is provided"
        );
        if let Err(SdkError::InvalidInput(msg)) = result {
            assert!(
                msg.contains("can't be provided"),
                "Error message should mention it can't be provided"
            );
        } else {
            panic!("Expected InvalidInput error");
        }
    }

    // ---- FeesIncluded only on amountless invoices ----

    #[test_all]
    fn test_validate_bolt11_invoice_fees_included_with_amountless_invoice() {
        let invoice = create_test_bolt11_invoice(); // No amount
        let request = create_fees_included_request(1000);
        let result = validate_request(&invoice, &request);
        assert!(
            result.is_ok(),
            "Should succeed when FeesIncluded is used for amountless Bolt11 invoice"
        );
    }

    #[test_all]
    fn test_validate_bolt11_invoice_fees_included_with_amount_invoice() {
        let mut invoice = create_test_bolt11_invoice();
        invoice.amount_msat = Some(1_000_000); // Invoice has fixed amount
        let request = create_fees_included_request(1000);
        let result = validate_request(&invoice, &request);
        assert!(
            result.is_err(),
            "Should fail when FeesIncluded is used for Bolt11 invoice with fixed amount"
        );
        if let Err(SdkError::InvalidInput(msg)) = result {
            assert!(
                msg.contains("not supported for invoices with a fixed amount"),
                "Error message should mention fixed amount"
            );
        } else {
            panic!("Expected InvalidInput error");
        }
    }

    // ---- Conversion direction ----

    #[test_all]
    fn test_validate_bolt11_invoice_with_valid_conversion() {
        let invoice = create_test_bolt11_invoice();
        let mut request = create_test_request();
        request.conversion_options = Some(ConversionOptions {
            conversion_type: ConversionType::ToBitcoin {
                from_token_identifier: "token123".to_string(),
            },
            max_slippage_bps: None,
            completion_timeout_secs: None,
        });
        let result = validate_request(&invoice, &request);
        assert!(
            result.is_ok(),
            "Should succeed when conversion to Bitcoin is provided"
        );
    }

    #[test_all]
    fn test_validate_bolt11_invoice_with_invalid_conversion() {
        let invoice = create_test_bolt11_invoice();
        let mut request = create_test_request();
        request.conversion_options = Some(ConversionOptions {
            conversion_type: ConversionType::FromBitcoin,
            max_slippage_bps: None,
            completion_timeout_secs: None,
        });
        let result = validate_request(&invoice, &request);
        assert!(
            result.is_err(),
            "Should fail when conversion from Bitcoin is provided"
        );
    }

    // ---- Token amount + fixed-amount invoice (anti-pattern) ----

    #[test_all]
    fn test_validate_bolt11_token_payment_to_fixed_amount_invoice_rejected() {
        // Fixed-amount invoice + user-supplied token amount + token_identifier
        // is ambiguous (would overpay or fail); the SDK-derive path requires
        // omitting `amount`.
        let mut invoice = create_test_bolt11_invoice();
        invoice.amount_msat = Some(1_000_000); // 1000 sats fixed
        let mut request = create_token_amount_request(2000, "token123");
        request.conversion_options = Some(ConversionOptions {
            conversion_type: ConversionType::ToBitcoin {
                from_token_identifier: "token123".to_string(),
            },
            max_slippage_bps: None,
            completion_timeout_secs: None,
        });
        let result = validate_request(&invoice, &request);
        assert!(
            result.is_err(),
            "Should reject token amount supplied for a fixed-amount Bolt11 invoice"
        );
        if let Err(SdkError::InvalidInput(msg)) = result {
            assert!(
                msg.contains("Token amount") && msg.contains("fixed amount"),
                "Error should explain the ambiguity (got: {msg})"
            );
        } else {
            panic!("Expected InvalidInput error");
        }
    }

    #[test_all]
    fn test_validate_bolt11_token_payment_to_fixed_amount_invoice_no_amount_ok() {
        // The supported path for paying a fixed-amount invoice with tokens:
        // user omits `amount`, SDK derives the conversion from the invoice + fees.
        let mut invoice = create_test_bolt11_invoice();
        invoice.amount_msat = Some(1_000_000);
        let mut request = create_test_request();
        request.token_identifier = Some("token123".to_string());
        request.conversion_options = Some(ConversionOptions {
            conversion_type: ConversionType::ToBitcoin {
                from_token_identifier: "token123".to_string(),
            },
            max_slippage_bps: None,
            completion_timeout_secs: None,
        });
        assert!(
            validate_request(&invoice, &request).is_ok(),
            "Token payment to fixed-amount invoice should be allowed when amount is omitted"
        );
    }
}
