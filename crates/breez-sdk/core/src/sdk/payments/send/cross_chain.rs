use crate::{
    ConversionOptions, SendPaymentMethod, SendPaymentResponse,
    cross_chain::CrossChainPrepared,
    error::SdkError,
    sdk::{BreezSdk, SyncType},
    token_conversion::{ConversionAmount, ConversionPurpose, TokenConversionResponse},
};

/// Dispatches a `SendPaymentMethod::CrossChainAddress` to its provider.
///
/// The caller passes the variant by reference rather than destructured fields;
/// this fn rebuilds the [`CrossChainPrepared`] internally and hands it to the
/// provider's `send`, which polls to terminal and returns the [`Payment`].
pub(in crate::sdk) async fn send(
    sdk: &BreezSdk,
    method: &SendPaymentMethod,
    token_identifier: Option<String>,
    idempotency_key: Option<String>,
) -> Result<SendPaymentResponse, SdkError> {
    let SendPaymentMethod::CrossChainAddress {
        route,
        recipient_address,
        amount_in,
        estimated_out,
        fee_amount,
        fee_asset,
        source_transfer_fee_sats,
        fee_mode,
        expires_at,
        provider_context,
    } = method
    else {
        return Err(SdkError::Generic(
            "send::cross_chain::send called with non-cross-chain payment_method".to_string(),
        ));
    };

    let prepared = CrossChainPrepared {
        amount_in: *amount_in,
        estimated_out: *estimated_out,
        fee_amount: *fee_amount,
        fee_asset: fee_asset.clone(),
        source_transfer_fee_sats: *source_transfer_fee_sats,
        fee_mode: *fee_mode,
        expires_at: expires_at.clone(),
        pair: route.clone(),
        recipient_address: recipient_address.clone(),
        token_identifier: token_identifier.clone(),
        provider_context: provider_context.clone(),
    };

    // Token transfers may not trigger the same wallet event path as BTC
    // transfers — kick off a sync so the payment row is available for the
    // provider's downstream polling.
    if token_identifier.is_some() {
        sdk.sync_coordinator
            .trigger_sync_no_wait(SyncType::WalletState, true)
            .await;
    }

    // Each provider's `send()` polls its own outbound leg to terminal and
    // returns the corresponding `Payment`. The SDK no longer wraps this with
    // an extra `wait_for_payment` step — the provider owns that.
    let payment = sdk
        .cross_chain_providers
        .get(route.provider)?
        .send(&prepared, idempotency_key)
        .await?;

    Ok(SendPaymentResponse { payment })
}

/// Runs the AMM token conversion for a cross-chain send.
///
/// - **`AmountIn`**: converter's slippage floor is the prepare-time
///   `estimate.amount_out`. Pass through.
/// - **`MinAmountOut`**: pass through unchanged — the cross-chain provider's
///   `amount_in` already encodes the sats-leg target including the
///   provider-side `source_transfer_fee_sats`.
pub(in crate::sdk::payments) async fn convert_token(
    sdk: &BreezSdk,
    conversion_options: &ConversionOptions,
    payment_method: &SendPaymentMethod,
    token_identifier: Option<&String>,
    conversion_amount: ConversionAmount,
) -> Result<(TokenConversionResponse, ConversionPurpose), SdkError> {
    let recipient_address = match payment_method {
        SendPaymentMethod::CrossChainAddress {
            recipient_address, ..
        } => recipient_address.clone(),
        _ => {
            return Err(SdkError::Generic(
                "convert_token called with non-cross-chain payment_method".to_string(),
            ));
        }
    };

    let purpose = ConversionPurpose::OngoingPayment {
        payment_request: recipient_address,
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
