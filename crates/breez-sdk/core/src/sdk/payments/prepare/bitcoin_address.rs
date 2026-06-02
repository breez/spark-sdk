use crate::{
    BitcoinAddressDetails, ConversionOptions, ConversionType, FeePolicy, SendOnchainFeeQuote,
    SendPaymentMethod,
    error::SdkError,
    models::{PrepareSendPaymentRequest, PrepareSendPaymentResponse},
    sdk::BreezSdk,
    sdk::payments::{conversion, validation},
    token_conversion::ConversionAmount,
    utils::bitcoin_dust::get_dust_limit_sats,
};

/// Validates a Bitcoin address request and returns the validated amount.
fn validate_request(request: &PrepareSendPaymentRequest) -> Result<u128, SdkError> {
    validation::validate_amount(request.amount)?;
    validation::validate_fee_policy_for_conversion(
        request.fee_policy,
        request.conversion_options.as_ref(),
    )?;

    // Token identifier cannot be provided for Bitcoin addresses unless ToBitcoin conversion
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

    // Amount is required for Bitcoin addresses
    let amount = request
        .amount
        .ok_or_else(|| SdkError::InvalidInput("Amount is required".to_string()))?;

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

    Ok(amount)
}

pub(super) async fn prepare(
    sdk: &BreezSdk,
    request: &PrepareSendPaymentRequest,
    withdrawal_address: &BitcoinAddressDetails,
    fee_policy: FeePolicy,
    token_identifier: Option<String>,
) -> Result<PrepareSendPaymentResponse, SdkError> {
    let amount = validate_request(request)?;

    if let Some(opts) = request.conversion_options.as_ref()
        && conversion::is_token_denominated(Some(amount), Some(opts), token_identifier.as_ref())
    {
        return prepare_token_denominated(
            sdk,
            opts,
            amount,
            withdrawal_address,
            token_identifier.as_ref(),
            fee_policy,
        )
        .await;
    }

    prepare_sats_denominated(
        sdk,
        amount,
        request,
        withdrawal_address,
        token_identifier,
        fee_policy,
    )
    .await
}

/// Sats-denominated Bitcoin-address prepare: `request.amount` is in sats. Validates
/// against the address dust limit (before fetching a fee quote, then again on the
/// post-fee output for `FeesIncluded`), fetches the coop-exit fee quote, and
/// attaches a `MinAmountOut` conversion estimate for display when conversion options
/// are set.
async fn prepare_sats_denominated(
    sdk: &BreezSdk,
    amount: u128,
    request: &PrepareSendPaymentRequest,
    withdrawal_address: &BitcoinAddressDetails,
    token_identifier: Option<String>,
    fee_policy: FeePolicy,
) -> Result<PrepareSendPaymentResponse, SdkError> {
    // Validate the amount meets the dust limit before making any network calls.
    // For FeesIncluded the output will be smaller after fees, but if the total
    // amount is already below dust there's no point fetching a fee quote.
    let dust_limit_sats = get_dust_limit_sats(&withdrawal_address.address)?;
    let amount_u64: u64 = amount.try_into()?;
    if amount_u64 < dust_limit_sats {
        return Err(SdkError::InvalidInput(format!(
            "Amount is below the minimum of {dust_limit_sats} sats required for this address"
        )));
    }

    // When a token→sats conversion will run (either auto-filled by an active
    // stable balance, or explicitly requested via `ToBitcoin` options), the
    // destination sats don't exist yet — pass None to skip leaf selection.
    let stable_balance_active = match &sdk.stable_balance {
        Some(sb) => sb.get_active_label().await.is_some(),
        None => false,
    };
    let sats_from_conversion =
        stable_balance_active || conversion::is_to_bitcoin(request.conversion_options.as_ref());
    let fee_quote_amount = if sats_from_conversion {
        None
    } else {
        Some(amount.try_into()?)
    };
    let fee_quote: SendOnchainFeeQuote = sdk
        .spark_wallet
        .fetch_coop_exit_fee_quote(&withdrawal_address.address, fee_quote_amount)
        .await?
        .into();

    // For FeesIncluded, validate the output after fees using the best case
    // (slow/lowest fee). Only reject if even the cheapest option results in dust.
    validate_dust(
        amount_u64,
        dust_limit_sats,
        fee_policy,
        fee_quote.speed_slow.total_fee_sat(),
    )?;

    // For conversion estimate, use fast fee as worst case
    let conversion_estimate = conversion::estimate_conversion(
        sdk,
        request.conversion_options.as_ref(),
        token_identifier.as_ref(),
        ConversionAmount::MinAmountOut(
            amount.saturating_add(u128::from(fee_quote.speed_fast.total_fee_sat())),
        ),
    )
    .await?;

    Ok(PrepareSendPaymentResponse {
        payment_method: SendPaymentMethod::BitcoinAddress {
            address: withdrawal_address.clone(),
            fee_quote,
        },
        amount,
        token_identifier,
        conversion_estimate,
        fee_policy,
    })
}

/// Token-denominated Bitcoin-address prepare: `token_amount` is in token base
/// units and `conversion_options` is `ToBitcoin`. Estimates the conversion, fetches
/// the onchain fee quote based on the estimated sats, and validates the output
/// after fees meets the dust limit.
///
/// Returns an explicit `InvalidInput` error when the converter can't validate the
/// requested conversion (rare — unsupported config / temporary outage). The
/// caller must not silently fall back to the sats-denominated path, since the
/// user's `token_amount` is in token units and would be misinterpreted as sats.
async fn prepare_token_denominated(
    sdk: &BreezSdk,
    conversion_options: &ConversionOptions,
    token_amount: u128,
    withdrawal_address: &BitcoinAddressDetails,
    token_identifier: Option<&String>,
    fee_policy: FeePolicy,
) -> Result<PrepareSendPaymentResponse, SdkError> {
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

    // Early dust check on the raw conversion output so we short-circuit
    // before the fee-quote network call when there's no chance of success.
    let dust_limit_sats = get_dust_limit_sats(&withdrawal_address.address)?;
    let total_u64: u64 = estimated_sats.try_into()?;
    if total_u64 < dust_limit_sats {
        return Err(SdkError::InvalidInput(format!(
            "Amount is below the minimum of {dust_limit_sats} sats required for this address"
        )));
    }

    // Pass None for amount — the sats don't exist yet (still tokens),
    // so leaf selection would fail. Get a generic fee estimate instead.
    let fee_quote: SendOnchainFeeQuote = sdk
        .spark_wallet
        .fetch_coop_exit_fee_quote(&withdrawal_address.address, None)
        .await?
        .into();

    // Token-denominated converts the input into sats; fees come out of the
    // converted output, which is the FeesIncluded shape — use the slow tier
    // as the best case for the post-fee dust check.
    validate_dust(
        total_u64,
        dust_limit_sats,
        FeePolicy::FeesIncluded,
        fee_quote.speed_slow.total_fee_sat(),
    )?;

    Ok(PrepareSendPaymentResponse {
        payment_method: SendPaymentMethod::BitcoinAddress {
            address: withdrawal_address.clone(),
            fee_quote,
        },
        amount: estimated_sats,
        // ToBitcoin conversion outputs sats — token_identifier is None
        token_identifier: None,
        conversion_estimate,
        fee_policy,
    })
}

/// Validates a Bitcoin send amount against the address dust limit.
///
/// Always rejects amounts below the dust limit. For `FeesIncluded`, also rejects
/// when the output after deducting `min_fee_sats` (the cheapest fee tier) would
/// dust — so callers should pass the lowest fee tier as the best case.
fn validate_dust(
    amount_sats: u64,
    dust_limit_sats: u64,
    fee_policy: FeePolicy,
    min_fee_sats: u64,
) -> Result<(), SdkError> {
    if amount_sats < dust_limit_sats {
        return Err(SdkError::InvalidInput(format!(
            "Amount is below the minimum of {dust_limit_sats} sats required for this address"
        )));
    }

    if fee_policy == FeePolicy::FeesIncluded {
        let output_amount_sats = amount_sats.saturating_sub(min_fee_sats);
        if output_amount_sats < dust_limit_sats {
            return Err(SdkError::InvalidInput(format!(
                "Amount is below the minimum of {dust_limit_sats} sats required for this address after lowest fees of {min_fee_sats} sats"
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::*;
    use super::{validate_dust, validate_request};
    use crate::{ConversionOptions, ConversionType, FeePolicy, error::SdkError};
    use macros::test_all;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    // ============ validate_request ============

    // ---- Amount required ----

    #[test_all]
    fn test_validate_bitcoin_address_with_amount() {
        let request = create_bitcoin_amount_request(1000);
        let result = validate_request(&request);
        assert!(result.is_ok(), "Should succeed when amount is provided");
    }

    #[test_all]
    fn test_validate_bitcoin_address_without_amount() {
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

    // ---- Token identifier requires ToBitcoin conversion ----

    #[test_all]
    fn test_validate_bitcoin_address_with_token_identifier() {
        let request = create_token_amount_request(1000, "token123");
        let result = validate_request(&request);
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

    // ---- FeesIncluded ----

    #[test_all]
    fn test_validate_bitcoin_address_with_fees_included() {
        let request = create_fees_included_request(1000);
        let result = validate_request(&request);
        assert!(
            result.is_ok(),
            "Should succeed when FeesIncluded is used for Bitcoin address"
        );
    }

    // ---- Conversion direction ----

    #[test_all]
    fn test_validate_bitcoin_address_with_valid_conversion() {
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
    fn test_validate_bitcoin_address_with_invalid_conversion() {
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

    // ============ validate_dust ============

    // ---- Base dust limit ----

    #[test_all]
    fn test_validate_dust_above_limit() {
        assert!(validate_dust(1000, 546, FeePolicy::FeesExcluded, 0).is_ok());
    }

    #[test_all]
    fn test_validate_dust_below_limit() {
        let result = validate_dust(500, 546, FeePolicy::FeesExcluded, 0);
        assert!(result.is_err(), "Should fail below dust limit");
        if let Err(SdkError::InvalidInput(msg)) = result {
            assert!(
                msg.contains("below the minimum") && !msg.contains("after lowest fees"),
                "Should use the base (pre-fee) message"
            );
        } else {
            panic!("Expected InvalidInput error");
        }
    }

    #[test_all]
    fn test_validate_dust_amount_equals_limit() {
        // Boundary: amount == dust is allowed (check is `<`, not `<=`).
        assert!(validate_dust(546, 546, FeePolicy::FeesExcluded, 0).is_ok());
    }

    // ---- FeesExcluded ignores fee ----

    #[test_all]
    fn test_validate_dust_fees_excluded_ignores_fee() {
        // FeesExcluded: a large fee is irrelevant as long as amount >= dust.
        assert!(validate_dust(600, 546, FeePolicy::FeesExcluded, 1000).is_ok());
    }

    // ---- FeesIncluded post-fee output ----

    #[test_all]
    fn test_validate_dust_fees_included_output_above_limit() {
        // 1000 - 400 = 600 >= 546 → ok.
        assert!(validate_dust(1000, 546, FeePolicy::FeesIncluded, 400).is_ok());
    }

    #[test_all]
    fn test_validate_dust_fees_included_output_equals_limit() {
        // Boundary: post-fee output exactly equals dust is allowed. 946 - 400 = 546.
        assert!(validate_dust(946, 546, FeePolicy::FeesIncluded, 400).is_ok());
    }

    #[test_all]
    fn test_validate_dust_fees_included_output_below_limit() {
        // 1000 - 500 = 500 < 546 → fail with the post-fee message.
        let result = validate_dust(1000, 546, FeePolicy::FeesIncluded, 500);
        assert!(result.is_err(), "Should fail when post-fee output dusts");
        if let Err(SdkError::InvalidInput(msg)) = result {
            assert!(
                msg.contains("after lowest fees of 500 sats"),
                "Should use the post-fee message"
            );
        } else {
            panic!("Expected InvalidInput error");
        }
    }

    #[test_all]
    fn test_validate_dust_fees_included_fee_exceeds_amount() {
        // min_fee_sats > amount_sats: output saturates to 0 (no underflow) and
        // dusts → error with the post-fee message.
        let result = validate_dust(600, 546, FeePolicy::FeesIncluded, 1000);
        assert!(result.is_err(), "Should fail when fee exceeds amount");
        if let Err(SdkError::InvalidInput(msg)) = result {
            assert!(
                msg.contains("after lowest fees of 1000 sats"),
                "Should use the post-fee message"
            );
        } else {
            panic!("Expected InvalidInput error");
        }
    }
}
