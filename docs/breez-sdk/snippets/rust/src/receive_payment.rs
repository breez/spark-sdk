use anyhow::Result;
use breez_sdk_spark::*;
use log::info;

async fn receive_lightning_bolt11(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: receive-payment-lightning-bolt11
    let description = "<invoice description>".to_string();
    // Optionally set the invoice amount you wish the payer to send
    let optional_amount_sats = Some(5_000);
    // Optionally set the expiry duration in seconds
    let optional_expiry_secs = Some(3600_u32);

    let response = sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::Bolt11Invoice {
                description,
                amount_sats: optional_amount_sats,
                expiry_secs: optional_expiry_secs,
                payment_hash: None,
            },
        })
        .await?;

    let payment_request = response.payment_request;
    info!("Payment request: {payment_request}");
    let receive_fee_sats = response.fee;
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
    let receive_fee_sats = response.fee;
    info!("Fees: {receive_fee_sats} sats");
    // ANCHOR_END: receive-payment-onchain
    Ok(())
}

async fn receive_spark_address(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: receive-payment-spark-address
    let response = sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?;

    let payment_request = response.payment_request;
    info!("Payment request: {payment_request}");
    let receive_fee_sats = response.fee;
    info!("Fees: {receive_fee_sats} sats");
    // ANCHOR_END: receive-payment-spark-address
    Ok(())
}

async fn receive_spark_invoice(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: receive-payment-spark-invoice
    let optional_description = "<invoice description>".to_string();
    let optional_amount_sats = Some(5_000);
    // Optionally set the expiry UNIX timestamp in seconds
    let optional_expiry_time_seconds = Some(1716691200);
    let optional_sender_public_key = Some("<sender public key>".to_string());

    let response = sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkInvoice {
                token_identifier: None,
                description: Some(optional_description),
                amount: optional_amount_sats,
                expiry_time: optional_expiry_time_seconds,
                sender_public_key: optional_sender_public_key,
            },
        })
        .await?;

    let payment_request = response.payment_request;
    info!("Payment request: {payment_request}");
    let receive_fee_sats = response.fee;
    info!("Fees: {receive_fee_sats} sats");
    // ANCHOR_END: receive-payment-spark-invoice
    Ok(())
}
