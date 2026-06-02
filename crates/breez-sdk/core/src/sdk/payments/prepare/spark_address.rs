use crate::{
    ConversionType, FeePolicy, SendPaymentMethod, SparkAddressDetails,
    error::SdkError,
    models::{PrepareSendPaymentRequest, PrepareSendPaymentResponse},
    sdk::BreezSdk,
    sdk::payments::{conversion, validation},
};

/// Validates a spark address request and returns the validated amount.
fn validate_request(request: &PrepareSendPaymentRequest) -> Result<u128, SdkError> {
    validation::validate_amount(request.amount)?;
    validation::validate_fee_policy_for_conversion(
        request.fee_policy,
        request.conversion_options.as_ref(),
    )?;

    // Amount is required for spark addresses
    let amount = request
        .amount
        .ok_or_else(|| SdkError::InvalidInput("Amount is required".to_string()))?;

    // Check if token identifier is provided
    let has_token_identifier = request.token_identifier.is_some();

    // Validate conversion depending on whether token identifier is provided.
    // token_identifier + ToBitcoin is allowed (send-all-with-conversion from stable balance).
    if let Some(conversion_options) = &request.conversion_options
        && !has_token_identifier
        && matches!(
            conversion_options.conversion_type,
            ConversionType::FromBitcoin
        )
    {
        return Err(SdkError::InvalidInput(
            "Conversion must be to Bitcoin when no token identifier is provided".to_string(),
        ));
    }

    // Token identifier is optional for spark addresses
    Ok(amount)
}

pub(super) async fn prepare(
    sdk: &BreezSdk,
    request: &PrepareSendPaymentRequest,
    details: &SparkAddressDetails,
    fee_policy: FeePolicy,
    token_identifier: Option<String>,
) -> Result<PrepareSendPaymentResponse, SdkError> {
    let amount = validate_request(request)?;

    let (amount, conversion_estimate) = conversion::resolve_send_amount_with_conversion_estimate(
        sdk,
        request.conversion_options.as_ref(),
        request.token_identifier.as_ref(),
        amount,
        fee_policy,
    )
    .await?;

    let response_token_identifier =
        conversion::response_token_identifier(conversion_estimate.as_ref(), token_identifier);

    Ok(PrepareSendPaymentResponse {
        payment_method: SendPaymentMethod::SparkAddress {
            address: details.address.clone(),
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

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    // ---- Amount required ----

    #[test_all]
    fn test_validate_spark_address_with_amount() {
        let request = create_bitcoin_amount_request(1000);
        let result = validate_request(&request);
        assert!(result.is_ok(), "Should succeed when amount is provided");
    }

    #[test_all]
    fn test_validate_spark_address_without_amount() {
        let request = create_test_request(); // No amount
        let result = validate_request(&request);
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

    // ---- Token identifier (optional for Spark address) ----

    #[test_all]
    fn test_validate_spark_address_with_token_identifier() {
        let request = create_token_amount_request(1000, "token123");
        let result = validate_request(&request);
        assert!(
            result.is_ok(),
            "Should succeed when token identifier is provided (optional)"
        );
    }

    // ---- FeesIncluded ----

    #[test_all]
    fn test_validate_spark_address_with_fees_included() {
        let request = create_fees_included_request(1000);
        let result = validate_request(&request);
        assert!(
            result.is_ok(),
            "Should succeed when FeesIncluded is used for Spark address"
        );
    }

    // ---- Conversion direction (no token identifier) ----

    #[test_all]
    fn test_validate_spark_address_with_valid_conversion() {
        let mut request = create_bitcoin_amount_request(1000);
        request.conversion_options = Some(ConversionOptions {
            conversion_type: ConversionType::ToBitcoin {
                from_token_identifier: "token123".to_string(),
            },
            max_slippage_bps: None,
            completion_timeout_secs: None,
        });
        let result = validate_request(&request);
        assert!(
            result.is_ok(),
            "Should succeed when conversion to Bitcoin is provided"
        );
    }

    #[test_all]
    fn test_validate_spark_address_with_invalid_conversion() {
        // FromBitcoin without token_identifier is invalid.
        let mut request = create_bitcoin_amount_request(1000);
        request.conversion_options = Some(ConversionOptions {
            conversion_type: ConversionType::FromBitcoin,
            max_slippage_bps: None,
            completion_timeout_secs: None,
        });
        let result = validate_request(&request);
        assert!(
            result.is_err(),
            "Should fail when conversion from Bitcoin is provided"
        );
    }

    // ---- Conversion direction (with token identifier) ----

    #[test_all]
    fn test_validate_spark_address_with_token_id_and_from_bitcoin_ok() {
        let mut request = create_token_amount_request(1000, "token123");
        request.conversion_options = Some(ConversionOptions {
            conversion_type: ConversionType::FromBitcoin,
            max_slippage_bps: None,
            completion_timeout_secs: None,
        });
        let result = validate_request(&request);
        assert!(
            result.is_ok(),
            "Should succeed when conversion from Bitcoin is provided with token identifier"
        );
    }

    #[test_all]
    fn test_validate_spark_address_with_token_id_and_to_bitcoin_ok() {
        let mut request = create_token_amount_request(1000, "token123");
        request.conversion_options = Some(ConversionOptions {
            conversion_type: ConversionType::ToBitcoin {
                from_token_identifier: "token123".to_string(),
            },
            max_slippage_bps: None,
            completion_timeout_secs: None,
        });
        let result = validate_request(&request);
        assert!(
            result.is_ok(),
            "Should succeed when ToBitcoin conversion is provided with token identifier (send-all-with-conversion)"
        );
    }
}
