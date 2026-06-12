use std::str::FromStr;

use spark_wallet::{ExitSpeed, TransferId};

use crate::{
    BitcoinAddressDetails, ConversionOptions, ConversionPurpose, FeePolicy,
    OnchainConfirmationSpeed, SendOnchainFeeQuote, SendPaymentOptions,
    error::SdkError,
    models::{Payment, SendPaymentRequest, SendPaymentResponse},
    sdk::BreezSdk,
    token_conversion::{ConversionAmount, TokenConversionResponse},
    utils::bitcoin_dust::get_dust_limit_sats,
};

pub(super) async fn send(
    sdk: &BreezSdk,
    address: &BitcoinAddressDetails,
    fee_quote: &SendOnchainFeeQuote,
    request: &SendPaymentRequest,
    amount_override: Option<u64>,
) -> Result<SendPaymentResponse, SdkError> {
    // Extract confirmation speed from options
    let confirmation_speed = match &request.options {
        Some(SendPaymentOptions::BitcoinAddress { confirmation_speed }) => {
            confirmation_speed.clone()
        }
        None => OnchainConfirmationSpeed::Fast, // Default to fast
        _ => {
            return Err(SdkError::InvalidInput(
                "Invalid options for Bitcoin address payment".to_string(),
            ));
        }
    };

    let exit_speed: ExitSpeed = confirmation_speed.clone().into();

    let fee_sats = fee_for_speed(fee_quote, &confirmation_speed);

    // Compute amount - for FeesIncluded, receiver gets total minus fees.
    // amount_override (send-all post-conversion) is always FeesIncluded.
    let total_sats: u64 = amount_override.unwrap_or(request.prepare_response.amount.try_into()?);
    let amount_sats = if request.prepare_response.fee_policy == FeePolicy::FeesIncluded {
        total_sats.saturating_sub(fee_sats)
    } else {
        total_sats
    };

    // Validate the output amount meets the dust limit for this address type
    let dust_limit_sats = get_dust_limit_sats(&address.address)?;
    if amount_sats < dust_limit_sats {
        return Err(SdkError::InvalidInput(format!(
            "Amount is below the minimum of {dust_limit_sats} sats required for this address"
        )));
    }

    let transfer_id = request
        .idempotency_key
        .as_ref()
        .map(|idempotency_key| TransferId::from_str(idempotency_key))
        .transpose()?;
    let response = sdk
        .spark_wallet
        .withdraw(
            &address.address,
            Some(amount_sats),
            exit_speed,
            fee_quote.clone().into(),
            transfer_id,
        )
        .await?;

    let payment: Payment = response.try_into()?;

    sdk.storage.apply_payment_update(payment.clone()).await?;

    Ok(SendPaymentResponse { payment })
}

/// Runs the token conversion for a Bitcoin-address send, returning the conversion
/// response and its `OngoingPayment` purpose. `AmountIn` passes through;
/// `MinAmountOut` is expanded to cover the on-chain fee for the selected speed.
pub(in crate::sdk::payments) async fn convert_token(
    sdk: &BreezSdk,
    conversion_options: &ConversionOptions,
    address: &BitcoinAddressDetails,
    fee_quote: &SendOnchainFeeQuote,
    request: &SendPaymentRequest,
    token_identifier: Option<&String>,
    conversion_amount: ConversionAmount,
) -> Result<(TokenConversionResponse, ConversionPurpose), SdkError> {
    let purpose = ConversionPurpose::OngoingPayment {
        payment_request: address.address.clone(),
    };

    let conversion_amount = match conversion_amount {
        ConversionAmount::AmountIn(_) => conversion_amount,
        ConversionAmount::MinAmountOut(amount) => {
            // Derive fee_sats from request.options confirmation speed (default: Fast).
            let speed = match &request.options {
                Some(SendPaymentOptions::BitcoinAddress { confirmation_speed }) => {
                    confirmation_speed.clone()
                }
                _ => OnchainConfirmationSpeed::Fast,
            };
            let fee_sats = fee_for_speed(fee_quote, &speed);
            // The absolute minimum amount out is the amount plus fee
            ConversionAmount::MinAmountOut(amount.saturating_add(u128::from(fee_sats)))
        }
    };

    let response = sdk
        .token_converter
        .convert(
            sdk.event_emitter.clone(),
            conversion_options,
            &purpose,
            token_identifier,
            conversion_amount,
            None,
        )
        .await?;
    Ok((response, purpose))
}

/// Returns the total fee (sats) for the requested confirmation speed.
fn fee_for_speed(fee_quote: &SendOnchainFeeQuote, speed: &OnchainConfirmationSpeed) -> u64 {
    match speed {
        OnchainConfirmationSpeed::Fast => fee_quote.speed_fast.total_fee_sat(),
        OnchainConfirmationSpeed::Medium => fee_quote.speed_medium.total_fee_sat(),
        OnchainConfirmationSpeed::Slow => fee_quote.speed_slow.total_fee_sat(),
    }
}

#[cfg(test)]
mod tests {
    use super::fee_for_speed;
    use crate::{OnchainConfirmationSpeed, SendOnchainFeeQuote, SendOnchainSpeedFeeQuote};
    use macros::test_all;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    fn quote_with_speeds(slow: u64, medium: u64, fast: u64) -> SendOnchainFeeQuote {
        let speed = |total: u64| SendOnchainSpeedFeeQuote {
            user_fee_sat: total,
            l1_broadcast_fee_sat: 0,
        };
        SendOnchainFeeQuote {
            id: "test".to_string(),
            expires_at: 0,
            speed_slow: speed(slow),
            speed_medium: speed(medium),
            speed_fast: speed(fast),
        }
    }

    #[test_all]
    fn test_fee_for_speed_slow() {
        let quote = quote_with_speeds(100, 200, 300);
        assert_eq!(fee_for_speed(&quote, &OnchainConfirmationSpeed::Slow), 100);
    }

    #[test_all]
    fn test_fee_for_speed_medium() {
        let quote = quote_with_speeds(100, 200, 300);
        assert_eq!(
            fee_for_speed(&quote, &OnchainConfirmationSpeed::Medium),
            200
        );
    }

    #[test_all]
    fn test_fee_for_speed_fast() {
        let quote = quote_with_speeds(100, 200, 300);
        assert_eq!(fee_for_speed(&quote, &OnchainConfirmationSpeed::Fast), 300);
    }
}
