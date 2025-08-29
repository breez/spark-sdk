use anyhow::Result;
use breez_sdk_spark::*;

async fn get_payment(sdk: &BreezSdk) -> Result<Payment> {
    // ANCHOR: get-payment
    let payment_id = "<payment id>".to_string();
    let response = sdk.get_payment(GetPaymentRequest { payment_id }).await?;
    let payment = response.payment;
    // ANCHOR_END: get-payment

    Ok(payment)
}

async fn list_payments(sdk: &BreezSdk) -> Result<Vec<Payment>> {
    // ANCHOR: list-payments
    let response = sdk
        .list_payments(ListPaymentsRequest {
            offset: None,
            limit: None,
        })
        .await?;
    let payments = response.payments;
    // ANCHOR_END: list-payments

    Ok(payments)
}

async fn list_payments_filtered(sdk: &BreezSdk) -> Result<Vec<Payment>> {
    // ANCHOR: list-payments-filtered
    let response = sdk
        .list_payments(ListPaymentsRequest {
            offset: Some(0),
            limit: Some(50),
        })
        .await?;
    let payments = response.payments;
    // ANCHOR_END: list-payments-filtered

    Ok(payments)
}
