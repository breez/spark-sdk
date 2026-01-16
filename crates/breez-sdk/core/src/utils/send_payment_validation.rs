use crate::{
    ConversionOptions, ConversionType, InputType, SparkInvoiceDetails, error::SdkError,
    models::PrepareSendPaymentRequest,
};
use web_time::{Duration, SystemTime, UNIX_EPOCH};

/// Validates a send payment request against the parsed input type.
pub(crate) fn validate_prepare_send_payment_request(
    input_type: &InputType,
    request: &PrepareSendPaymentRequest,
    identity_public_key: &str,
) -> Result<(), SdkError> {
    match input_type {
        InputType::SparkInvoice(spark_invoice_details) => {
            validate_spark_invoice_request(spark_invoice_details, request, identity_public_key)
        }
        InputType::SparkAddress(_) => validate_spark_address_request(request),
        InputType::Bolt11Invoice(_) => validate_bolt11_invoice_request(request),
        InputType::BitcoinAddress(_) => validate_bitcoin_address_request(request),
        _ => Err(SdkError::InvalidInput(
            "Unsupported payment method".to_string(),
        )),
    }
}

/// Validates a spark invoice request against the provided request parameters.
fn validate_spark_invoice_request(
    spark_invoice_details: &SparkInvoiceDetails,
    request: &PrepareSendPaymentRequest,
    identity_public_key: &str,
) -> Result<(), SdkError> {
    // Validate token identifier
    if let Some(token_identifier) = &spark_invoice_details.token_identifier {
        let Some(requested_token_identifier) = &request.token_identifier else {
            return Err(SdkError::InvalidInput(
                "Token identifier is required for tokens invoice".to_string(),
            ));
        };
        if requested_token_identifier != token_identifier {
            return Err(SdkError::InvalidInput(
                "Requested token identifier does not match invoice token identifier".to_string(),
            ));
        }
        // Validate conversion to Bitcoin is not supported for tokens invoices
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
    } else if request.token_identifier.is_some() {
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
            format!("Invoice can only be paid by sender public key {sender_public_key}",)
                .to_string(),
        ));
    }

    // Validate amount
    if let Some(invoice_amount) = spark_invoice_details.amount
        && let Some(request_amount) = request.amount
        && invoice_amount != request_amount
    {
        return Err(SdkError::InvalidInput(
            "Requested amount does not match invoice amount".to_string(),
        ));
    }

    Ok(())
}

/// Validates a spark address request.
fn validate_spark_address_request(request: &PrepareSendPaymentRequest) -> Result<(), SdkError> {
    // Amount is required for spark addresses
    if request.amount.is_none() {
        return Err(SdkError::InvalidInput("Amount is required".to_string()));
    }
    // Validate conversion depending on whether token identifier is provided
    if let Some(conversion_options) = &request.conversion_options {
        match (
            &request.token_identifier,
            &conversion_options.conversion_type,
        ) {
            (Some(_), ConversionType::ToBitcoin { .. }) => {
                return Err(SdkError::InvalidInput(
                    "Conversion must be from Bitcoin when a token identifier is provided"
                        .to_string(),
                ));
            }
            (None, ConversionType::FromBitcoin) => {
                return Err(SdkError::InvalidInput(
                    "Conversion must be to Bitcoin when no token identifier is provided"
                        .to_string(),
                ));
            }
            _ => {}
        }
    }

    // Token identifier is optional for spark addresses
    Ok(())
}

/// Validates a Bolt11 invoice request.
fn validate_bolt11_invoice_request(request: &PrepareSendPaymentRequest) -> Result<(), SdkError> {
    // Token identifier cannot be provided for Bolt11 invoices
    if request.token_identifier.is_some() {
        return Err(SdkError::InvalidInput(
            "Token identifier can't be provided for this payment request: non-spark address"
                .to_string(),
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

/// Validates a Bitcoin address request.
fn validate_bitcoin_address_request(request: &PrepareSendPaymentRequest) -> Result<(), SdkError> {
    // Token identifier cannot be provided for Bitcoin addresses
    if request.token_identifier.is_some() {
        return Err(SdkError::InvalidInput(
            "Token identifier can't be provided for this payment request: non-spark address"
                .to_string(),
        ));
    }
    // Amount is required for Bitcoin addresses
    if request.amount.is_none() {
        return Err(SdkError::InvalidInput("Amount is required".to_string()));
    }
    // Validate conversion from Bitcoin is not supported for Bitcoin addresses
    if matches!(
        &request.conversion_options,
        Some(ConversionOptions {
            conversion_type: ConversionType::FromBitcoin,
            ..
        })
    ) {
        return Err(SdkError::InvalidInput(
            "Conversion must be to Bitcoin for Bitcoin addresses".to_string(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::PrepareSendPaymentRequest;
    use crate::{
        BitcoinAddressDetails, BitcoinNetwork, Bolt11InvoiceDetails, SparkAddressDetails,
        SparkInvoiceDetails,
    };
    use macros::test_all;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    fn create_test_request() -> PrepareSendPaymentRequest {
        PrepareSendPaymentRequest {
            payment_request: "test_request".to_string(),
            amount: None,
            token_identifier: None,
            conversion_options: None,
        }
    }

    fn create_test_invoice() -> SparkInvoiceDetails {
        SparkInvoiceDetails {
            invoice: "test_invoice".to_string(),
            identity_public_key: "test_identity_key".to_string(),
            network: BitcoinNetwork::Regtest,
            amount: None,
            token_identifier: None,
            expiry_time: None,
            description: None,
            sender_public_key: None,
        }
    }

    // SparkInvoice tests
    #[test_all]
    fn test_validate_spark_invoice_token_identifier_match() {
        let mut invoice = create_test_invoice();
        invoice.token_identifier = Some("token123".to_string());

        let mut request = create_test_request();
        request.token_identifier = Some("token123".to_string());

        let identity_key = "test_identity".to_string();
        let result = validate_spark_invoice_request(&invoice, &request, &identity_key);
        assert!(
            result.is_ok(),
            "Should succeed when token identifiers match"
        );
    }

    #[test_all]
    fn test_validate_spark_invoice_token_identifier_mismatch() {
        let mut invoice = create_test_invoice();
        invoice.token_identifier = Some("token123".to_string());

        let mut request = create_test_request();
        request.token_identifier = Some("token456".to_string());

        let identity_key = "test_identity".to_string();
        let result = validate_spark_invoice_request(&invoice, &request, &identity_key);
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
    fn test_validate_spark_invoice_token_identifier_required() {
        let mut invoice = create_test_invoice();
        invoice.token_identifier = Some("token123".to_string());

        let request = create_test_request(); // No token identifier

        let identity_key = "test_identity".to_string();
        let result = validate_spark_invoice_request(&invoice, &request, &identity_key);
        assert!(
            result.is_err(),
            "Should fail when token identifier is required but not provided"
        );
        if let Err(SdkError::InvalidInput(msg)) = result {
            assert!(
                msg.contains("Token identifier is required"),
                "Error message should mention requirement"
            );
        } else {
            panic!("Expected InvalidInput error");
        }
    }

    #[test_all]
    fn test_validate_spark_invoice_token_identifier_not_allowed() {
        let invoice = create_test_invoice(); // No token identifier

        let mut request = create_test_request();
        request.token_identifier = Some("token123".to_string());

        let identity_key = "test_identity".to_string();
        let result = validate_spark_invoice_request(&invoice, &request, &identity_key);
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
        let result = validate_spark_invoice_request(&invoice, &request, &identity_key);
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
        let future_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;
        invoice.expiry_time = Some(future_time);

        let request = create_test_request();
        let identity_key = "test_identity".to_string();
        let result = validate_spark_invoice_request(&invoice, &request, &identity_key);
        assert!(result.is_ok(), "Should succeed when invoice hasn't expired");
    }

    #[test_all]
    fn test_validate_spark_invoice_sender_public_key_match() {
        let mut invoice = create_test_invoice();
        invoice.sender_public_key = Some("sender_key123".to_string());

        let request = create_test_request();
        let identity_key = "sender_key123".to_string();
        let result = validate_spark_invoice_request(&invoice, &request, &identity_key);
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
        let result = validate_spark_invoice_request(&invoice, &request, &identity_key);
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

    #[test_all]
    fn test_validate_spark_invoice_amount_match() {
        let mut invoice = create_test_invoice();
        invoice.amount = Some(1000);

        let mut request = create_test_request();
        request.amount = Some(1000);

        let identity_key = "test_identity".to_string();
        let result = validate_spark_invoice_request(&invoice, &request, &identity_key);
        assert!(result.is_ok(), "Should succeed when amounts match");
    }

    #[test_all]
    fn test_validate_spark_invoice_amount_mismatch() {
        let mut invoice = create_test_invoice();
        invoice.amount = Some(1000);

        let mut request = create_test_request();
        request.amount = Some(2000);

        let identity_key = "test_identity".to_string();
        let result = validate_spark_invoice_request(&invoice, &request, &identity_key);
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
        let result = validate_spark_invoice_request(&invoice, &request, &identity_key);
        assert!(
            result.is_ok(),
            "Should succeed when only invoice has amount"
        );
    }

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

        let mut request = create_test_request();
        request.token_identifier = Some("token123".to_string());
        request.amount = Some(1000);

        let identity_key = "sender_key123".to_string();
        let result = validate_spark_invoice_request(&invoice, &request, &identity_key);
        assert!(result.is_ok(), "Should succeed when all validations pass");
    }

    #[test_all]
    fn test_validate_spark_invoice_with_valid_conversion() {
        let invoice = create_test_invoice();

        let mut request = create_test_request();
        request.conversion_options = Some(ConversionOptions {
            conversion_type: ConversionType::ToBitcoin {
                from_token_identifier: "token123".to_string(),
            },
            max_slippage_bps: None,
            completion_timeout_secs: None,
        });

        let identity_key = "test_identity".to_string();
        let result = validate_spark_invoice_request(&invoice, &request, &identity_key);
        assert!(
            result.is_ok(),
            "Should succeed when conversion to Bitcoin is provided"
        );
    }

    #[test_all]
    fn test_validate_token_spark_invoice_with_valid_conversion() {
        let mut invoice = create_test_invoice();
        invoice.token_identifier = Some("token123".to_string());

        let mut request = create_test_request();
        request.token_identifier = Some("token123".to_string());
        request.conversion_options = Some(ConversionOptions {
            conversion_type: ConversionType::FromBitcoin,
            max_slippage_bps: None,
            completion_timeout_secs: None,
        });

        let identity_key = "test_identity".to_string();
        let result = validate_spark_invoice_request(&invoice, &request, &identity_key);
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
        let result = validate_spark_invoice_request(&invoice, &request, &identity_key);
        assert!(
            result.is_err(),
            "Should fail when conversion from Bitcoin is provided"
        );
    }

    #[test_all]
    fn test_validate_token_spark_invoice_with_invalid_conversion() {
        let mut invoice = create_test_invoice();
        invoice.token_identifier = Some("token123".to_string());

        let mut request = create_test_request();
        request.token_identifier = Some("token123".to_string());
        request.conversion_options = Some(ConversionOptions {
            conversion_type: ConversionType::ToBitcoin {
                from_token_identifier: "token123".to_string(),
            },
            max_slippage_bps: None,
            completion_timeout_secs: None,
        });

        let identity_key = "test_identity".to_string();
        let result = validate_spark_invoice_request(&invoice, &request, &identity_key);
        assert!(
            result.is_err(),
            "Should fail when conversion to Bitcoin is provided"
        );
    }

    // SparkAddress tests
    #[test_all]
    fn test_validate_spark_address_with_amount() {
        let mut request = create_test_request();
        request.amount = Some(1000);
        let result = validate_spark_address_request(&request);
        assert!(result.is_ok(), "Should succeed when amount is provided");
    }

    #[test_all]
    fn test_validate_spark_address_without_amount() {
        let request = create_test_request(); // No amount
        let result = validate_spark_address_request(&request);
        assert!(result.is_err(), "Should fail when amount is not provided");
        if let Err(SdkError::InvalidInput(msg)) = result {
            assert!(
                msg.contains("Amount is required"),
                "Error message should mention requirement"
            );
        } else {
            panic!("Expected InvalidInput error");
        }
    }

    #[test_all]
    fn test_validate_spark_address_with_token_identifier() {
        let mut request = create_test_request();
        request.amount = Some(1000);
        request.token_identifier = Some("token123".to_string());
        let result = validate_spark_address_request(&request);
        assert!(
            result.is_ok(),
            "Should succeed when token identifier is provided (optional)"
        );
    }

    #[test_all]
    fn test_validate_spark_address_with_valid_conversion() {
        let mut request = create_test_request();
        request.amount = Some(1000);
        request.conversion_options = Some(ConversionOptions {
            conversion_type: ConversionType::ToBitcoin {
                from_token_identifier: "token123".to_string(),
            },
            max_slippage_bps: None,
            completion_timeout_secs: None,
        });
        let result = validate_spark_address_request(&request);
        assert!(
            result.is_ok(),
            "Should succeed when conversion to Bitcoin is provided"
        );
    }

    #[test_all]
    fn test_validate_token_spark_address_with_valid_conversion() {
        let mut request = create_test_request();
        request.amount = Some(1000);
        request.token_identifier = Some("token123".to_string());
        request.conversion_options = Some(ConversionOptions {
            conversion_type: ConversionType::FromBitcoin,
            max_slippage_bps: None,
            completion_timeout_secs: None,
        });
        let result = validate_spark_address_request(&request);
        assert!(
            result.is_ok(),
            "Should succeed when conversion from Bitcoin is provided"
        );
    }

    #[test_all]
    fn test_validate_spark_address_with_invalid_conversion() {
        let mut request = create_test_request();
        request.amount = Some(1000);
        request.conversion_options = Some(ConversionOptions {
            conversion_type: ConversionType::FromBitcoin,
            max_slippage_bps: None,
            completion_timeout_secs: None,
        });
        let result = validate_spark_address_request(&request);
        assert!(
            result.is_err(),
            "Should fail when conversion from Bitcoin is provided"
        );
    }

    #[test_all]
    fn test_validate_token_spark_address_with_invalid_conversion() {
        let mut request = create_test_request();
        request.amount = Some(1000);
        request.token_identifier = Some("token123".to_string());
        request.conversion_options = Some(ConversionOptions {
            conversion_type: ConversionType::ToBitcoin {
                from_token_identifier: "token123".to_string(),
            },
            max_slippage_bps: None,
            completion_timeout_secs: None,
        });
        let result = validate_spark_address_request(&request);
        assert!(
            result.is_err(),
            "Should fail when conversion to Bitcoin is provided"
        );
    }

    // Bolt11Invoice tests
    #[test_all]
    fn test_validate_bolt11_invoice_without_token_identifier() {
        let request = create_test_request();
        let result = validate_bolt11_invoice_request(&request);
        assert!(
            result.is_ok(),
            "Should succeed when token identifier is not provided"
        );
    }

    #[test_all]
    fn test_validate_bolt11_invoice_with_token_identifier() {
        let mut request = create_test_request();
        request.token_identifier = Some("token123".to_string());
        let result = validate_bolt11_invoice_request(&request);
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

    #[test_all]
    fn test_validate_bolt11_invoice_with_valid_conversion() {
        let mut request = create_test_request();
        request.conversion_options = Some(ConversionOptions {
            conversion_type: ConversionType::ToBitcoin {
                from_token_identifier: "token123".to_string(),
            },
            max_slippage_bps: None,
            completion_timeout_secs: None,
        });
        let result = validate_bolt11_invoice_request(&request);
        assert!(
            result.is_ok(),
            "Should succeed when conversion to Bitcoin is provided"
        );
    }

    #[test_all]
    fn test_validate_bolt11_invoice_with_invalid_conversion() {
        let mut request = create_test_request();
        request.conversion_options = Some(ConversionOptions {
            conversion_type: ConversionType::FromBitcoin,
            max_slippage_bps: None,
            completion_timeout_secs: None,
        });
        let result = validate_bolt11_invoice_request(&request);
        assert!(
            result.is_err(),
            "Should fail when conversion from Bitcoin is provided"
        );
    }

    // BitcoinAddress tests
    #[test_all]
    fn test_validate_bitcoin_address_with_amount() {
        let mut request = create_test_request();
        request.amount = Some(1000);
        let result = validate_bitcoin_address_request(&request);
        assert!(result.is_ok(), "Should succeed when amount is provided");
    }

    #[test_all]
    fn test_validate_bitcoin_address_without_amount() {
        let request = create_test_request(); // No amount
        let result = validate_bitcoin_address_request(&request);
        assert!(result.is_err(), "Should fail when amount is not provided");
        if let Err(SdkError::InvalidInput(msg)) = result {
            assert!(
                msg.contains("Amount is required"),
                "Error message should mention requirement"
            );
        } else {
            panic!("Expected InvalidInput error");
        }
    }

    #[test_all]
    fn test_validate_bitcoin_address_with_token_identifier() {
        let mut request = create_test_request();
        request.amount = Some(1000);
        request.token_identifier = Some("token123".to_string());
        let result = validate_bitcoin_address_request(&request);
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

    #[test_all]
    fn test_validate_bitcoin_address_with_valid_conversion() {
        let mut request = create_test_request();
        request.amount = Some(1000);
        request.conversion_options = Some(ConversionOptions {
            conversion_type: ConversionType::ToBitcoin {
                from_token_identifier: "token123".to_string(),
            },
            max_slippage_bps: None,
            completion_timeout_secs: None,
        });
        let result = validate_bitcoin_address_request(&request);
        assert!(
            result.is_ok(),
            "Should succeed when conversion to Bitcoin is provided"
        );
    }

    #[test_all]
    fn test_validate_bitcoin_address_with_invalid_conversion() {
        let mut request = create_test_request();
        request.amount = Some(1000);
        request.conversion_options = Some(ConversionOptions {
            conversion_type: ConversionType::FromBitcoin,
            max_slippage_bps: None,
            completion_timeout_secs: None,
        });
        let result = validate_bitcoin_address_request(&request);
        assert!(
            result.is_err(),
            "Should fail when conversion from Bitcoin is provided"
        );
    }

    // Integration tests using validate_send_payment_request
    #[test_all]
    fn test_validate_send_payment_spark_invoice() {
        let mut invoice = create_test_invoice();
        invoice.token_identifier = Some("token123".to_string());

        let mut request = create_test_request();
        request.token_identifier = Some("token123".to_string());

        let input_type = InputType::SparkInvoice(invoice);
        let identity_key = "test_identity".to_string();
        let result = validate_prepare_send_payment_request(&input_type, &request, &identity_key);
        assert!(result.is_ok(), "Should succeed for valid spark invoice");
    }

    #[test_all]
    fn test_validate_send_payment_spark_address() {
        use crate::PaymentRequestSource;
        let address_details = SparkAddressDetails {
            address: "test_address".to_string(),
            identity_public_key: "test_identity_key".to_string(),
            network: BitcoinNetwork::Regtest,
            source: PaymentRequestSource::default(),
        };

        let mut request = create_test_request();
        request.amount = Some(1000);

        let input_type = InputType::SparkAddress(address_details);
        let identity_key = "test_identity".to_string();
        let result = validate_prepare_send_payment_request(&input_type, &request, &identity_key);
        assert!(result.is_ok(), "Should succeed for valid spark address");
    }

    #[test_all]
    fn test_validate_send_payment_bolt11_invoice() {
        use crate::{Bolt11Invoice, PaymentRequestSource};
        let invoice_details = Bolt11InvoiceDetails {
            amount_msat: None,
            description: None,
            description_hash: None,
            expiry: 3600,
            invoice: Bolt11Invoice {
                bolt11: "lnbc1...".to_string(),
                source: PaymentRequestSource::default(),
            },
            min_final_cltv_expiry_delta: 144,
            network: BitcoinNetwork::Regtest,
            payee_pubkey: "test_pubkey".to_string(),
            payment_hash: "test_hash".to_string(),
            payment_secret: "test_secret".to_string(),
            routing_hints: vec![],
            timestamp: 0,
        };

        let request = create_test_request();

        let input_type = InputType::Bolt11Invoice(invoice_details);
        let identity_key = "test_identity".to_string();
        let result = validate_prepare_send_payment_request(&input_type, &request, &identity_key);
        assert!(result.is_ok(), "Should succeed for valid bolt11 invoice");
    }

    #[test_all]
    fn test_validate_send_payment_bitcoin_address() {
        use crate::PaymentRequestSource;
        let address_details = BitcoinAddressDetails {
            address: "bc1...".to_string(),
            network: BitcoinNetwork::Regtest,
            source: PaymentRequestSource::default(),
        };

        let mut request = create_test_request();
        request.amount = Some(1000);

        let input_type = InputType::BitcoinAddress(address_details);
        let identity_key = "test_identity".to_string();
        let result = validate_prepare_send_payment_request(&input_type, &request, &identity_key);
        assert!(result.is_ok(), "Should succeed for valid bitcoin address");
    }

    #[test_all]
    fn test_validate_send_payment_unsupported() {
        let request = create_test_request();
        let input_type = InputType::Url("https://example.com".to_string());
        let identity_key = "test_identity".to_string();
        let result = validate_prepare_send_payment_request(&input_type, &request, &identity_key);
        assert!(
            result.is_err(),
            "Should fail for unsupported payment method"
        );
        if let Err(SdkError::InvalidInput(msg)) = result {
            assert!(
                msg.contains("Unsupported payment method"),
                "Error message should mention unsupported method"
            );
        } else {
            panic!("Expected InvalidInput error");
        }
    }
}
