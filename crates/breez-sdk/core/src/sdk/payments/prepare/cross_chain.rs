use breez_sdk_common::input;

use crate::{
    CrossChainFeeMode, CrossChainRoutePair, FeePolicy, SendPaymentMethod,
    error::SdkError,
    models::PrepareSendPaymentResponse,
    sdk::BreezSdk,
};

pub(crate) async fn prepare(
    sdk: &BreezSdk,
    address: &str,
    route: &CrossChainRoutePair,
    amount: u128,
    token_identifier: Option<String>,
    fee_policy: FeePolicy,
    max_slippage_bps: Option<u32>,
) -> Result<PrepareSendPaymentResponse, SdkError> {
    if amount == 0 {
        return Err(SdkError::InvalidInput(
            "Amount must be greater than 0.".to_string(),
        ));
    }

    // Validate address is a recognized cross-chain address.
    if input::detect_address_family(address).is_none() {
        return Err(SdkError::InvalidInput(
            "Address is not a recognized cross-chain address".to_string(),
        ));
    }

    // Resolve slippage: request → config default → built-in default.
    let config_default = sdk
        .config
        .cross_chain_config
        .as_ref()
        .and_then(|c| c.default_slippage_bps);
    if let Some(bps) = max_slippage_bps
        && !(crate::cross_chain::MIN_CROSS_CHAIN_SLIPPAGE_BPS
            ..=crate::cross_chain::MAX_CROSS_CHAIN_SLIPPAGE_BPS)
            .contains(&bps)
    {
        return Err(SdkError::InvalidInput(format!(
            "max_slippage_bps {bps} must be in {}..={}",
            crate::cross_chain::MIN_CROSS_CHAIN_SLIPPAGE_BPS,
            crate::cross_chain::MAX_CROSS_CHAIN_SLIPPAGE_BPS,
        )));
    }
    let resolved_slippage_bps = max_slippage_bps
        .or(config_default)
        .unwrap_or(crate::cross_chain::DEFAULT_CROSS_CHAIN_SLIPPAGE_BPS);

    let service = sdk.cross_chain_providers.get(route.provider)?;

    let fee_mode = match fee_policy {
        FeePolicy::FeesExcluded => CrossChainFeeMode::FeesExcluded,
        FeePolicy::FeesIncluded => CrossChainFeeMode::FeesIncluded,
    };

    let prepared = service
        .prepare(
            address,
            route,
            amount,
            token_identifier.clone(),
            Some(resolved_slippage_bps),
            fee_mode,
        )
        .await?;

    Ok(PrepareSendPaymentResponse {
        payment_method: SendPaymentMethod::CrossChainAddress {
            route: prepared.pair,
            recipient_address: prepared.recipient_address,
            amount_in: prepared.amount_in,
            estimated_out: prepared.estimated_out,
            fee_amount: prepared.fee_amount,
            fee_asset: prepared.fee_asset,
            source_transfer_fee_sats: prepared.source_transfer_fee_sats,
            fee_mode: prepared.fee_mode,
            expires_at: prepared.expires_at,
            provider_context: prepared.provider_context,
        },
        amount,
        token_identifier,
        conversion_estimate: None,
        fee_policy,
    })
}
