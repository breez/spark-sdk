use crate::{
    FeePolicy, LnurlPayInfo, LnurlPayRequest, LnurlPayResponse, SendPaymentMethod,
    error::SdkError,
    events::SdkEvent,
    models::{PrepareSendPaymentResponse, SendPaymentRequest},
    persist::PaymentMetadata,
    sdk::{BreezSdk, helpers::process_success_action},
};

use super::super::conversion;

#[allow(clippy::too_many_lines)]
pub(in crate::sdk::payments) async fn send(
    sdk: &BreezSdk,
    request: LnurlPayRequest,
) -> Result<LnurlPayResponse, SdkError> {
    sdk.maybe_ensure_spark_private_mode_initialized().await?;

    let is_fees_included = request.prepare_response.fee_policy == FeePolicy::FeesIncluded;

    // For FeesIncluded, extract amount from the invoice (set during prepare)
    let receiver_amount_sats: u64 = if is_fees_included {
        request
            .prepare_response
            .invoice_details
            .amount_msat
            .ok_or_else(|| SdkError::Generic("Missing invoice amount".to_string()))?
            / 1000
    } else {
        request.prepare_response.amount_sats
    };

    // Calculate amount override for FeesIncluded operations
    let amount_override = if is_fees_included {
        // Re-estimate current fee for the invoice
        let current_fee = sdk
            .spark_wallet
            .fetch_lightning_send_fee_estimate(
                &request.prepare_response.invoice_details.invoice.bolt11,
                None,
            )
            .await?;

        // fees_included_fee = first_fee (from prepare), which is the total we need to pay in fees
        let fees_included_fee = request.prepare_response.fee_sats;

        // Reconcile the prepare-time fee against the re-estimated fee, overpaying
        // by the difference to respect the prepared amount.
        let overpayment = super::bolt11::fee_overpayment(fees_included_fee, current_fee)?;

        if overpayment > 0 {
            tracing::info!(
                overpayment_sats = overpayment,
                fees_included_fee_sats = fees_included_fee,
                current_fee_sats = current_fee,
                "FeesIncluded fee overpayment applied"
            );
        }
        Some(receiver_amount_sats.saturating_add(overpayment))
    } else {
        None
    };

    // For conversions, use FeesIncluded so the send path deducts fees from
    // the post-conversion balance. For non-conversion FeesIncluded, the LNURL
    // flow handles fees via invoice sizing and amount_override.
    let has_conversion = request.prepare_response.conversion_estimate.is_some();
    let internal_fee_policy = if is_fees_included && has_conversion {
        FeePolicy::FeesIncluded
    } else {
        FeePolicy::FeesExcluded
    };

    let mut payment = Box::pin(conversion::orchestrate_send(
        sdk,
        SendPaymentRequest {
            prepare_response: PrepareSendPaymentResponse {
                payment_method: SendPaymentMethod::Bolt11Invoice {
                    invoice_details: request.prepare_response.invoice_details,
                    spark_transfer_fee_sats: None,
                    lightning_fee_sats: request.prepare_response.fee_sats,
                },
                // For conversions, use the prepare's total amount (before fee
                // deduction) so the sats_change logic in complete_conversion_and_send
                // correctly computes the post-conversion amount override.
                // For non-conversions, use the invoice amount.
                amount: if has_conversion {
                    u128::from(request.prepare_response.amount_sats)
                } else {
                    u128::from(receiver_amount_sats)
                },
                // LNURL always sends sats — token_identifier is None on the
                // internal PrepareSendPaymentResponse even when a conversion
                // is present (the token info is in conversion_estimate).
                token_identifier: None,
                conversion_estimate: request.prepare_response.conversion_estimate,
                fee_policy: internal_fee_policy,
            },
            options: None,
            idempotency_key: request.idempotency_key,
        },
        true,
        // For conversions, don't pass amount_override — let
        // complete_conversion_and_send compute it from sats_change.
        // For non-conversions, use the LNURL-computed override.
        if has_conversion {
            None
        } else {
            amount_override
        },
    ))
    .await?
    .payment;

    let success_action = process_success_action(
        &payment,
        request
            .prepare_response
            .success_action
            .clone()
            .map(Into::into)
            .as_ref(),
    )?;

    let lnurl_info = LnurlPayInfo {
        ln_address: request.prepare_response.pay_request.address,
        comment: request.prepare_response.comment,
        domain: Some(request.prepare_response.pay_request.domain),
        metadata: Some(request.prepare_response.pay_request.metadata_str),
        processed_success_action: success_action.clone().map(From::from),
        raw_success_action: request.prepare_response.success_action,
    };
    let lnurl_description = lnurl_info.extract_description();

    match &mut payment.details {
        Some(crate::PaymentDetails::Lightning {
            lnurl_pay_info,
            description,
            ..
        }) => {
            *lnurl_pay_info = Some(lnurl_info.clone());
            description.clone_from(&lnurl_description);
        }
        // When the LNURL server includes a Spark routing hint, the payment
        // is routed via Spark transfer. The Spark variant doesn't carry
        // lnurl fields, so we just persist the metadata separately below.
        Some(crate::PaymentDetails::Spark { .. }) => {}
        _ => {
            return Err(SdkError::Generic(
                "Expected Lightning or Spark payment details".to_string(),
            ));
        }
    }

    sdk.storage
        .insert_payment_metadata(
            payment.id.clone(),
            PaymentMetadata {
                lnurl_pay_info: Some(lnurl_info),
                lnurl_description,
                ..Default::default()
            },
        )
        .await?;

    // Emit the payment with metadata already included
    sdk.event_emitter
        .emit(&SdkEvent::from_payment(payment.clone()))
        .await;
    Ok(LnurlPayResponse {
        payment,
        success_action: success_action.map(From::from),
    })
}
