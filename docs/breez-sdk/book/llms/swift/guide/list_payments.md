# Listing payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_payments

To view your payment history, you can list all the payments that have been sent and received.

```swift
let response = try await sdk.listPayments(
    request: ListPaymentsRequest())
let payments = response.payments
```



## Filtering Payments

When listing payments you can also filter and page the results.

```swift
// Filter by asset (Bitcoin or Token)
let assetFilter = AssetFilter.token(tokenIdentifier: "token_identifier_here")
// To filter by Bitcoin instead:
// let assetFilter = AssetFilter.bitcoin

let response = try await sdk.listPayments(
    request: ListPaymentsRequest(
        // Filter by payment type
        typeFilter: [PaymentType.send, PaymentType.receive],
        // Filter by status
        statusFilter: [PaymentStatus.completed],
        assetFilter: assetFilter,
        // Time range filters
        fromTimestamp: 1_704_067_200,  // Unix timestamp
        toTimestamp: 1_735_689_600,  // Unix timestamp
        // Pagination
        offset: 0,
        limit: 50,
        // Sort order (true = oldest first, false = newest first)
        sortAscending: false
    ))
let payments = response.payments
```



## Get Payment

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_payment

You can also retrieve a single payment using the payment id:

```swift
let paymentId = "<payment id>"
let response = try await sdk.getPayment(
    request: GetPaymentRequest(paymentId: paymentId)
)
let payment = response.payment
```
