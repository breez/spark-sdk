use std::str::FromStr;

use spark_wallet::{QuotedLightningSend, SendQuoteAmount, TransferId};
use tracing::{debug, info, warn};

use crate::{
    Bolt11InvoiceDetails, ConversionOptions, ConversionPurpose, FeePolicy, LightningFeeQuote,
    SendPaymentOptions,
    error::SdkError,
    models::{SendPaymentRequest, SendPaymentResponse},
    sdk::{
        BreezSdk, PendingLightningSend, fees_changed, issued_quote, payments::send_quote_amount,
    },
    signer::{ExternalPrepareTransferRequest, ExternalPreparedTransfer, ExternalSignedQuote},
    token_conversion::{ConversionAmount, TokenConversionResponse},
    utils::fees::{fee_overpayment, fee_within_prepared},
};

#[expect(clippy::too_many_arguments)]
pub(super) async fn send(
    sdk: &BreezSdk,
    invoice_details: &Bolt11InvoiceDetails,
    spark_transfer_fee_sats: Option<u64>,
    lightning_fee_sats: u64,
    fee_quote: Option<&LightningFeeQuote>,
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
    // (unquoted send-all-with-conversion via LNURL: overpays the invoice to drain the wallet).
    let is_fees_included = request.prepare_response.fee_policy == FeePolicy::FeesIncluded;
    let transfer_id = request
        .idempotency_key
        .as_ref()
        .map(|idempotency_key| TransferId::from_str(idempotency_key))
        .transpose()?;

    // A quote prices the Lightning route only.
    let fee_quote = fee_quote.filter(|_| !is_spark_route);
    if let Some(quote) = send_quote(
        sdk,
        invoice_details,
        fee_quote,
        lightning_fee_sats,
        request,
        amount_override,
        amount,
        transfer_id.as_ref(),
    )
    .await?
    {
        let payment = sdk
            .lightning_sender
            .pay_and_persist_quoted_lightning_invoice(
                &invoice_details.invoice.bolt11,
                quote,
                completion_timeout_secs.unwrap_or(0).into(),
            )
            .await?;
        return Ok(SendPaymentResponse { payment });
    }

    // A fee priced from a quote that isn't committed carries its partner
    // markup, which an unquoted send must not pay as routing fees.
    let fee_sats = if fee_quote.is_some() {
        sdk.spark_wallet
            .fetch_lightning_send_fee_estimate(&invoice_details.invoice.bolt11, amount_override)
            .await?
    } else {
        fee_sats
    };
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
    let amount_to_send_sats = amount_to_send
        .map(|a| Ok::<u64, SdkError>(a.try_into()?))
        .transpose()?;

    // Under FeesIncluded, record the net amount reaching the receiver (the
    // fee-deducted amount_to_send) rather than the gross total, so amount + fees
    // equals what the user paid, consistent with the coop-exit, spark, and
    // client-signing paths. Other flows keep the requested amount: an unquoted
    // LNURL send runs internally as FeesExcluded and overpays the fixed invoice,
    // but that overpayment is paid to the SSP as fees, not to the receiver, so
    // the displayed amount must stay the invoice amount.
    let displayed_amount = if is_fees_included {
        amount_to_send.unwrap_or(amount)
    } else {
        amount
    };
    let payment = sdk
        .lightning_sender
        .pay_and_persist_lightning_invoice(
            &invoice_details.invoice.bolt11,
            amount_to_send_sats,
            fee_sats,
            prefer_spark,
            displayed_amount,
            transfer_id,
            completion_timeout_secs.unwrap_or(0).into(),
        )
        .await?;

    Ok(SendPaymentResponse { payment })
}

#[expect(clippy::too_many_arguments)]
pub(super) async fn send_signed(
    sdk: &BreezSdk,
    prepare_transfer: &ExternalPrepareTransferRequest,
    signed: &ExternalPreparedTransfer,
    bolt11: &str,
    amount_sat: u64,
    fee_sat: u64,
    fee_policy: FeePolicy,
    completion_timeout_secs: Option<u32>,
) -> Result<SendPaymentResponse, SdkError> {
    let amount_to_send = if fee_policy == FeePolicy::FeesIncluded {
        receiver_amount_with_overpayment(sdk, bolt11, amount_sat, fee_sat).await?
    } else {
        amount_sat
    };

    let transfer_id = prepare_transfer.transfer_id()?;
    let _in_flight = sdk
        .lightning_sender
        .record_pending_send(&PendingLightningSend {
            transfer_id: transfer_id.to_string(),
            invoice: bolt11.to_string(),
            amount_sats: Some(amount_to_send),
            displayed_amount: u128::from(amount_to_send),
            fee_quote: None,
        })
        .await;

    let result = Box::pin(sdk.spark_wallet.publish_lightning_send_package(
        transfer_id.clone(),
        prepare_transfer.leaf_ids()?,
        bolt11.to_string(),
        Some(amount_to_send),
        signed.to_prepared_transfer()?,
    ))
    .await?;
    sdk.lightning_sender
        .forget_pending_send(&transfer_id.to_string())
        .await;

    let payment = sdk
        .lightning_sender
        .payment_from_pay_result(
            result,
            u128::from(amount_to_send),
            completion_timeout_secs.unwrap_or(0).into(),
        )
        .await?;
    Ok(SendPaymentResponse { payment })
}

/// Publishes a client-signed Lightning send priced by `fee_quote`. The quote is
/// checked again, and the package must be the one built for it: the same
/// transfer, each leaf going to one of the quote's receivers.
#[expect(clippy::too_many_arguments)]
pub(super) async fn send_signed_quoted(
    sdk: &BreezSdk,
    prepare_transfer: &ExternalPrepareTransferRequest,
    signed: &ExternalPreparedTransfer,
    signed_quote: &ExternalSignedQuote,
    bolt11: &str,
    fee_quote: &LightningFeeQuote,
    amount_sat: u64,
    completion_timeout_secs: Option<u32>,
) -> Result<SendPaymentResponse, SdkError> {
    let quote = sdk
        .spark_wallet
        .verify_lightning_send_quote(
            bolt11,
            &issued_quote(fee_quote)?,
            fee_quote.payment_msats,
            SendQuoteAmount::Payment(amount_sat),
        )
        .await?;
    if prepare_transfer.transfer_id()? != quote.transfer_id {
        return Err(SdkError::InvalidInput(
            "The package was not built for its fee quote".to_string(),
        ));
    }
    let leaves = prepare_transfer
        .leaves
        .iter()
        .map(|leaf| Ok((leaf.node_id.to_tree_node_id()?, leaf.receiver_pubkey()?)))
        .collect::<Result<Vec<_>, SdkError>>()?;
    let manifest_hash_signature = signed_quote.to_signed_quote()?.signature;
    let displayed_amount = u128::from(quote.amount_sats);

    let _in_flight = sdk
        .lightning_sender
        .record_pending_send(&PendingLightningSend {
            transfer_id: quote.transfer_id.to_string(),
            invoice: bolt11.to_string(),
            amount_sats: quote.amount_to_send,
            displayed_amount,
            fee_quote: Some(fee_quote.clone()),
        })
        .await;

    let result = Box::pin(sdk.spark_wallet.publish_quoted_lightning_send_package(
        bolt11.to_string(),
        &quote,
        leaves,
        signed.to_prepared_transfer()?,
        manifest_hash_signature,
    ))
    .await?;
    sdk.lightning_sender
        .forget_pending_send(&quote.transfer_id.to_string())
        .await;

    let payment = sdk
        .lightning_sender
        .payment_from_pay_result(
            result,
            displayed_amount,
            completion_timeout_secs.unwrap_or(0).into(),
        )
        .await?;
    Ok(SendPaymentResponse { payment })
}

/// The quote to commit a Lightning send under: the prepared `fee_quote`, or a
/// new one when a conversion settled the amount it only estimated. `None` for
/// an unquoted send.
#[expect(clippy::too_many_arguments)]
async fn send_quote(
    sdk: &BreezSdk,
    invoice_details: &Bolt11InvoiceDetails,
    fee_quote: Option<&LightningFeeQuote>,
    lightning_fee_sats: u64,
    request: &SendPaymentRequest,
    amount_override: Option<u64>,
    amount: u128,
    transfer_id: Option<&TransferId>,
) -> Result<Option<QuotedLightningSend>, SdkError> {
    let Some(fee_quote) = fee_quote else {
        return Ok(None);
    };
    let fee_policy = request.prepare_response.fee_policy;
    let invoice = &invoice_details.invoice.bolt11;
    match converted_quote_amount(
        invoice_details.amount_msat,
        fee_policy == FeePolicy::FeesIncluded,
        amount_override,
    ) {
        Some(converted) => {
            converted_send_quote(
                sdk,
                invoice,
                converted,
                lightning_fee_sats,
                amount.try_into()?,
                transfer_id,
            )
            .await
        }
        None => Ok(Some(
            sdk.lightning_sender
                .committable_send_quote(
                    invoice,
                    fee_quote,
                    send_quote_amount(invoice_details, fee_policy, amount)?,
                    transfer_id,
                )
                .await?,
        )),
    }
}

/// What to quote an amountless send for once a conversion has settled its
/// amount: the prepared quote only priced the estimated amount. `None` when
/// the prepared quote names the amounts the send pays.
fn converted_quote_amount(
    invoice_amount_msat: Option<u64>,
    is_fees_included: bool,
    amount_override: Option<u64>,
) -> Option<SendQuoteAmount> {
    match (invoice_amount_msat, amount_override) {
        (None, Some(converted)) if is_fees_included => Some(SendQuoteAmount::Total(converted)),
        (None, Some(converted)) => Some(SendQuoteAmount::Payment(converted)),
        _ => None,
    }
}

/// A quote for an amountless send on the amount a conversion delivered. Its fee
/// may grow with the amount but not beyond what the prepare response showed on
/// the estimate, `prepared_fee` on `prepared_amount`. `None`, logged, when there
/// is no quote to be had or it carries no partner fee: the send goes unquoted.
async fn converted_send_quote(
    sdk: &BreezSdk,
    invoice: &str,
    amount: SendQuoteAmount,
    prepared_fee: u64,
    prepared_amount: u64,
    transfer_id: Option<&TransferId>,
) -> Result<Option<QuotedLightningSend>, SdkError> {
    let quote = match sdk
        .spark_wallet
        .fetch_lightning_send_quote(invoice, amount, transfer_id)
        .await
    {
        Ok(quote) if quote.carries_partner_fee() => quote,
        Ok(_) => {
            debug!("Sending without a fee quote: it carries no partner fee");
            return Ok(None);
        }
        Err(e) => {
            warn!("Sending without a fee quote: {e}");
            return Ok(None);
        }
    };
    let basis = match amount {
        SendQuoteAmount::Total(_) => quote.total_sats,
        SendQuoteAmount::Payment(_) | SendQuoteAmount::Invoice => quote.amount_sats,
    };
    let fee = quote.total_sats.saturating_sub(quote.amount_sats);
    let rounding_sats = u64::try_from(quote.buckets.len()).unwrap_or(u64::MAX);
    if !fee_within_prepared(prepared_fee, prepared_amount, fee, basis, rounding_sats) {
        return Err(fees_changed());
    }
    Ok(Some(quote))
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
    receiver_amount_with_overpayment(sdk, invoice, receiver_amount, stored_fee).await
}

async fn receiver_amount_with_overpayment(
    sdk: &BreezSdk,
    invoice: &str,
    receiver_amount: u64,
    stored_fee: u64,
) -> Result<u64, SdkError> {
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

#[cfg(test)]
mod tests {
    use macros::test_all;
    use spark_wallet::SendQuoteAmount;

    use super::converted_quote_amount;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    #[test_all]
    fn a_converted_amountless_send_is_quoted_on_the_converted_sats() {
        assert_eq!(
            converted_quote_amount(None, true, Some(5_000)),
            Some(SendQuoteAmount::Total(5_000))
        );
        assert_eq!(
            converted_quote_amount(None, false, Some(5_000)),
            Some(SendQuoteAmount::Payment(5_000))
        );
    }

    #[test_all]
    fn the_prepared_quote_stands_when_the_send_pays_what_it_names() {
        // No conversion settled the amount.
        assert_eq!(converted_quote_amount(None, true, None), None);
        // A fixed-amount invoice is paid its amount whatever was converted.
        assert_eq!(
            converted_quote_amount(Some(2_000_000), true, Some(5_000)),
            None
        );
        assert_eq!(
            converted_quote_amount(Some(2_000_000), false, Some(5_000)),
            None
        );
    }
}
