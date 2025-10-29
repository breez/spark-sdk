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

async fn wait_for_payment_example(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: wait-for-payment
    // Waiting for a payment given its payment request (Bolt11 or Spark invoice)
    let payment_request = "<Bolt11 or Spark invoice>".to_string();

    // Wait for a payment to be completed using a payment request
    let payment_request_response = sdk
        .wait_for_payment(WaitForPaymentRequest {
            identifier: WaitForPaymentIdentifier::PaymentRequest(payment_request),
        })
        .await?;

    info!("Payment received with ID: {}", payment_request_response.payment.id);

    // Waiting for a payment given its payment id
    let payment_id = "<payment id>".to_string();

    // Wait for a payment to be completed using a payment id
    let payment_id_response = sdk
        .wait_for_payment(WaitForPaymentRequest {
            identifier: WaitForPaymentIdentifier::PaymentId(payment_id),
        })
        .await?;

    info!("Payment received with ID: {}", payment_id_response.payment.id);
    // ANCHOR_END: wait-for-payment
    Ok(())
}
