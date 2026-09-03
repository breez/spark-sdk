use std::str::FromStr;

use platform_utils::time::{SystemTime, UNIX_EPOCH};
use spark_wallet::{ExitSpeed, TransferId};
use tracing::{info, warn};

use crate::{
    BitcoinAddressDetails, ConversionOptions, ConversionPurpose, FeePolicy,
    OnchainConfirmationSpeed, SendOnchainFeeQuote, SendPaymentOptions,
    error::SdkError,
    models::{Payment, SendPaymentRequest, SendPaymentResponse},
    sdk::BreezSdk,
    signer::{ExternalPrepareTransferRequest, ExternalPreparedTransfer},
    token_conversion::{ConversionAmount, TokenConversionResponse},
    utils::bitcoin_dust::get_dust_limit_sats,
};

/// A quote within this buffer of expiry is treated as already dead and refetched.
/// The provider only checks expiry when it receives the quote, which is the first
/// call the withdrawal makes, so this covers the leaf reservation ahead of it and
/// any skew between the client clock and the provider's.
const FEE_QUOTE_EXPIRY_BUFFER_SECS: u64 = 10;

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

    // Compute amount - for FeesIncluded, receiver gets total minus fees.
    // amount_override (send-all post-conversion) is always FeesIncluded.
    let total_sats: u64 = amount_override.unwrap_or(request.prepare_response.amount.try_into()?);

    // What prepare put in front of the caller is the budget, whatever the quote
    // costs by the time the withdrawal goes out.
    let budget_sats = fee_for_speed(fee_quote, &confirmation_speed);
    let fee_quote = refreshed_fee_quote(sdk, address, fee_quote, total_sats).await?;
    let confirmation_speed = affordable_speed(&fee_quote, &confirmation_speed, budget_sats)?;
    let exit_speed: ExitSpeed = confirmation_speed.clone().into();
    let fee_sats = fee_for_speed(&fee_quote, &confirmation_speed);

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
            fee_quote.into(),
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

pub(super) async fn send_signed(
    sdk: &BreezSdk,
    prepare_transfer: &ExternalPrepareTransferRequest,
    signed: &ExternalPreparedTransfer,
    address: &str,
    amount_sat: u64,
    confirmation_speed: &OnchainConfirmationSpeed,
    fee_quote: &SendOnchainFeeQuote,
) -> Result<SendPaymentResponse, SdkError> {
    let transfer = sdk
        .spark_wallet
        .publish_coop_exit_package(
            prepare_transfer.transfer_id()?,
            prepare_transfer.leaf_ids()?,
            address,
            amount_sat,
            confirmation_speed.clone().into(),
            fee_quote.clone().into(),
            signed.to_prepared_transfer()?,
        )
        .await?;
    let payment: Payment = transfer.try_into()?;
    sdk.storage.apply_payment_update(payment.clone()).await?;
    Ok(SendPaymentResponse { payment })
}

/// Returns the quote to withdraw against, refetching the prepare-time one when it
/// is at or near expiry.
///
/// A token conversion runs between prepare and send and can outlive the quote's
/// roughly one-minute lifetime.
async fn refreshed_fee_quote(
    sdk: &BreezSdk,
    address: &BitcoinAddressDetails,
    prepared: &SendOnchainFeeQuote,
    total_sats: u64,
) -> Result<SendOnchainFeeQuote, SdkError> {
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| SdkError::Generic("Failed to read current time".to_string()))?
        .as_secs();
    if quote_is_valid(prepared.expires_at, now_secs, FEE_QUOTE_EXPIRY_BUFFER_SECS) {
        return Ok(prepared.clone());
    }

    let refreshed: SendOnchainFeeQuote = match sdk
        .spark_wallet
        .fetch_coop_exit_fee_quote(&address.address, Some(total_sats))
        .await
    {
        Ok(quote) => quote.into(),
        Err(e) => {
            // An estimate is not something the provider will honour, so there
            // is nothing to fall back to. Otherwise leave the provider to
            // adjudicate the old quote rather than stranding a balance a token
            // conversion has already spent.
            if prepared.is_estimate {
                return Err(e.into());
            }
            warn!(
                "Failed to refresh the expiring onchain fee quote, using the prepared one: {e:?}"
            );
            return Ok(prepared.clone());
        }
    };

    Ok(refreshed)
}

/// Whether `expires_at` (Unix seconds) still leaves `buffer_secs` of headroom.
fn quote_is_valid(expires_at: u64, now_secs: u64, buffer_secs: u64) -> bool {
    expires_at > now_secs.saturating_add(buffer_secs)
}

/// The speed to withdraw at: the one the caller chose, or the fastest cheaper
/// one that still fits the fee they were shown.
///
/// Speeds are priced relative to the current network rate, so when the rate
/// rises the same fee buys a lower-labelled speed. Stepping down holds the
/// caller to the fee they agreed while keeping the confirmation priority they
/// paid for, as long as the provider's speeds stay evenly spaced. Once even the
/// slowest speed costs more than that, failing is all
/// that is left, and on a conversion-funded send their tokens are already spent
/// by then.
fn affordable_speed(
    refreshed: &SendOnchainFeeQuote,
    selected: &OnchainConfirmationSpeed,
    budget_sats: u64,
) -> Result<OnchainConfirmationSpeed, SdkError> {
    let candidates: &[OnchainConfirmationSpeed] = match selected {
        OnchainConfirmationSpeed::Fast => &[
            OnchainConfirmationSpeed::Fast,
            OnchainConfirmationSpeed::Medium,
            OnchainConfirmationSpeed::Slow,
        ],
        OnchainConfirmationSpeed::Medium => &[
            OnchainConfirmationSpeed::Medium,
            OnchainConfirmationSpeed::Slow,
        ],
        OnchainConfirmationSpeed::Slow => &[OnchainConfirmationSpeed::Slow],
    };

    for (index, candidate) in candidates.iter().enumerate() {
        let fee_sats = fee_for_speed(refreshed, candidate);
        if fee_sats <= budget_sats {
            let stepped_down = index > 0;
            if stepped_down {
                info!(
                    "Onchain fee rose past the {budget_sats} sats quoted for {selected:?}, \
                     withdrawing at {candidate:?} for {fee_sats} sats instead"
                );
            }
            return Ok(candidate.clone());
        }
    }

    let slowest_fee_sats = fee_for_speed(refreshed, &OnchainConfirmationSpeed::Slow);
    Err(SdkError::InvalidInput(format!(
        "The onchain fee rose to {slowest_fee_sats} sats at the slowest speed, above the \
         {budget_sats} sats quoted for this payment. Please re-prepare the payment."
    )))
}

fn fee_for_speed(fee_quote: &SendOnchainFeeQuote, speed: &OnchainConfirmationSpeed) -> u64 {
    match speed {
        OnchainConfirmationSpeed::Fast => fee_quote.speed_fast.total_fee_sat(),
        OnchainConfirmationSpeed::Medium => fee_quote.speed_medium.total_fee_sat(),
        OnchainConfirmationSpeed::Slow => fee_quote.speed_slow.total_fee_sat(),
    }
}

#[cfg(test)]
mod tests {
    use super::{FEE_QUOTE_EXPIRY_BUFFER_SECS, affordable_speed, fee_for_speed, quote_is_valid};
    use crate::{
        OnchainConfirmationSpeed, SendOnchainFeeQuote, SendOnchainSpeedFeeQuote, error::SdkError,
    };
    use macros::test_all;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    const SPEEDS: [OnchainConfirmationSpeed; 3] = [
        OnchainConfirmationSpeed::Slow,
        OnchainConfirmationSpeed::Medium,
        OnchainConfirmationSpeed::Fast,
    ];

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
            is_estimate: false,
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

    // ============ quote_is_valid ============

    #[test_all]
    fn test_quote_is_valid_outside_the_buffer() {
        assert!(quote_is_valid(1000, 900, FEE_QUOTE_EXPIRY_BUFFER_SECS));
    }

    #[test_all]
    fn test_quote_is_valid_inside_the_buffer() {
        // Not yet expired, but dies mid-withdrawal.
        assert!(!quote_is_valid(1000, 990, FEE_QUOTE_EXPIRY_BUFFER_SECS));
    }

    #[test_all]
    fn test_quote_is_valid_boundary() {
        // Exactly `buffer` of headroom is not enough (the check is `>`).
        let now = 1000 - FEE_QUOTE_EXPIRY_BUFFER_SECS;
        assert!(!quote_is_valid(1000, now, FEE_QUOTE_EXPIRY_BUFFER_SECS));
        assert!(quote_is_valid(1000, now - 1, FEE_QUOTE_EXPIRY_BUFFER_SECS));
    }

    #[test_all]
    fn test_quote_is_valid_expired() {
        assert!(!quote_is_valid(1000, 1001, FEE_QUOTE_EXPIRY_BUFFER_SECS));
    }

    #[test_all]
    fn test_quote_is_valid_unset_expiry() {
        // An unset expiry reads as long dead, so the quote is refetched.
        assert!(!quote_is_valid(0, 1000, FEE_QUOTE_EXPIRY_BUFFER_SECS));
    }

    #[test_all]
    fn test_quote_is_valid_saturates() {
        assert!(!quote_is_valid(u64::MAX, u64::MAX, 1));
    }

    // ============ affordable_speed ============

    // Tiers sit one step (240 sats) apart, as the provider prices them.
    fn ladder(network_rate: u64) -> SendOnchainFeeQuote {
        let fee = |premium: u64| {
            network_rate
                .saturating_add(premium)
                .saturating_mul(240)
                .saturating_add(750)
        };
        quote_with_speeds(fee(1), fee(2), fee(3))
    }

    #[test_all]
    fn test_affordable_speed_keeps_the_chosen_one_when_the_fee_holds() {
        let refreshed = ladder(3);
        for speed in &SPEEDS {
            let budget = fee_for_speed(&refreshed, speed);
            let chosen = affordable_speed(&refreshed, speed, budget).unwrap();
            assert_eq!(fee_for_speed(&refreshed, &chosen), budget);
        }
    }

    #[test_all]
    fn test_affordable_speed_keeps_the_chosen_one_when_the_fee_falls() {
        // A cheaper refreshed quote is spent as-is, not rounded up to the budget.
        let prepared = ladder(3);
        let refreshed = ladder(1);
        let budget = fee_for_speed(&prepared, &OnchainConfirmationSpeed::Slow);
        let chosen = affordable_speed(&refreshed, &OnchainConfirmationSpeed::Slow, budget).unwrap();
        assert_eq!(
            fee_for_speed(&refreshed, &chosen),
            fee_for_speed(&refreshed, &OnchainConfirmationSpeed::Slow)
        );
    }

    #[test_all]
    fn test_affordable_speed_steps_down_rather_than_exceed_the_budget() {
        // The rate rose a step, so the budget now buys the tier below. The sats
        // per vbyte are the same as the caller paid for: only the label moves.
        let prepared = ladder(3);
        let refreshed = ladder(4);

        let budget = fee_for_speed(&prepared, &OnchainConfirmationSpeed::Fast);
        let chosen = affordable_speed(&refreshed, &OnchainConfirmationSpeed::Fast, budget).unwrap();
        assert!(matches!(chosen, OnchainConfirmationSpeed::Medium));
        assert_eq!(fee_for_speed(&refreshed, &chosen), budget);

        let budget = fee_for_speed(&prepared, &OnchainConfirmationSpeed::Medium);
        let chosen =
            affordable_speed(&refreshed, &OnchainConfirmationSpeed::Medium, budget).unwrap();
        assert!(matches!(chosen, OnchainConfirmationSpeed::Slow));
        assert_eq!(fee_for_speed(&refreshed, &chosen), budget);
    }

    #[test_all]
    fn test_affordable_speed_never_steps_up() {
        // A generous budget does not buy a faster confirmation than was asked for.
        let refreshed = ladder(3);
        let budget = fee_for_speed(&refreshed, &OnchainConfirmationSpeed::Fast);
        let chosen = affordable_speed(&refreshed, &OnchainConfirmationSpeed::Slow, budget).unwrap();
        assert!(matches!(chosen, OnchainConfirmationSpeed::Slow));
    }

    #[test_all]
    fn test_affordable_speed_fails_when_the_slowest_is_out_of_reach() {
        // Slow has nothing below it, so a rise leaves no tier within budget.
        let prepared = ladder(3);
        let refreshed = ladder(4);
        let budget = fee_for_speed(&prepared, &OnchainConfirmationSpeed::Slow);

        let Err(SdkError::InvalidInput(msg)) =
            affordable_speed(&refreshed, &OnchainConfirmationSpeed::Slow, budget)
        else {
            panic!("Expected InvalidInput when no speed fits the budget");
        };
        assert!(
            msg.contains("re-prepare"),
            "Error should tell the caller to re-prepare: {msg}"
        );
    }
}
