use breez_sdk_common::input;

use crate::{
    CrossChainRoutePair, FeePolicy, SendPaymentMethod,
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

    let service = sdk.cross_chain_providers.get(route.provider)?;

    let prepared = service
        .prepare(address, route, amount, token_identifier.clone(), None)
        .await?;

    Ok(PrepareSendPaymentResponse {
        payment_method: SendPaymentMethod::CrossChainAddress {
            route: prepared.pair,
            recipient_address: prepared.recipient_address,
            amount_in: prepared.amount_in,
            estimated_out: prepared.estimated_out,
            fee_amount: prepared.fee_amount,
            fee_asset: prepared.fee_asset,
            expires_at: prepared.expires_at,
            provider_context: prepared.provider_context,
        },
        amount,
        token_identifier,
        conversion_estimate: None,
        fee_policy,
    })
}
