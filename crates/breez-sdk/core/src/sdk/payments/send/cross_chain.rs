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
        asset_amount_in,
        estimated_out,
        fee_amount,
        service_fee_amount,
        service_fee_asset,
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
        asset_amount_in: *asset_amount_in,
        estimated_out: *estimated_out,
        fee_amount: *fee_amount,
        service_fee_amount: *service_fee_amount,
        service_fee_asset: service_fee_asset.clone(),
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
        .cross_chain_context
        .get(route.provider)?
        .send(&prepared, idempotency_key)
        .await?;

    Ok(SendPaymentResponse { payment })
}

/// Folds `source_transfer_fee_sats` into a `MinAmountOut` target so the AMM
/// covers both the provider invoice and the outbound sats leg. `AmountIn`
/// passes through.
fn expand_min_amount_out(
    conversion_amount: ConversionAmount,
    source_transfer_fee_sats: u64,
) -> ConversionAmount {
    match conversion_amount {
        ConversionAmount::AmountIn(_) => conversion_amount,
        ConversionAmount::MinAmountOut(amount) => ConversionAmount::MinAmountOut(
            amount.saturating_add(u128::from(source_transfer_fee_sats)),
        ),
    }
}

/// Runs the AMM token conversion for a cross-chain send. `MinAmountOut` is
/// expanded via [`expand_min_amount_out`]; `AmountIn` passes through.
pub(in crate::sdk::payments) async fn convert_token(
    sdk: &BreezSdk,
    conversion_options: &ConversionOptions,
    payment_method: &SendPaymentMethod,
    token_identifier: Option<&String>,
    conversion_amount: ConversionAmount,
) -> Result<(TokenConversionResponse, ConversionPurpose), SdkError> {
    let (recipient_address, source_transfer_fee_sats) = match payment_method {
        SendPaymentMethod::CrossChainAddress {
            recipient_address,
            source_transfer_fee_sats,
            ..
        } => (recipient_address.clone(), *source_transfer_fee_sats),
        _ => {
            return Err(SdkError::Generic(
                "convert_token called with non-cross-chain payment_method".to_string(),
            ));
        }
    };

    let purpose = ConversionPurpose::OngoingPayment {
        payment_request: recipient_address,
    };

    let conversion_amount = expand_min_amount_out(conversion_amount, source_transfer_fee_sats);

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
    use super::*;
    use macros::test_all;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    #[test_all]
    fn expand_min_amount_out_adds_source_transfer_fee_for_boltz() {
        let expanded = expand_min_amount_out(ConversionAmount::MinAmountOut(1_000), 25);
        match expanded {
            ConversionAmount::MinAmountOut(v) => assert_eq!(v, 1_025),
            other @ ConversionAmount::AmountIn(_) => {
                panic!("expected MinAmountOut, got {other:?}")
            }
        }
    }

    #[test_all]
    fn expand_min_amount_out_is_identity_when_source_transfer_fee_zero() {
        let expanded = expand_min_amount_out(ConversionAmount::MinAmountOut(1_000), 0);
        match expanded {
            ConversionAmount::MinAmountOut(v) => assert_eq!(v, 1_000),
            other @ ConversionAmount::AmountIn(_) => {
                panic!("expected MinAmountOut, got {other:?}")
            }
        }
    }

    #[test_all]
    fn expand_min_amount_out_passes_amount_in_through() {
        let expanded = expand_min_amount_out(ConversionAmount::AmountIn(5_000_000), 25);
        match expanded {
            ConversionAmount::AmountIn(v) => assert_eq!(v, 5_000_000),
            other @ ConversionAmount::MinAmountOut(_) => {
                panic!("expected AmountIn, got {other:?}")
            }
        }
    }
}
