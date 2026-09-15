# Listing payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_payments

To view your payment history, you can list all the payments that have been sent and received.

```rust
let response = sdk.list_payments(ListPaymentsRequest::default()).await?;
let payments = response.payments;
```



## Filtering Payments

When listing payments you can also filter and page the results.

```rust
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
        payment_details_filter: None,
    })
    .await?;
let payments = response.payments;
```



## Get Payment

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_payment

You can also retrieve a single payment using the payment id:

```rust
let payment_id = "<payment id>".to_string();
let response = sdk.get_payment(GetPaymentRequest { payment_id }).await?;
let payment = response.payment;
```
