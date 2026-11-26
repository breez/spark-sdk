use std::ops::ControlFlow;
use std::str::FromStr;

use spark_wallet::{QuotedLightningSend, SendQuoteAmount, TransferId};
use tracing::info;

use crate::{
    Bolt11InvoiceDetails, ConversionOptions, ConversionPurpose, FeePolicy, LightningFeeQuote,
    SendPaymentMethod, SendPaymentOptions,
    error::SdkError,
    models::{Payment, PaymentStatus, SendPaymentRequest, SendPaymentResponse},
    sdk::{
        BreezSdk, InFlightSend, PendingLightningSend, Resumption, issued_quote,
        payments::{send_fee_quote, send_quote_amount},
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
    // A quote prices the Lightning route only.
    let fee_quote = fee_quote.filter(|_| !is_spark_route);
    let (transfer_id, _claim) =
        match retried_send(sdk, request, fee_quote, completion_timeout_secs).await? {
            ControlFlow::Break(payment) => return Ok(SendPaymentResponse { payment: *payment }),
            ControlFlow::Continue(send) => send,
        };
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
        unquoted_fee(sdk, invoice_details, fee_sats, amount, amount_override).await?
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

    let displayed_amount =
        displayed_send_amount(invoice_details, is_fees_included, amount_to_send, amount);
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
    // A package published before may have committed its leaves: that send is
    // resumed rather than refused, whether or not its quote has expired since.
    let _claim = match sdk
        .lightning_sender
        .resume_if_pending(
            &quote.transfer_id,
            completion_timeout_secs.unwrap_or(0).into(),
        )
        .await?
    {
        Resumption::Resumed(payment) => return Ok(SendPaymentResponse { payment: *payment }),
        Resumption::Send(claim) => claim,
    };
    // Checked before the send is recorded, so a package that lapsed during
    // approval leaves nothing for the resume loop.
    if quote.check_unexpired().is_err() {
        return Err(SdkError::quote_expired());
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

/// What an earlier send of a prepare response left under the transfer its fee
/// quote fixes, when it has no idempotency key.
pub(super) enum PriorSend {
    /// A send to resume.
    Pending,
    Recorded(Box<Payment>),
}

/// Finds what an earlier send of this unkeyed quoted prepare response left. A
/// failed payment is refused, since its quote is spent.
pub(super) async fn prior_send(
    sdk: &BreezSdk,
    request: &SendPaymentRequest,
) -> Result<Option<PriorSend>, SdkError> {
    let (
        None,
        SendPaymentMethod::Bolt11Invoice {
            fee_quote: Some(fee_quote),
            ..
        },
    ) = (
        &request.idempotency_key,
        &request.prepare_response.payment_method,
    )
    else {
        return Ok(None);
    };
    let transfer_id = quote_transfer_id(fee_quote)?;
    if sdk.lightning_sender.is_pending(&transfer_id).await? {
        return Ok(Some(PriorSend::Pending));
    }
    match sdk.storage.get_payment_by_id(transfer_id.to_string()).await {
        Ok(payment) if payment.status == PaymentStatus::Failed => Err(SdkError::InvalidInput(
            "This prepare response's payment failed: prepare the payment again".to_string(),
        )),
        Ok(payment) => Ok(Some(PriorSend::Recorded(Box::new(payment)))),
        Err(_) => Ok(None),
    }
}

fn quote_transfer_id(fee_quote: &LightningFeeQuote) -> Result<TransferId, SdkError> {
    spark_wallet::send_quote_transfer_id(&issued_quote(fee_quote)?)
        .map_err(|e| SdkError::InvalidInput(format!("Invalid fee quote: {e}")))
}

/// A retried send may have committed its leaves the first time, so a pending
/// send under its transfer id is resumed rather than sent again. That id is the
/// idempotency key when one is set, otherwise the one the prepared `fee_quote`
/// commits, under which a payment already recorded is returned. Otherwise the
/// transfer id to send under, with the guard holding it until the send is over.
async fn retried_send(
    sdk: &BreezSdk,
    request: &SendPaymentRequest,
    fee_quote: Option<&LightningFeeQuote>,
    completion_timeout_secs: Option<u32>,
) -> Result<ControlFlow<Box<Payment>, (Option<TransferId>, Option<InFlightSend>)>, SdkError> {
    let (transfer_id, keyed) = match (&request.idempotency_key, fee_quote) {
        (Some(idempotency_key), _) => (TransferId::from_str(idempotency_key)?, true),
        (None, Some(fee_quote)) => (quote_transfer_id(fee_quote)?, false),
        (None, None) => return Ok(ControlFlow::Continue((None, None))),
    };
    let claim = match sdk
        .lightning_sender
        .resume_if_pending(&transfer_id, completion_timeout_secs.unwrap_or(0).into())
        .await?
    {
        Resumption::Resumed(payment) => return Ok(ControlFlow::Break(payment)),
        Resumption::Send(claim) => claim,
    };
    // A resume on sync can finish the send after `prior_send` found it pending.
    if !keyed && let Some(PriorSend::Recorded(payment)) = prior_send(sdk, request).await? {
        return Ok(ControlFlow::Break(payment));
    }
    Ok(ControlFlow::Continue((Some(transfer_id), Some(claim))))
}

/// The amount an unquoted Lightning send records. Under `FeesIncluded` it is
/// the net reaching the receiver (`amount_to_send`), so amount and fees add
/// up to what the user paid as on the coop-exit, Spark and client-signing
/// paths, otherwise the requested `amount`. A fixed invoice receives its own
/// amount either way: an overpayment of it is paid to the SSP as fees, which
/// is how a sync records it too.
fn displayed_send_amount(
    invoice_details: &Bolt11InvoiceDetails,
    is_fees_included: bool,
    amount_to_send: Option<u128>,
    amount: u128,
) -> u128 {
    let displayed = if is_fees_included {
        amount_to_send.unwrap_or(amount)
    } else {
        amount
    };
    invoice_details
        .amount_msat
        .filter(|msat| *msat > 0)
        .map_or(displayed, |msat| {
            displayed.min(u128::from(msat.div_ceil(1000)))
        })
}

/// The routing fee to send a prepared quoted payment unquoted with, once the
/// SSP priced it no partner fee: a fresh estimate on the amount sent, which may
/// not exceed `prepared_fee` on the prepared `amount`.
async fn unquoted_fee(
    sdk: &BreezSdk,
    invoice_details: &Bolt11InvoiceDetails,
    prepared_fee: u64,
    amount: u128,
    amount_override: Option<u64>,
) -> Result<u64, SdkError> {
    let estimate = sdk
        .spark_wallet
        .fetch_lightning_send_fee_estimate(&invoice_details.invoice.bolt11, amount_override)
        .await?;
    let basis = amount_override.map_or(amount, u128::from);
    if !fee_within_prepared(
        prepared_fee,
        amount.try_into()?,
        estimate,
        basis.try_into()?,
        0,
    ) {
        return Err(SdkError::fees_changed());
    }
    Ok(estimate)
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
    // A conversion settles the amount of an amountless send, which the prepared
    // quote only priced as an estimate.
    match amount_override.filter(|_| invoice_details.amount_msat.is_none()) {
        Some(converted) => {
            converted_send_quote(
                sdk,
                invoice,
                send_quote_amount(invoice_details, fee_policy, converted.into())?,
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

/// A quote for an amountless send on the amount a conversion delivered. Its fee
/// may grow with the amount but not beyond what the prepare response showed on
/// the estimate, `prepared_fee` on `prepared_amount`. `None` when the SSP
/// prices no partner fee, and the send goes unquoted.
async fn converted_send_quote(
    sdk: &BreezSdk,
    invoice: &str,
    amount: SendQuoteAmount,
    prepared_fee: u64,
    prepared_amount: u64,
    transfer_id: Option<&TransferId>,
) -> Result<Option<QuotedLightningSend>, SdkError> {
    let Some(quote) = send_fee_quote(sdk, invoice, amount, transfer_id).await? else {
        return Ok(None);
    };
    let basis = match amount {
        SendQuoteAmount::Total(_) => quote.total_sats,
        SendQuoteAmount::Payment(_) | SendQuoteAmount::Invoice => quote.amount_sats,
    };
    let fee = quote.total_sats.saturating_sub(quote.amount_sats);
    let rounding_sats = u64::try_from(quote.buckets.len()).unwrap_or(u64::MAX);
    if !fee_within_prepared(prepared_fee, prepared_amount, fee, basis, rounding_sats) {
        return Err(SdkError::fees_changed());
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

    use super::displayed_send_amount;
    use crate::sdk::payments::prepare::test_helpers::create_test_bolt11_invoice;

    #[test_all]
    fn a_fixed_invoice_records_its_own_amount_however_much_is_sent() {
        let mut invoice = create_test_bolt11_invoice();
        invoice.amount_msat = Some(9_900_000);
        assert_eq!(
            displayed_send_amount(&invoice, true, Some(9_930), 9_970),
            9_900
        );
        assert_eq!(displayed_send_amount(&invoice, false, None, 9_900), 9_900);
    }

    #[test_all]
    fn an_amountless_invoice_records_what_reaches_the_receiver() {
        let invoice = create_test_bolt11_invoice();
        assert_eq!(
            displayed_send_amount(&invoice, true, Some(9_930), 9_970),
            9_930
        );
        assert_eq!(
            displayed_send_amount(&invoice, false, Some(5_000), 5_000),
            5_000
        );
    }
}
