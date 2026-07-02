pub(super) mod bitcoin_address;
pub(in crate::sdk) mod bolt11;
pub(in crate::sdk::payments) mod cross_chain;
pub(super) mod spark_address;
pub(super) mod spark_invoice;

use crate::{
    ConversionEstimate, SendPaymentMethod,
    error::SdkError,
    events::SdkEvent,
    models::{
        PublishSignedTransferPackageResponse, SendPaymentRequest, SendPaymentResponse,
        SignedTransferPackage, TransferSignature, TransferTarget, UnsignedTransferPackage,
    },
    sdk::BreezSdk,
    signer::{ExternalPrepareTransferRequest, ExternalPreparedTransfer},
};

use super::conversion;

pub(in crate::sdk) async fn publish_signed_package_inner(
    sdk: &BreezSdk,
    signed_package: &SignedTransferPackage,
) -> Result<Option<SendPaymentResponse>, SdkError> {
    let res = match (&signed_package.unsigned, &signed_package.signature) {
        (UnsignedTransferPackage::Swap { .. }, TransferSignature::Transfer { .. }) => {
            super::client_signing::submit_swap(sdk, signed_package).await?;
            return Ok(None);
        }
        (
            UnsignedTransferPackage::Transfer {
                prepare_transfer,
                amount_sat,
                target,
                ..
            },
            TransferSignature::Transfer { signed },
        ) => deferred_transfer_send(sdk, prepare_transfer, signed, *amount_sat, target).await,
        (
            UnsignedTransferPackage::Token { token_context, .. },
            TransferSignature::Token { signed },
        ) => spark_address::send_token_signed(sdk, token_context, signed).await,
        _ => {
            return Err(SdkError::InvalidInput(
                "signature does not match the unsigned package".to_string(),
            ));
        }
    }?;
    Ok(Some(res))
}

pub(in crate::sdk::payments) async fn publish_signed_transfer_package(
    sdk: &BreezSdk,
    signed_package: &SignedTransferPackage,
) -> Result<PublishSignedTransferPackageResponse, SdkError> {
    if matches!(
        &signed_package.unsigned,
        UnsignedTransferPackage::Transfer {
            target: TransferTarget::Lightning {
                lnurl_pay: Some(_),
                ..
            },
            ..
        }
    ) {
        return Err(SdkError::InvalidInput(
            "LNURL pay packages must be published with publish_signed_lnurl_pay_package"
                .to_string(),
        ));
    }
    match publish_signed_package_inner(sdk, signed_package).await? {
        None => Ok(PublishSignedTransferPackageResponse::SwapCompleted),
        Some(res) => {
            sdk.event_emitter
                .emit(&SdkEvent::from_payment(res.payment.clone()))
                .await;
            Ok(PublishSignedTransferPackageResponse::PaymentSent {
                payment: res.payment,
            })
        }
    }
}

async fn deferred_transfer_send(
    sdk: &BreezSdk,
    prepare_transfer: &ExternalPrepareTransferRequest,
    signed: &ExternalPreparedTransfer,
    amount_sat: u64,
    target: &TransferTarget,
) -> Result<SendPaymentResponse, SdkError> {
    if let Ok(payment) = sdk
        .storage
        .get_payment_by_id(prepare_transfer.transfer_id.clone())
        .await
    {
        return Ok(SendPaymentResponse { payment });
    }

    match target {
        TransferTarget::Spark { spark_invoice, .. } => {
            spark_address::send_signed(sdk, prepare_transfer, signed, spark_invoice.clone()).await
        }
        TransferTarget::Lightning { bolt11, .. } => {
            bolt11::send_signed(sdk, prepare_transfer, signed, bolt11, amount_sat).await
        }
        TransferTarget::CoopExit { address, fee_quote } => {
            bitcoin_address::send_signed(
                sdk,
                prepare_transfer,
                signed,
                address,
                amount_sat,
                fee_quote,
            )
            .await
        }
    }
}

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
