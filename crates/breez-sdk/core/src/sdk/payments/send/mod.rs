pub(super) mod bitcoin_address;
pub(in crate::sdk) mod bolt11;
pub(in crate::sdk::payments) mod cross_chain;
pub(super) mod spark_address;
pub(super) mod spark_invoice;

use crate::{
    ConversionEstimate, SendPaymentMethod,
    error::SdkError,
    events::SdkEvent,
    models::{SendPaymentRequest, SendPaymentResponse},
    sdk::BreezSdk,
};

use super::conversion;

// Top-level dispatcher for `send_payment`: routes between the convert-then-send
// pipeline and the direct send, then emits the payment event.
//
// Send-with-token-conversion pipeline (orchestrate_send → ...):
//
//   1. validate idempotency key + check existing payment by key
//   2. no conversion_estimate → send_internal directly, return
//   3. acquire stable-balance payment guard (suppresses auto-convert)
//   4. execute_pre_send_conversion: decide AmountIn vs MinAmountOut, dispatch to
//      send::<type>::convert_token (per-type fee shape)
//   5. pre_link_conversion_children (self-transfer only — parent known up front)
//   6. complete_conversion_and_send: wait for the conversion receive payment,
//      compute amount_override, send_internal, link children, persist Completed,
//      fetch payment with conversion details
//   7. emit payment event (unless suppressed)
pub(in crate::sdk) async fn orchestrate_send(
    sdk: &BreezSdk,
    request: SendPaymentRequest,
    mut suppress_payment_event: bool,
    amount_override: Option<u64>,
) -> Result<SendPaymentResponse, SdkError> {
    let token_identifier = request.prepare_response.token_identifier.clone();

    // Token transfers have no idempotency hook; retrying would re-spend the
    // source. Sats-only sends are covered by the provider's TransferId.
    let has_token_leg =
        token_identifier.is_some() || request.prepare_response.conversion_estimate.is_some();
    if request.idempotency_key.is_some() && has_token_leg {
        return Err(SdkError::InvalidInput(
            "Idempotency key is not supported for payments with a token \
             transfer leg (direct token send or AMM conversion)."
                .to_string(),
        ));
    }
    if let Some(idempotency_key) = &request.idempotency_key {
        // If an idempotency key is provided, check if a payment with that id already exists
        if let Ok(payment) = sdk.storage.get_payment_by_id(idempotency_key.clone()).await {
            return Ok(SendPaymentResponse { payment });
        }
    }
    let conversion_estimate = request.prepare_response.conversion_estimate.clone();
    // Perform the send payment, with conversion if requested
    let res = if let Some(ConversionEstimate {
        options: conversion_options,
        ..
    }) = &conversion_estimate
    {
        Box::pin(conversion::convert_token_send_payment_internal(
            sdk,
            conversion_options,
            &request,
            amount_override,
            &mut suppress_payment_event,
        ))
        .await
    } else {
        Box::pin(send_internal(sdk, &request, amount_override)).await
    };
    // Emit payment status event. Client runtime listens to payment events
    // and schedules a wallet-state refresh when background sync is active.
    if let Ok(response) = &res
        && !suppress_payment_event
    {
        // Emit the payment with metadata already included
        sdk.event_emitter
            .emit(&SdkEvent::from_payment(response.payment.clone()))
            .await;
    }
    res
}

pub(super) async fn send_internal(
    sdk: &BreezSdk,
    request: &SendPaymentRequest,
    amount_override: Option<u64>,
) -> Result<SendPaymentResponse, SdkError> {
    let amount = request.prepare_response.amount;
    let token_identifier = request.prepare_response.token_identifier.clone();

    match &request.prepare_response.payment_method {
        SendPaymentMethod::SparkAddress { address, .. } => {
            Box::pin(spark_address::send(
                sdk,
                address,
                token_identifier,
                amount_override.map_or(amount, u128::from),
                request.options.as_ref(),
                request.idempotency_key.clone(),
            ))
            .await
        }
        SendPaymentMethod::SparkInvoice {
            spark_invoice_details,
            ..
        } => {
            spark_invoice::send(
                sdk,
                &spark_invoice_details.invoice,
                request,
                amount_override.map_or(amount, u128::from),
            )
            .await
        }
        SendPaymentMethod::Bolt11Invoice {
            invoice_details,
            spark_transfer_fee_sats,
            lightning_fee_sats,
            ..
        } => {
            Box::pin(bolt11::send(
                sdk,
                invoice_details,
                *spark_transfer_fee_sats,
                *lightning_fee_sats,
                request,
                amount_override,
                amount,
            ))
            .await
        }
        SendPaymentMethod::BitcoinAddress { address, fee_quote } => {
            bitcoin_address::send(sdk, address, fee_quote, request, amount_override).await
        }
        method @ SendPaymentMethod::CrossChainAddress { .. } => {
            cross_chain::send(
                sdk,
                method,
                token_identifier,
                request.idempotency_key.clone(),
            )
            .await
        }
    }
}
