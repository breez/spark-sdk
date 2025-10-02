use anyhow::Result;
use breez_sdk_spark::*;
use log::info;

async fn receive_lightning_bolt11(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: receive-payment-lightning-bolt11
    let description = "<invoice description>".to_string();
    // Optionally set the invoice amount you wish the payer to send
    let optional_amount_sats = Some(5_000);

    let response = sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::Bolt11Invoice {
                description,
                amount_sats: optional_amount_sats,
            },
        })
        .await?;

    let payment_request = response.payment_request;
    info!("Payment request: {payment_request}");
    let receive_fee_sats = response.fee_sats;
    info!("Fees: {receive_fee_sats} sats");
    // ANCHOR_END: receive-payment-lightning-bolt11
    Ok(())
}

async fn receive_onchain(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: receive-payment-onchain
    let response = sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::BitcoinAddress,
        })
        .await?;

    let payment_request = response.payment_request;
    info!("Payment request: {payment_request}");
    let receive_fee_sats = response.fee_sats;
    info!("Fees: {receive_fee_sats} sats");
    // ANCHOR_END: receive-payment-onchain
    Ok(())
}

async fn receive_spark(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: receive-payment-spark
    let response = sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?;

    let payment_request = response.payment_request;
    info!("Payment request: {payment_request}");
    let receive_fee_sats = response.fee_sats;
    info!("Fees: {receive_fee_sats} sats");
    // ANCHOR_END: receive-payment-spark
    Ok(())
}

async fn wait_for_payment_example(sdk: &BreezSdk, payment_request: String) -> Result<()> {
    // ANCHOR: wait-for-payment
    // Wait for a payment to be completed using a payment request
    let response = sdk
        .wait_for_payment(WaitForPaymentRequest {
            identifier: WaitForPaymentIdentifier::PaymentRequest(payment_request),
        })
        .await?;

    info!("Payment received with ID: {}", response.payment.id);
    // ANCHOR_END: wait-for-payment
    Ok(())
}
