# Listing payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_payments

To view your payment history, you can list all the payments that have been sent and received.

```csharp
var response = await sdk.ListPayments(request: new ListPaymentsRequest());
var payments = response.payments;
```



## Filtering Payments

When listing payments you can also filter and page the results.

```csharp
// Filter by asset (Bitcoin or Token)
var assetFilter = new AssetFilter.Token(tokenIdentifier: "token_identifier_here");
// To filter by Bitcoin instead:
// var assetFilter = new AssetFilter.Bitcoin();

var request = new ListPaymentsRequest(
    // Filter by payment type
    typeFilter: new PaymentType[] { PaymentType.Send, PaymentType.Receive },
    // Filter by status
    statusFilter: new PaymentStatus[] { PaymentStatus.Completed },
    assetFilter: assetFilter,
    // Time range filters
    fromTimestamp: 1704067200, // Unix timestamp
    toTimestamp: 1735689600,   // Unix timestamp
                               // Pagination
    offset: 0,
    limit: 50,
    // Sort order (true = oldest first, false = newest first)
    sortAscending: false
);

var response = await sdk.ListPayments(request: request);
var payments = response.payments;
```



## Get Payment

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_payment

You can also retrieve a single payment using the payment id:

```csharp
var paymentId = "<payment id>";
var response = await sdk.GetPayment(
    request: new GetPaymentRequest(paymentId: paymentId)
);
var payment = response.payment;
```
