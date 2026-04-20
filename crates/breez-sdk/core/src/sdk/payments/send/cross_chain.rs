use crate::{
    CrossChainProviderContext, CrossChainRoutePair, SendPaymentMethod, SendPaymentResponse,
    cross_chain::CrossChainPrepared,
    error::SdkError,
    sdk::{BreezSdk, SyncType},
};

#[allow(clippy::too_many_arguments)]
pub(in crate::sdk) async fn send(
    sdk: &BreezSdk,
    route: &CrossChainRoutePair,
    recipient_address: &str,
    amount_in: u128,
    estimated_out: u128,
    fee_amount: u128,
    fee_asset: Option<String>,
    expires_at: &str,
    provider_context: &CrossChainProviderContext,
    token_identifier: Option<String>,
) -> Result<SendPaymentResponse, SdkError> {
    let service = sdk.cross_chain_providers.get(route.provider)?;

    let prepared = CrossChainPrepared {
        amount_in,
        estimated_out,
        fee_amount,
        fee_asset: fee_asset.clone(),
        expires_at: expires_at.to_string(),
        pair: route.clone(),
        recipient_address: recipient_address.to_string(),
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
    let payment = service.send(&prepared).await?;

    Ok(SendPaymentResponse { payment })
}

/// Cross-chain conversion is not supported on the cross-chain framework
/// commit — later commits add the AMM conversion leg.
pub(in crate::sdk::payments) async fn convert_token_unsupported() -> Result<(), SdkError> {
    Err(SdkError::InvalidInput(
        "Cross-chain sends do not support AMM conversions".to_string(),
    ))
}

// Stub re-export to keep `payments::conversion::execute_pre_send_conversion`'s
// CrossChain arm out of the way at this commit; later commits add a real
// `convert_token` matching the bolt11/spark patterns.
#[allow(dead_code)]
pub(in crate::sdk::payments) fn _payment_method_marker(_: &SendPaymentMethod) {}
