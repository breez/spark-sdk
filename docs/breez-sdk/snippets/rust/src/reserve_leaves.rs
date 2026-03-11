use anyhow::Result;
use breez_sdk_spark::*;
use log::info;

async fn prepare_send_payment_reserve_leaves(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: prepare-send-payment-reserve-leaves
    let payment_request = "<payment request>".to_string();
    let amount_sats = Some(50_000);

    let prepare_response = sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request,
            amount: amount_sats,
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
            reserve_leaves: Some(true),
        })
        .await?;

    // The reservation ID can be used to cancel the reservation if needed
    if let Some(reservation_id) = &prepare_response.reservation_id {
        info!("Reservation ID: {reservation_id}");
    }

    // Send payment as usual using the prepare response
    // sdk.send_payment(SendPaymentRequest { prepare_response, options: None, idempotency_key: None }).await?;
    // ANCHOR_END: prepare-send-payment-reserve-leaves
    Ok(())
}

async fn cancel_prepare_send_payment(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: cancel-prepare-send-payment
    let reservation_id = "<reservation id from prepare response>".to_string();

    sdk.cancel_prepare_send_payment(CancelPrepareSendPaymentRequest { reservation_id })
        .await?;
    // ANCHOR_END: cancel-prepare-send-payment
    Ok(())
}
