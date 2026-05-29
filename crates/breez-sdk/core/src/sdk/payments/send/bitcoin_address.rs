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

    // Calculate fee based on selected speed
    let fee_sats = match confirmation_speed {
        OnchainConfirmationSpeed::Fast => fee_quote.speed_fast.total_fee_sat(),
        OnchainConfirmationSpeed::Medium => fee_quote.speed_medium.total_fee_sat(),
        OnchainConfirmationSpeed::Slow => fee_quote.speed_slow.total_fee_sat(),
    };

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
            // Derive fee_sats from request.options confirmation speed
            let fee_sats = match &request.options {
                Some(SendPaymentOptions::BitcoinAddress { confirmation_speed }) => {
                    match confirmation_speed {
                        OnchainConfirmationSpeed::Slow => fee_quote.speed_slow.total_fee_sat(),
                        OnchainConfirmationSpeed::Medium => fee_quote.speed_medium.total_fee_sat(),
                        OnchainConfirmationSpeed::Fast => fee_quote.speed_fast.total_fee_sat(),
                    }
                }
                _ => fee_quote.speed_fast.total_fee_sat(), // Default to fast
            };
            // The absolute minimum amount out is the amount plus fee
            ConversionAmount::MinAmountOut(amount.saturating_add(u128::from(fee_sats)))
        }
    };

    let response = sdk
        .token_converter
        .convert(
            conversion_options,
            &purpose,
            token_identifier,
            conversion_amount,
            None,
        )
        .await?;
    Ok((response, purpose))
}
