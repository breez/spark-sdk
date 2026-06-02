//! Shared validation building blocks for the send-payment prepare flow.
//!
//! These are the cross-cutting checks that every payment type enforces. Each
//! per-type `prepare/<type>.rs::validate_request` calls them first, so the
//! complete set of rules for an input type is visible in that type's own file.

use crate::{ConversionOptions, ConversionType, FeePolicy, error::SdkError};

/// Validates that amount is > 0 if provided.
pub(in crate::sdk) fn validate_amount(amount: Option<u128>) -> Result<(), SdkError> {
    if let Some(0) = amount {
        return Err(SdkError::InvalidInput(
            "Amount must be greater than 0".to_string(),
        ));
    }
    Ok(())
}

/// Validates that `FeesIncluded` is not combined with a `FromBitcoin` conversion.
/// `FeesIncluded` + `ToBitcoin` is allowed (send-all-with-conversion from stable balance).
pub(in crate::sdk) fn validate_fee_policy_for_conversion(
    fee_policy: Option<FeePolicy>,
    conversion_options: Option<&ConversionOptions>,
) -> Result<(), SdkError> {
    if fee_policy == Some(FeePolicy::FeesIncluded)
        && conversion_options.is_some()
        && !matches!(
            conversion_options,
            Some(ConversionOptions {
                conversion_type: ConversionType::ToBitcoin { .. },
                ..
            })
        )
    {
        return Err(SdkError::InvalidInput(
            "FeesIncluded cannot be combined with FromBitcoin conversion".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use macros::test_all;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    fn from_bitcoin_options() -> ConversionOptions {
        ConversionOptions {
            conversion_type: ConversionType::FromBitcoin,
            max_slippage_bps: None,
            completion_timeout_secs: None,
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

    #[test_all]
    fn test_validate_amount_zero() {
        let result = validate_amount(Some(0));
        assert!(result.is_err(), "Should fail for zero amount");
        if let Err(SdkError::InvalidInput(msg)) = result {
            assert!(
                msg.contains("must be greater than 0"),
                "Error message should mention requirement"
            );
        } else {
            panic!("Expected InvalidInput error");
        }
    }

    #[test_all]
    fn test_validate_amount_none_and_positive_ok() {
        assert!(validate_amount(None).is_ok());
        assert!(validate_amount(Some(1)).is_ok());
    }

    #[test_all]
    fn test_validate_fees_included_with_from_bitcoin_conversion_fails() {
        let result = validate_fee_policy_for_conversion(
            Some(FeePolicy::FeesIncluded),
            Some(&from_bitcoin_options()),
        );
        assert!(
            result.is_err(),
            "Should fail for FeesIncluded with FromBitcoin conversion"
        );
        if let Err(SdkError::InvalidInput(msg)) = result {
            assert!(
                msg.contains("FeesIncluded cannot be combined with FromBitcoin conversion"),
                "Error message should mention FeesIncluded and FromBitcoin conversion"
            );
        } else {
            panic!("Expected InvalidInput error");
        }
    }

    #[test_all]
    fn test_validate_fees_included_with_to_bitcoin_conversion_succeeds() {
        assert!(
            validate_fee_policy_for_conversion(
                Some(FeePolicy::FeesIncluded),
                Some(&to_bitcoin_options())
            )
            .is_ok(),
            "Should succeed for FeesIncluded with ToBitcoin conversion (send-all-with-conversion)"
        );
    }

    #[test_all]
    fn test_validate_fee_policy_no_conversion_ok() {
        // FeesIncluded with no conversion options is fine.
        assert!(validate_fee_policy_for_conversion(Some(FeePolicy::FeesIncluded), None).is_ok());
    }
}
