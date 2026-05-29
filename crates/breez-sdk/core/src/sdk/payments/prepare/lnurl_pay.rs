use breez_sdk_common::lnurl::{
    error::LnurlError,
    pay::{ValidatedCallbackResponse, validate_lnurl_pay},
};
use tracing::info;

use crate::{
    ConversionEstimate, FeePolicy, InputType, PrepareLnurlPayRequest, PrepareLnurlPayResponse,
    SendPaymentMethod, error::SdkError, sdk::BreezSdk,
};

use super::super::conversion;

pub(in crate::sdk::payments) async fn prepare(
    sdk: &BreezSdk,
    request: PrepareLnurlPayRequest,
) -> Result<PrepareLnurlPayResponse, SdkError> {
    let fee_policy = request.fee_policy.unwrap_or_default();

    // For token conversions, the helper returns the raw estimated sats.
    // For plain LNURL pay (no conversion) it returns request.amount unchanged.
    let (estimated_sats, conversion_estimate) = conversion::estimate_sats_from_token_conversion(
        sdk,
        request.conversion_options.as_ref(),
        request.token_identifier.as_ref(),
        request.amount,
        fee_policy,
    )
    .await?;
    let is_token_conversion = conversion_estimate.is_some();

    // When token_identifier is set, the amount is in token units and the
    // conversion output (estimated_sats) is all we have to pay with — there
    // are no separate sats to cover fees. Force FeesIncluded so fees are
    // deducted from the conversion output. Reject explicit FeesExcluded.
    let (amount, fee_policy) = if is_token_conversion && request.token_identifier.is_some() {
        if fee_policy == FeePolicy::FeesExcluded {
            return Err(SdkError::InvalidInput(
                "Token conversion with token_identifier requires FeesIncluded fee policy"
                    .to_string(),
            ));
        }
        (estimated_sats, FeePolicy::FeesIncluded)
    } else {
        (estimated_sats, fee_policy)
    };

    // FeesIncluded uses the double-query approach
    if fee_policy == FeePolicy::FeesIncluded {
        let amount_sats: u64 = amount
            .try_into()
            .map_err(|_| SdkError::InvalidInput("Amount too large for LNURL".to_string()))?;
        return prepare_fees_included(sdk, request, amount_sats, conversion_estimate).await;
    }

    // Regular send (no FeesIncluded, no conversion)
    let amount_sats: u64 = amount
        .try_into()
        .map_err(|_| SdkError::InvalidInput("Amount too large for LNURL".to_string()))?;

    let success_data = match validate_lnurl_pay(
        sdk.lnurl_client.as_ref(),
        amount_sats.saturating_mul(1_000),
        &request.comment,
        &request.pay_request.clone().into(),
        sdk.config.network.into(),
        request.validate_success_action_url,
    )
    .await?
    {
        ValidatedCallbackResponse::EndpointError { data } => {
            return Err(LnurlError::EndpointError(data.reason).into());
        }
        ValidatedCallbackResponse::EndpointSuccess { data } => data,
    };

    let prepare_response = sdk
        .prepare_send_payment(crate::PrepareSendPaymentRequest {
            payment_request: success_data.pr,
            amount: Some(u128::from(amount_sats)),
            token_identifier: request.token_identifier.clone(),
            conversion_options: request.conversion_options.clone(),
            fee_policy: None,
        })
        .await?;

    let SendPaymentMethod::Bolt11Invoice {
        invoice_details,
        lightning_fee_sats,
        ..
    } = prepare_response.payment_method
    else {
        return Err(SdkError::Generic(
            "Expected Bolt11Invoice payment method".to_string(),
        ));
    };

    Ok(PrepareLnurlPayResponse {
        amount_sats,
        comment: request.comment,
        pay_request: request.pay_request,
        invoice_details,
        fee_sats: lightning_fee_sats,
        success_action: success_data.success_action.map(From::from),
        conversion_estimate: prepare_response.conversion_estimate,
        fee_policy,
    })
}

/// Prepares an LNURL pay `FeesIncluded` operation using a double-query approach.
///
/// This method:
/// 1. Validates amount doesn't exceed LNURL `max_sendable`
/// 2. First query: gets invoice for full amount to estimate fees
/// 3. Calculates actual send amount (amount - estimated fee)
/// 4. Second query: gets invoice for actual amount
/// 5. Returns the prepare response with the second invoice
async fn prepare_fees_included(
    sdk: &BreezSdk,
    request: PrepareLnurlPayRequest,
    amount_sats: u64,
    conversion_estimate: Option<ConversionEstimate>,
) -> Result<PrepareLnurlPayResponse, SdkError> {
    if amount_sats == 0 {
        return Err(SdkError::InvalidInput(
            "Amount must be greater than 0".to_string(),
        ));
    }

    // 1. Validate amount is within LNURL limits
    let min_sendable_sats = request.pay_request.min_sendable.div_ceil(1000);
    let max_sendable_sats = request.pay_request.max_sendable / 1000;

    if amount_sats < min_sendable_sats {
        return Err(SdkError::InvalidInput(format!(
            "Amount ({amount_sats} sats) is below LNURL minimum ({min_sendable_sats} sats)"
        )));
    }

    if amount_sats > max_sendable_sats {
        return Err(SdkError::InvalidInput(format!(
            "Amount ({amount_sats} sats) exceeds LNURL maximum ({max_sendable_sats} sats)"
        )));
    }

    // 2. First query: get invoice for full amount to estimate fees
    // Note: We don't intend to pay this invoice. It's only for fee estimation.
    let first_invoice = validate_lnurl_pay(
        sdk.lnurl_client.as_ref(),
        amount_sats.saturating_mul(1_000), // convert to msats
        &request.comment,
        &request.pay_request.clone().into(),
        sdk.config.network.into(),
        request.validate_success_action_url,
    )
    .await?;

    let first_data = match first_invoice {
        ValidatedCallbackResponse::EndpointError { data } => {
            return Err(LnurlError::EndpointError(data.reason).into());
        }
        ValidatedCallbackResponse::EndpointSuccess { data } => data,
    };

    // 3. Get fee estimate for first invoice
    let first_fee = sdk
        .spark_wallet
        .fetch_lightning_send_fee_estimate(&first_data.pr, None)
        .await?;

    // 4. Calculate actual send amount (amount - fee)
    let actual_amount = amount_sats.saturating_sub(first_fee);

    // Validate against LNURL minimum
    if actual_amount < min_sendable_sats {
        return Err(SdkError::InvalidInput(format!(
            "Amount after fees ({actual_amount} sats) is below LNURL minimum ({min_sendable_sats} sats)"
        )));
    }

    // 5. Second query: get invoice for actual amount (back-to-back, no delay)
    let success_data = match validate_lnurl_pay(
        sdk.lnurl_client.as_ref(),
        actual_amount.saturating_mul(1_000),
        &request.comment,
        &request.pay_request.clone().into(),
        sdk.config.network.into(),
        request.validate_success_action_url,
    )
    .await?
    {
        ValidatedCallbackResponse::EndpointError { data } => {
            return Err(LnurlError::EndpointError(data.reason).into());
        }
        ValidatedCallbackResponse::EndpointSuccess { data } => data,
    };

    // 6. Get actual fee for the smaller invoice
    let actual_fee = sdk
        .spark_wallet
        .fetch_lightning_send_fee_estimate(&success_data.pr, None)
        .await?;

    // If fee increased between queries, fail (user must retry)
    if actual_fee > first_fee {
        return Err(SdkError::Generic(
            "Fee increased between queries. Please retry.".to_string(),
        ));
    }

    // Parse the invoice to get details
    let parsed = sdk.parse(&success_data.pr).await?;
    let InputType::Bolt11Invoice(invoice_details) = parsed else {
        return Err(SdkError::Generic(
            "Expected Bolt11 invoice from LNURL".to_string(),
        ));
    };

    info!(
        "LNURL FeesIncluded prepared: amount={amount_sats}, receiver_amount={actual_amount}, fee={first_fee}"
    );

    Ok(PrepareLnurlPayResponse {
        amount_sats,
        comment: request.comment,
        pay_request: request.pay_request,
        invoice_details,
        fee_sats: first_fee,
        success_action: success_data.success_action.map(From::from),
        conversion_estimate,
        fee_policy: FeePolicy::FeesIncluded,
    })
}
