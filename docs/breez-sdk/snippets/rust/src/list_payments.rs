use anyhow::Result;
use breez_sdk_spark::*;

#[allow(dead_code)]
async fn get_payment(sdk: &BreezSdk) -> Result<Payment> {
    // ANCHOR: get-payment
    let payment_id = "<payment id>".to_string();
    let response = sdk.get_payment(GetPaymentRequest { payment_id }).await?;
    let payment = response.payment;
    // ANCHOR_END: get-payment

    Ok(payment)
}

#[allow(dead_code)]
async fn list_payments(sdk: &BreezSdk) -> Result<Vec<Payment>> {
    // ANCHOR: list-payments
    let response = sdk.list_payments(ListPaymentsRequest::default()).await?;
    let payments = response.payments;
    // ANCHOR_END: list-payments

    Ok(payments)
}

#[allow(dead_code)]
async fn list_payments_filtered(sdk: &BreezSdk) -> Result<Vec<Payment>> {
    // ANCHOR: list-payments-filtered
    // Filter by asset (Bitcoin or Token)
    let asset_filter = AssetFilter::Token {
        token_identifier: Some("token_identifier_here".to_string()),
    };
    // To filter by Bitcoin instead:
    // let asset_filter = AssetFilter::Bitcoin;

    let response = sdk
        .list_payments(ListPaymentsRequest {
            // Filter by payment type
            type_filter: Some(vec![PaymentType::Send, PaymentType::Receive]),
            // Filter by status
            status_filter: Some(vec![PaymentStatus::Completed]),
            asset_filter: Some(asset_filter),
            // Time range filters
            from_timestamp: Some(1704067200), // Unix timestamp
            to_timestamp: Some(1735689600),   // Unix timestamp
            // Pagination
            offset: Some(0),
            limit: Some(50),
            // Sort order (true = oldest first, false = newest first)
            sort_ascending: Some(false),
            spark_htlc_status_filter: None,
        })
        .await?;
    let payments = response.payments;
    // ANCHOR_END: list-payments-filtered

    Ok(payments)
}
