use std::str::FromStr;

use spark_wallet::TransferId;
use tracing::info;

use crate::{
    Bolt11InvoiceDetails, ConversionOptions, ConversionPurpose, FeePolicy, SendPaymentOptions,
    error::SdkError,
    models::{SendPaymentRequest, SendPaymentResponse},
    sdk::BreezSdk,
    token_conversion::{ConversionAmount, TokenConversionResponse},
    utils::fees::fee_overpayment,
};

pub(super) async fn send(
    sdk: &BreezSdk,
    invoice_details: &Bolt11InvoiceDetails,
    spark_transfer_fee_sats: Option<u64>,
    lightning_fee_sats: u64,
    request: &SendPaymentRequest,
    amount_override: Option<u64>,
    amount: u128,
) -> Result<SendPaymentResponse, SdkError> {
    // Determine routing preference and actual fee before calculating the send amount,
    // so FeesIncluded deducts the correct fee (Spark=0 vs Lightning).
    let (prefer_spark, completion_timeout_secs) = match request.options {
        Some(SendPaymentOptions::Bolt11Invoice {
            prefer_spark,
            completion_timeout_secs,
        }) => (prefer_spark, completion_timeout_secs),
        _ => (sdk.config.prefer_spark_over_lightning, None),
    };
    let is_spark_route = prefer_spark && spark_transfer_fee_sats.is_some();
    let fee_sats = if is_spark_route {
        spark_transfer_fee_sats.unwrap_or(0)
    } else {
        lightning_fee_sats
    };

    // Handle FeesIncluded: deduct fees from the total balance.
    // Applies to both amountless invoices and fixed-amount invoices with amount_override
    // (send-all-with-conversion via LNURL — overpays the invoice to drain the wallet).
    let is_fees_included = request.prepare_response.fee_policy == FeePolicy::FeesIncluded;
    let amount_to_send = if is_fees_included
        && (invoice_details.amount_msat.is_none() || amount_override.is_some())
    {
        let total_sats: u64 = match amount_override {
            Some(sat_balance) => sat_balance,
            None => amount.try_into()?,
        };
        // Spark route: deduct known fee directly (often 0).
        // Lightning route: re-estimate fees via calculate_fees_included_amount
        // which handles fee changes between prepare and send.
        let amt = if is_spark_route {
            total_sats.saturating_sub(fee_sats)
        } else {
            calculate_fees_included_amount(
                sdk,
                &invoice_details.invoice.bolt11,
                total_sats,
                fee_sats,
            )
            .await?
        };
        Some(u128::from(amt))
    } else {
        match amount_override {
            Some(amt) => Some(amt.into()),
            None => match invoice_details.amount_msat {
                Some(_) => None,
                None => Some(amount),
            },
        }
    };
    let transfer_id = request
        .idempotency_key
        .as_ref()
        .map(|idempotency_key| TransferId::from_str(idempotency_key))
        .transpose()?;
    let amount_to_send_sats = amount_to_send
        .map(|a| Ok::<u64, SdkError>(a.try_into()?))
        .transpose()?;

    let payment = sdk
        .lightning_sender
        .pay_and_persist_lightning_invoice(
            &invoice_details.invoice.bolt11,
            amount_to_send_sats,
            fee_sats,
            prefer_spark,
            amount,
            transfer_id,
            completion_timeout_secs.unwrap_or(0).into(),
        )
        .await?;

    Ok(SendPaymentResponse { payment })
}

/// For `FeesIncluded` + amountless Bolt11: calculates the amount to send
/// (`receiver_amount` + any overpayment from fee decrease).
async fn calculate_fees_included_amount(
    sdk: &BreezSdk,
    invoice: &str,
    user_amount: u64,
    stored_fee: u64,
) -> Result<u64, SdkError> {
    let receiver_amount = user_amount.saturating_sub(stored_fee);
    if receiver_amount == 0 {
        return Err(SdkError::InvalidInput(
            "Amount too small to cover fees".to_string(),
        ));
    }

    // Re-estimate current fee for receiver amount
    let current_fee = sdk
        .spark_wallet
        .fetch_lightning_send_fee_estimate(invoice, Some(receiver_amount))
        .await?;

    let overpayment = fee_overpayment(stored_fee, current_fee)?;
    if overpayment > 0 {
        info!(
            overpayment_sats = overpayment,
            stored_fee_sats = stored_fee,
            current_fee_sats = current_fee,
            "FeesIncluded fee overpayment applied for Bolt11"
        );
    }

    Ok(receiver_amount.saturating_add(overpayment))
}

/// Runs the token conversion for a Bolt11 send, returning the conversion response
/// and its `OngoingPayment` purpose. `AmountIn` passes through; `MinAmountOut` is
/// expanded to cover the routing fee (Spark transfer when preferred and available,
/// otherwise Lightning) so the converter delivers enough to complete the send.
#[expect(clippy::too_many_arguments)]
pub(in crate::sdk::payments) async fn convert_token(
    sdk: &BreezSdk,
    conversion_options: &ConversionOptions,
    invoice_details: &Bolt11InvoiceDetails,
    spark_transfer_fee_sats: Option<u64>,
    lightning_fee_sats: u64,
    request: &SendPaymentRequest,
    token_identifier: Option<&String>,
    conversion_amount: ConversionAmount,
) -> Result<(TokenConversionResponse, ConversionPurpose), SdkError> {
    let purpose = ConversionPurpose::OngoingPayment {
        payment_request: invoice_details.invoice.bolt11.clone(),
    };

    let conversion_amount = match conversion_amount {
        ConversionAmount::AmountIn(_) => conversion_amount,
        ConversionAmount::MinAmountOut(amount) => {
            // Determine the fee to be used based on preference
            let fee_sats = match request.options {
                Some(SendPaymentOptions::Bolt11Invoice { prefer_spark, .. }) => {
                    match (prefer_spark, spark_transfer_fee_sats) {
                        (true, Some(fee)) => fee,
                        _ => lightning_fee_sats,
                    }
                }
                _ => lightning_fee_sats,
            };
            // The absolute minimum amount out is the lightning invoice amount plus fee
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
