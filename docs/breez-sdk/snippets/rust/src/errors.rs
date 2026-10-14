use anyhow::Result;
use breez_sdk_spark::*;
use log::info;

pub(crate) async fn handle_errors(
    sdk: &BreezSdk,
    request: PrepareSendPaymentRequest,
) -> Result<()> {
    // ANCHOR: handle-errors
    match sdk.prepare_send_payment(request).await {
        Ok(prepare_response) => {
            info!("Payment prepared: {:?}", prepare_response.payment_method);
        }
        Err(SdkError::InsufficientFunds { .. }) => {
            info!("Not enough funds for this payment");
        }
        Err(SdkError::CrossChainDisabled { docs_url }) => {
            info!("Cross-chain payments are not enabled, see {docs_url}");
        }
        Err(error) => {
            info!("Failed to prepare the payment: {error}");
        }
    }
    // ANCHOR_END: handle-errors
    Ok(())
}
