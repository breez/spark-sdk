use crate::{
    BitcoinAddressDetails, ConversionOptions, ConversionType, FeePolicy, SendOnchainFeeQuote,
    SendOnchainSpeedFeeQuote, SendPaymentMethod,
    error::SdkError,
    models::{PrepareSendPaymentRequest, PrepareSendPaymentResponse},
    sdk::BreezSdk,
    sdk::payments::{conversion, validation},
    token_conversion::ConversionAmount,
    utils::bitcoin_dust::get_dust_limit_sats,
};

// A wallet with nothing to price against cannot be quoted by the provider, which
// is the state a stable-balance wallet is in until its token conversion runs.
// These constants estimate the fee locally for that case.

/// How far the provider's per-speed rates sit above a high-priority rate,
/// indexed slow, medium, fast. Measured between 1 and 5 sat/vB, where the
/// network's block targets converge. Under congestion the provider may price
/// the speeds off targets further apart than this.
const ESTIMATE_SPEED_PREMIUM_SAT_PER_VBYTE: [u64; 3] = [1, 2, 3];
/// Headroom on top, for the rate moving while the conversion runs.
const ESTIMATE_DRIFT_HEADROOM_SAT_PER_VBYTE: u64 = 1;
/// The size the provider prices every withdrawal at, whatever the input count:
/// the wallet's funds are spent on the Spark side, not as on-chain inputs.
/// Documented at docs.spark.money/wallets/estimate-fees.
const ESTIMATE_VSIZE_VBYTES: u64 = 250;
/// The provider's flat service fee, the same for every speed and amount.
const ESTIMATE_USER_FEE_SAT: u64 = 750;

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

    // A wallet short of the amount funds the send by converting tokens, and the
    // SSP will not price an exit before that conversion has produced the sats,
    // so bound the fee locally and quote at send instead.
    let stable_balance_active = match &sdk.stable_balance {
        Some(sb) => sb.get_active_label().await.is_some(),
        None => false,
    };
    let balance_sats = sdk.spark_wallet.get_balance().await?;
    let conversion_funds_the_send = sats_from_conversion(
        stable_balance_active,
        request.conversion_options.as_ref(),
        balance_sats,
        amount_u64,
    );
    let fee_quote: SendOnchainFeeQuote = if conversion_funds_the_send && balance_sats == 0 {
        estimate_coop_exit_fee_quote(sdk).await?
    } else {
        // Short of the amount but holding something, the wallet still has funds
        // to price against, and the fee does not vary with what is selected.
        // With no conversion behind it, the send is priced against the amount
        // it will spend rather than every leaf the wallet holds.
        let target_sats = if conversion_funds_the_send {
            None
        } else {
            Some(amount_u64)
        };
        sdk.spark_wallet
            .fetch_coop_exit_fee_quote(&withdrawal_address.address, target_sats)
            .await?
            .into()
    };

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

    // Estimate only when the wallet holds nothing to price against. Any balance
    // at all, as a send-all wallet holding both tokens and sats has, gives a
    // real quote: the fee does not vary with what is selected.
    let fee_quote: SendOnchainFeeQuote = if sdk.spark_wallet.get_balance().await? == 0 {
        estimate_coop_exit_fee_quote(sdk).await?
    } else {
        sdk.spark_wallet
            .fetch_coop_exit_fee_quote(&withdrawal_address.address, None)
            .await?
            .into()
    };

    // Token-denominated converts the input into sats; fees come out of the
    // converted output, which is the FeesIncluded shape.
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

/// Estimates what a withdrawal will cost at each speed, for a wallet holding
/// nothing the provider can price against.
///
/// Each speed is estimated one rung high, so the estimate bounds the real fee
/// and the payment can be sized against it.
fn estimate_from_rate(fastest_fee_sat_per_vbyte: u64) -> SendOnchainFeeQuote {
    let speed = |premium: u64| SendOnchainSpeedFeeQuote {
        user_fee_sat: ESTIMATE_USER_FEE_SAT,
        l1_broadcast_fee_sat: fastest_fee_sat_per_vbyte
            .saturating_add(premium)
            .saturating_add(ESTIMATE_DRIFT_HEADROOM_SAT_PER_VBYTE)
            .saturating_mul(ESTIMATE_VSIZE_VBYTES),
    };
    SendOnchainFeeQuote {
        id: String::new(),
        expires_at: 0,
        speed_slow: speed(ESTIMATE_SPEED_PREMIUM_SAT_PER_VBYTE[0]),
        speed_medium: speed(ESTIMATE_SPEED_PREMIUM_SAT_PER_VBYTE[1]),
        speed_fast: speed(ESTIMATE_SPEED_PREMIUM_SAT_PER_VBYTE[2]),
        is_estimate: true,
    }
}

async fn estimate_coop_exit_fee_quote(sdk: &BreezSdk) -> Result<SendOnchainFeeQuote, SdkError> {
    let fees = sdk.chain_service.recommended_fees().await?;
    Ok(estimate_from_rate(fees.fastest_fee))
}

/// Whether the sats being sent will have to come from a token conversion.
///
/// The provider cannot price a cooperative exit without funds to quote against,
/// so this decides whether prepare quotes the fee or bounds it locally and
/// leaves the send to fetch a quote.
///
/// A conversion has to be in play, either auto-filled by an active stable
/// balance or asked for outright with `ToBitcoin` options, *and* the wallet has
/// to be short of the amount today. A wallet that can already cover it has
/// funds to quote against, whatever it converts afterwards.
fn sats_from_conversion(
    stable_balance_active: bool,
    conversion_options: Option<&ConversionOptions>,
    balance_sats: u64,
    amount_sats: u64,
) -> bool {
    (stable_balance_active || conversion::is_to_bitcoin(conversion_options))
        && balance_sats < amount_sats
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
    use super::{estimate_from_rate, sats_from_conversion, validate_dust, validate_request};
    use crate::{
        ConversionOptions, ConversionType, FeePolicy, OnchainConfirmationSpeed,
        SendOnchainFeeQuote, error::SdkError,
    };
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

    // ============ coop-exit fee estimate ============

    const SPEEDS: [OnchainConfirmationSpeed; 3] = [
        OnchainConfirmationSpeed::Slow,
        OnchainConfirmationSpeed::Medium,
        OnchainConfirmationSpeed::Fast,
    ];

    fn fee_for(quote: &SendOnchainFeeQuote, speed: &OnchainConfirmationSpeed) -> u64 {
        match speed {
            OnchainConfirmationSpeed::Slow => quote.speed_slow.total_fee_sat(),
            OnchainConfirmationSpeed::Medium => quote.speed_medium.total_fee_sat(),
            OnchainConfirmationSpeed::Fast => quote.speed_fast.total_fee_sat(),
        }
    }

    /// The step between the provider's speeds, measured at 240 sats. The
    /// estimate has to clear a real fee by at least this much to survive the
    /// rate moving a rung while the conversion runs, and by no more than two of
    /// them, or the conversion is sized against a fee nowhere near the real one.
    const MEASURED_RUNG_SAT: u64 = 240;

    /// Asserts the estimate bounds `measured` with between one and two rungs of
    /// headroom, rather than pinning an exact margin, which the rate moves.
    fn assert_bounds(estimated: u64, measured: u64, context: &str) {
        let margin = estimated.saturating_sub(measured);
        assert!(
            estimated >= measured.saturating_add(MEASURED_RUNG_SAT),
            "estimate of {estimated} should clear the {measured} sat fee by a \
             rung, {context}"
        );
        assert!(
            margin <= MEASURED_RUNG_SAT.saturating_mul(2),
            "estimate of {estimated} overshoots the {measured} sat fee by \
             {margin} sats, over two rungs, {context}"
        );
    }

    #[test_all]
    fn test_estimate_bounds_every_measured_fee() {
        // Total fees the mainnet provider quoted on 2026-08-28, by the
        // mempool.space high-priority rate at the time. Written out rather than
        // recomputed from the constants, so that changing any of them fails
        // here instead of scaling both sides of the comparison together.
        let measured: [(u64, [u64; 3]); 4] = [
            (2, [1470, 1710, 1950]),
            (3, [1710, 1950, 2190]),
            (4, [1950, 2190, 2430]),
            (5, [2190, 2430, 2670]),
        ];

        for (network_rate, fees) in measured {
            let quote = estimate_from_rate(network_rate);
            for (speed, measured_fee) in SPEEDS.iter().zip(fees) {
                assert_bounds(
                    fee_for(&quote, speed),
                    measured_fee,
                    &format!("measured at {network_rate} sat/vB for {speed:?}"),
                );
            }
        }
    }

    #[test_all]
    fn test_estimate_covers_the_fee_the_production_send_paid() {
        // The withdrawal that verified this fix paid 1,710 sats at the fast
        // speed with the network rate at 1 sat/vB.
        let quote = estimate_from_rate(1);
        assert_bounds(
            quote.speed_fast.total_fee_sat(),
            1710,
            "paid by the production send at 1 sat/vB for Fast",
        );
    }

    #[test_all]
    fn test_estimate_speeds_are_ordered() {
        let quote = estimate_from_rate(3);
        assert!(quote.speed_slow.total_fee_sat() < quote.speed_medium.total_fee_sat());
        assert!(quote.speed_medium.total_fee_sat() < quote.speed_fast.total_fee_sat());
    }

    #[test_all]
    fn test_estimate_is_marked_and_carries_no_provider_identity() {
        // `expires_at` of zero also reads as long expired, so the send refetches.
        let quote = estimate_from_rate(3);
        assert!(quote.is_estimate);
        assert!(quote.id.is_empty());
        assert_eq!(quote.expires_at, 0);
    }

    #[test_all]
    fn test_estimate_saturates() {
        let quote = estimate_from_rate(u64::MAX);
        assert!(quote.speed_fast.total_fee_sat() > 0);
    }

    // ============ sats_from_conversion ============

    // This predicate decides whether the prepare response carries a fee quote:
    // true means the sats do not exist yet, so the quote is fetched at send.

    fn to_bitcoin() -> ConversionOptions {
        ConversionOptions {
            conversion_type: ConversionType::ToBitcoin {
                from_token_identifier: "token123".to_string(),
            },
            max_slippage_bps: None,
            completion_timeout_secs: None,
        }
    }

    #[test_all]
    fn test_sats_from_conversion_stable_balance_and_short() {
        // The reported case: stable balance holding no sats at all.
        assert!(sats_from_conversion(true, None, 0, 10_000));
    }

    #[test_all]
    fn test_sats_from_conversion_explicit_to_bitcoin_and_short() {
        assert!(sats_from_conversion(false, Some(&to_bitcoin()), 0, 10_000));
    }

    #[test_all]
    fn test_sats_from_conversion_covered_balance_still_quotes() {
        // A wallet that can already cover the amount has funds to quote
        // against, so it keeps its prepare-time fee quote whether or not stable
        // balance is on or a conversion was asked for.
        assert!(!sats_from_conversion(true, None, 50_000, 10_000));
        assert!(!sats_from_conversion(
            false,
            Some(&to_bitcoin()),
            50_000,
            10_000
        ));
    }

    #[test_all]
    fn test_sats_from_conversion_boundary() {
        // Exactly covering the amount is covered: the check is `<`.
        assert!(!sats_from_conversion(true, None, 10_000, 10_000));
        assert!(sats_from_conversion(true, None, 9_999, 10_000));
    }

    #[test_all]
    fn test_sats_from_conversion_needs_a_conversion_in_play() {
        // Short of the amount with nothing to convert from is insufficient
        // funds, not a deferred quote.
        assert!(!sats_from_conversion(false, None, 0, 10_000));
    }

    #[test_all]
    fn test_sats_from_conversion_from_bitcoin_is_not_a_source() {
        // FromBitcoin spends sats to buy tokens, so it does not produce the
        // sats this send needs. `validate_request` rejects it for this
        // destination anyway.
        let options = ConversionOptions {
            conversion_type: ConversionType::FromBitcoin,
            max_slippage_bps: None,
            completion_timeout_secs: None,
        };
        assert!(!sats_from_conversion(false, Some(&options), 0, 10_000));
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
