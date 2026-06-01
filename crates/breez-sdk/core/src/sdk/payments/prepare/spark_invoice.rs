use platform_utils::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::{
    ConversionOptions, ConversionType, FeePolicy, SendPaymentMethod, SparkInvoiceDetails,
    error::SdkError,
    models::{PrepareSendPaymentRequest, PrepareSendPaymentResponse},
    sdk::BreezSdk,
    sdk::payments::{conversion, validation},
};

/// Validates a spark invoice request against the provided request parameters.
fn validate_request(
    spark_invoice_details: &SparkInvoiceDetails,
    request: &PrepareSendPaymentRequest,
    identity_public_key: &str,
) -> Result<(), SdkError> {
    validation::validate_amount(request.amount)?;
    validation::validate_fee_policy_for_conversion(
        request.fee_policy,
        request.conversion_options.as_ref(),
    )?;

    // FeesIncluded is only supported for amountless Spark invoices
    if request.fee_policy == Some(FeePolicy::FeesIncluded) && spark_invoice_details.amount.is_some()
    {
        return Err(SdkError::InvalidInput(
            "FeesIncluded is not supported for invoices with a fixed amount".to_string(),
        ));
    }

    let requested_token_identifier = request.token_identifier.clone();
    let request_amount = request.amount;

    // Validate token identifier
    if let Some(token_identifier) = &spark_invoice_details.token_identifier {
        // Error if token identifier doesn't match (when explicitly provided)
        if let Some(ref req_token_id) = requested_token_identifier
            && req_token_id != token_identifier
        {
            return Err(SdkError::InvalidInput(
                "Requested token identifier does not match invoice token identifier".to_string(),
            ));
        }
        // pay_amount: None is allowed - defers to invoice
        // Validate token conversion to Bitcoin is not supported for tokens invoices
        if matches!(
            &request.conversion_options,
            Some(ConversionOptions {
                conversion_type: ConversionType::ToBitcoin { .. },
                ..
            })
        ) {
            return Err(SdkError::InvalidInput(
                "Conversion must be from Bitcoin for tokens invoice".to_string(),
            ));
        }
    } else if requested_token_identifier.is_some() {
        return Err(SdkError::InvalidInput(
            "Token identifier can't be provided for this payment request: non-tokens invoice"
                .to_string(),
        ));
    } else if matches!(
        &request.conversion_options,
        Some(ConversionOptions {
            conversion_type: ConversionType::FromBitcoin,
            ..
        })
    ) {
        return Err(SdkError::InvalidInput(
            "Conversion must be to Bitcoin for non-tokens invoice".to_string(),
        ));
    }

    // Validate expiry time
    if let Some(expiry_time) = spark_invoice_details.expiry_time {
        let current_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| SdkError::Generic("Failed to get current time".to_string()))?;
        if current_time > Duration::from_secs(expiry_time) {
            return Err(SdkError::InvalidInput("Invoice has expired".to_string()));
        }
    }

    // Validate sender public key
    if let Some(sender_public_key) = &spark_invoice_details.sender_public_key
        && identity_public_key != sender_public_key
    {
        return Err(SdkError::InvalidInput(
            format!("Invoice can only be paid by sender public key {sender_public_key}")
                .to_string(),
        ));
    }

    // Validate amount
    if let Some(invoice_amount) = spark_invoice_details.amount
        && let Some(req_amount) = request_amount
        && invoice_amount != req_amount
    {
        return Err(SdkError::InvalidInput(
            "Requested amount does not match invoice amount".to_string(),
        ));
    }

    // Validate amount is provided when invoice has no amount
    if spark_invoice_details.amount.is_none() && request_amount.is_none() {
        return Err(SdkError::InvalidInput(
            "Amount is required when invoice has no amount".to_string(),
        ));
    }

    Ok(())
}

pub(super) async fn prepare(
    sdk: &BreezSdk,
    request: &PrepareSendPaymentRequest,
    details: &SparkInvoiceDetails,
    fee_policy: FeePolicy,
    token_identifier: Option<String>,
) -> Result<PrepareSendPaymentResponse, SdkError> {
    validate_request(
        details,
        request,
        &sdk.spark_wallet.get_identity_public_key().to_string(),
    )?;

    // Use request's token_identifier if provided, otherwise fall back to invoice's
    let effective_token_identifier = token_identifier.or_else(|| details.token_identifier.clone());

    let amount = details
        .amount
        .or(request.amount)
        .ok_or(SdkError::InvalidInput("Amount is required".to_string()))?;

    let (amount, conversion_estimate) = conversion::resolve_send_amount_with_conversion_estimate(
        sdk,
        request.conversion_options.as_ref(),
        effective_token_identifier.as_ref(),
        amount,
        fee_policy,
    )
    .await?;

    let response_token_identifier = conversion::response_token_identifier(
        conversion_estimate.as_ref(),
        effective_token_identifier,
    );

    Ok(PrepareSendPaymentResponse {
        payment_method: SendPaymentMethod::SparkInvoice {
            spark_invoice_details: details.clone(),
            fee: 0,
            token_identifier: response_token_identifier.clone(),
        },
        amount,
        token_identifier: response_token_identifier,
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
    use platform_utils::time::{SystemTime, UNIX_EPOCH};

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    // ---- Token identifier match / mismatch / allowed / not allowed ----

    #[test_all]
    fn test_validate_spark_invoice_token_identifier_match() {
        let mut invoice = create_test_invoice();
        invoice.token_identifier = Some("token123".to_string());

        let request = create_token_amount_request(1000, "token123");

        let identity_key = "test_identity".to_string();
        let result = validate_request(&invoice, &request, &identity_key);
        assert!(
            result.is_ok(),
            "Should succeed when token identifiers match"
        );
    }

    #[test_all]
    fn test_validate_spark_invoice_token_identifier_mismatch() {
        let mut invoice = create_test_invoice();
        invoice.token_identifier = Some("token123".to_string());

        let request = create_token_amount_request(1000, "token456");

        let identity_key = "test_identity".to_string();
        let result = validate_request(&invoice, &request, &identity_key);
        assert!(
            result.is_err(),
            "Should fail when token identifiers don't match"
        );
        if let Err(SdkError::InvalidInput(msg)) = result {
            assert!(
                msg.contains("does not match"),
                "Error message should mention mismatch"
            );
        } else {
            panic!("Expected InvalidInput error");
        }
    }

    #[test_all]
    fn test_validate_spark_invoice_no_pay_amount_allowed_for_token_invoice() {
        let mut invoice = create_test_invoice();
        invoice.token_identifier = Some("token123".to_string());
        invoice.amount = Some(1000); // Invoice specifies amount

        let request = create_test_request(); // No pay_amount - defers to invoice

        let identity_key = "test_identity".to_string();
        let result = validate_request(&invoice, &request, &identity_key);
        assert!(
            result.is_ok(),
            "Should succeed when pay_amount is None for token invoice (defers to invoice)"
        );
    }

    #[test_all]
    fn test_validate_spark_invoice_token_identifier_not_allowed() {
        let invoice = create_test_invoice(); // No token identifier

        let request = create_token_amount_request(1000, "token123");

        let identity_key = "test_identity".to_string();
        let result = validate_request(&invoice, &request, &identity_key);
        assert!(
            result.is_err(),
            "Should fail when token identifier is provided for non-token invoice"
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

    // ---- FeesIncluded ----

    #[test_all]
    fn test_validate_spark_invoice_fees_included_with_amountless_invoice() {
        let invoice = create_test_invoice(); // No amount
        let request = create_fees_included_request(1000);

        let identity_key = "test_identity".to_string();
        let result = validate_request(&invoice, &request, &identity_key);
        assert!(
            result.is_ok(),
            "Should succeed when FeesIncluded is used for amountless Spark invoice"
        );
    }

    #[test_all]
    fn test_validate_spark_invoice_fees_included_with_amount_invoice() {
        let mut invoice = create_test_invoice();
        invoice.amount = Some(1000); // Invoice has fixed amount
        let request = create_fees_included_request(1000);

        let identity_key = "test_identity".to_string();
        let result = validate_request(&invoice, &request, &identity_key);
        assert!(
            result.is_err(),
            "Should fail when FeesIncluded is used for Spark invoice with fixed amount"
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

    #[test_all]
    fn test_validate_spark_invoice_fees_included_with_token_invoice() {
        let mut invoice = create_test_invoice();
        invoice.token_identifier = Some("token123".to_string()); // Token invoice (amountless)
        let mut request = create_fees_included_request(1000);
        request.token_identifier = Some("token123".to_string());

        let identity_key = "test_identity".to_string();
        let result = validate_request(&invoice, &request, &identity_key);
        assert!(
            result.is_ok(),
            "Should succeed when FeesIncluded is used for token Spark invoice"
        );
    }

    // ---- Invoice expiry ----

    #[test_all]
    fn test_validate_spark_invoice_expired() {
        let mut invoice = create_test_invoice();
        let expired_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .saturating_sub(1);
        invoice.expiry_time = Some(expired_time);

        let request = create_test_request();
        let identity_key = "test_identity".to_string();
        let result = validate_request(&invoice, &request, &identity_key);
        assert!(result.is_err(), "Should fail when invoice has expired");
        if let Err(SdkError::InvalidInput(msg)) = result {
            assert!(
                msg.contains("expired"),
                "Error message should mention expiry"
            );
        } else {
            panic!("Expected InvalidInput error");
        }
    }

    #[allow(clippy::arithmetic_side_effects)]
    #[test_all]
    fn test_validate_spark_invoice_valid_expiry_time() {
        let mut invoice = create_test_invoice();
        invoice.amount = Some(1000); // Invoice specifies amount
        let future_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;
        invoice.expiry_time = Some(future_time);

        let request = create_test_request();
        let identity_key = "test_identity".to_string();
        let result = validate_request(&invoice, &request, &identity_key);
        assert!(result.is_ok(), "Should succeed when invoice hasn't expired");
    }

    // ---- Sender public key ----

    #[test_all]
    fn test_validate_spark_invoice_sender_public_key_match() {
        let mut invoice = create_test_invoice();
        invoice.amount = Some(1000); // Invoice specifies amount
        invoice.sender_public_key = Some("sender_key123".to_string());

        let request = create_test_request();
        let identity_key = "sender_key123".to_string();
        let result = validate_request(&invoice, &request, &identity_key);
        assert!(
            result.is_ok(),
            "Should succeed when sender public key matches"
        );
    }

    #[test_all]
    fn test_validate_spark_invoice_sender_public_key_mismatch() {
        let mut invoice = create_test_invoice();
        invoice.sender_public_key = Some("sender_key123".to_string());

        let request = create_test_request();
        let identity_key = "different_key".to_string();
        let result = validate_request(&invoice, &request, &identity_key);
        assert!(
            result.is_err(),
            "Should fail when sender public key doesn't match"
        );
        if let Err(SdkError::InvalidInput(msg)) = result {
            assert!(
                msg.contains("can only be paid by sender public key"),
                "Error message should mention sender restriction"
            );
        } else {
            panic!("Expected InvalidInput error");
        }
    }

    // ---- Amount validation ----

    #[test_all]
    fn test_validate_spark_invoice_amount_match() {
        let mut invoice = create_test_invoice();
        invoice.amount = Some(1000);

        let request = create_bitcoin_amount_request(1000);

        let identity_key = "test_identity".to_string();
        let result = validate_request(&invoice, &request, &identity_key);
        assert!(result.is_ok(), "Should succeed when amounts match");
    }

    #[test_all]
    fn test_validate_spark_invoice_amount_mismatch() {
        let mut invoice = create_test_invoice();
        invoice.amount = Some(1000);

        let request = create_bitcoin_amount_request(2000);

        let identity_key = "test_identity".to_string();
        let result = validate_request(&invoice, &request, &identity_key);
        assert!(result.is_err(), "Should fail when amounts don't match");
        if let Err(SdkError::InvalidInput(msg)) = result {
            assert!(
                msg.contains("does not match invoice amount"),
                "Error message should mention amount mismatch"
            );
        } else {
            panic!("Expected InvalidInput error");
        }
    }

    #[test_all]
    fn test_validate_spark_invoice_amount_invoice_only() {
        let mut invoice = create_test_invoice();
        invoice.amount = Some(1000);

        let request = create_test_request(); // No amount in request

        let identity_key = "test_identity".to_string();
        let result = validate_request(&invoice, &request, &identity_key);
        assert!(
            result.is_ok(),
            "Should succeed when only invoice has amount"
        );
    }

    #[test_all]
    fn test_validate_spark_invoice_amount_required_when_no_invoice_amount() {
        let invoice = create_test_invoice(); // No amount

        let request = create_test_request(); // No pay_amount

        let identity_key = "test_identity".to_string();
        let result = validate_request(&invoice, &request, &identity_key);
        assert!(
            result.is_err(),
            "Should fail when neither invoice nor request has amount"
        );
        if let Err(SdkError::InvalidInput(msg)) = result {
            assert!(
                msg.contains("Amount is required"),
                "Error message should mention amount requirement"
            );
        } else {
            panic!("Expected InvalidInput error");
        }
    }

    #[test_all]
    fn test_validate_spark_invoice_token_amount_required_when_no_invoice_amount() {
        let mut invoice = create_test_invoice();
        invoice.token_identifier = Some("token123".to_string());
        // invoice.amount is None

        let request = create_test_request(); // No pay_amount

        let identity_key = "test_identity".to_string();
        let result = validate_request(&invoice, &request, &identity_key);
        assert!(
            result.is_err(),
            "Should fail when neither token invoice nor request has amount"
        );
        if let Err(SdkError::InvalidInput(msg)) = result {
            assert!(
                msg.contains("Amount is required"),
                "Error message should mention amount requirement"
            );
        } else {
            panic!("Expected InvalidInput error");
        }
    }

    // ---- Comprehensive positive ----

    #[allow(clippy::arithmetic_side_effects)]
    #[test_all]
    fn test_validate_spark_invoice_all_valid() {
        let mut invoice = create_test_invoice();
        invoice.token_identifier = Some("token123".to_string());
        invoice.amount = Some(1000);
        invoice.sender_public_key = Some("sender_key123".to_string());
        let future_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;
        invoice.expiry_time = Some(future_time);

        let request = create_token_amount_request(1000, "token123");

        let identity_key = "sender_key123".to_string();
        let result = validate_request(&invoice, &request, &identity_key);
        assert!(result.is_ok(), "Should succeed when all validations pass");
    }

    // ---- Conversion direction ----

    #[test_all]
    fn test_validate_spark_invoice_with_valid_token_conversion() {
        let mut invoice = create_test_invoice();
        invoice.amount = Some(1000); // Invoice specifies amount

        let mut request = create_test_request();
        request.conversion_options = Some(ConversionOptions {
            conversion_type: ConversionType::ToBitcoin {
                from_token_identifier: "token123".to_string(),
            },
            max_slippage_bps: None,
            completion_timeout_secs: None,
        });

        let identity_key = "test_identity".to_string();
        let result = validate_request(&invoice, &request, &identity_key);
        assert!(
            result.is_ok(),
            "Should succeed when conversion to Bitcoin is provided"
        );
    }

    #[test_all]
    fn test_validate_token_spark_invoice_with_valid_conversion() {
        let mut invoice = create_test_invoice();
        invoice.token_identifier = Some("token123".to_string());

        let mut request = create_token_amount_request(1000, "token123");
        request.conversion_options = Some(ConversionOptions {
            conversion_type: ConversionType::FromBitcoin,
            max_slippage_bps: None,
            completion_timeout_secs: None,
        });

        let identity_key = "test_identity".to_string();
        let result = validate_request(&invoice, &request, &identity_key);
        assert!(
            result.is_ok(),
            "Should succeed when conversion from Bitcoin is provided"
        );
    }

    #[test_all]
    fn test_validate_spark_invoice_with_invalid_conversion() {
        let invoice = create_test_invoice();

        let mut request = create_test_request();
        request.conversion_options = Some(ConversionOptions {
            conversion_type: ConversionType::FromBitcoin,
            max_slippage_bps: None,
            completion_timeout_secs: None,
        });

        let identity_key = "test_identity".to_string();
        let result = validate_request(&invoice, &request, &identity_key);
        assert!(
            result.is_err(),
            "Should fail when conversion from Bitcoin is provided"
        );
    }

    #[test_all]
    fn test_validate_token_spark_invoice_with_invalid_conversion() {
        let mut invoice = create_test_invoice();
        invoice.token_identifier = Some("token123".to_string());

        let mut request = create_token_amount_request(1000, "token123");
        request.conversion_options = Some(ConversionOptions {
            conversion_type: ConversionType::ToBitcoin {
                from_token_identifier: "token123".to_string(),
            },
            max_slippage_bps: None,
            completion_timeout_secs: None,
        });

        let identity_key = "test_identity".to_string();
        let result = validate_request(&invoice, &request, &identity_key);
        assert!(
            result.is_err(),
            "Should fail when conversion to Bitcoin is provided"
        );
    }
}
