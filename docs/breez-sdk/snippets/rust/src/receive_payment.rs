use anyhow::Result;
use breez_sdk_spark::*;
use log::info;

async fn prepare_receive_lightning(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: prepare-receive-payment-lightning
    let description = "<invoice description>".to_string();
    // Optionally set the invoice amount you wish the payer to send
    let optional_amount_sats = Some(5_000);

    let prepare_response = sdk.prepare_receive_payment(PrepareReceivePaymentRequest {
        payment_method: ReceivePaymentMethod::Bolt11Invoice {
            description,
            amount_sats: optional_amount_sats,
        },
    })?;

    let receive_fee_sats = prepare_response.fee_sats;
    info!("Fees: {receive_fee_sats} sats");
    // ANCHOR_END: prepare-receive-payment-lightning
    Ok(())
}

async fn prepare_receive_onchain(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: prepare-receive-payment-onchain
    let prepare_response = sdk.prepare_receive_payment(PrepareReceivePaymentRequest {
        payment_method: ReceivePaymentMethod::BitcoinAddress,
    })?;

    let receive_fee_sats = prepare_response.fee_sats;
    info!("Fees: {receive_fee_sats} sats");
    // ANCHOR_END: prepare-receive-payment-onchain
    Ok(())
}

async fn prepare_receive_spark(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: prepare-receive-payment-spark
    let prepare_response = sdk.prepare_receive_payment(PrepareReceivePaymentRequest {
        payment_method: ReceivePaymentMethod::SparkAddress,
    })?;

    let receive_fee_sats = prepare_response.fee_sats;
    info!("Fees: {receive_fee_sats} sats");
    // ANCHOR_END: prepare-receive-payment-spark
    Ok(())
}

async fn receive_payment(
    sdk: &BreezSdk,
    prepare_response: PrepareReceivePaymentResponse,
) -> Result<()> {
    // ANCHOR: receive-payment
    let res = sdk
        .receive_payment(ReceivePaymentRequest { prepare_response })
        .await?;

    let payment_request = res.payment_request;
    // ANCHOR_END: receive-payment
    info!("Payment request: {payment_request}");
    Ok(())
}
