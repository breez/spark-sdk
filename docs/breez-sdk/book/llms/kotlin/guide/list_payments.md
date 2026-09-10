# Listing payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_payments

To view your payment history, you can list all the payments that have been sent and received.

```kotlin
try {
    val response = sdk.listPayments(ListPaymentsRequest())
    val payments = response.payments
} catch (e: Exception) {
    // handle error
}
```



## Filtering Payments

When listing payments you can also filter and page the results.

```kotlin
try {
    // Filter by asset (Bitcoin or Token)
    val assetFilter = AssetFilter.Token(tokenIdentifier = "token_identifier_here")
    // To filter by Bitcoin instead:
    // val assetFilter = AssetFilter.Bitcoin

    val response = sdk.listPayments(
        ListPaymentsRequest(
            // Filter by payment type
            typeFilter = listOf(PaymentType.SEND, PaymentType.RECEIVE),
            // Filter by status
            statusFilter = listOf(PaymentStatus.COMPLETED),
            assetFilter = assetFilter,
            // Time range filters
            fromTimestamp = 1704067200u, // Unix timestamp
            toTimestamp = 1735689600u,   // Unix timestamp
            // Pagination
            offset = 0u,
            limit = 50u,
            // Sort order (true = oldest first, false = newest first)
            sortAscending = false
        ))
    val payments = response.payments
} catch (e: Exception) {
    // handle error
}
```



## Get Payment

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_payment

You can also retrieve a single payment using the payment id:

```kotlin
try {
    val paymentId = "<payment id>";
    val response = sdk.getPayment(GetPaymentRequest(paymentId))
    val payment = response.payment
} catch (e: Exception) {
    // handle error
}
```
