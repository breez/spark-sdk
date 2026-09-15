# Listing payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_payments

To view your payment history, you can list all the payments that have been sent and received.

```dart
ListPaymentsRequest request = ListPaymentsRequest();
ListPaymentsResponse response = await sdk.listPayments(request: request);
List<Payment> payments = response.payments;
```



## Filtering Payments

When listing payments you can also filter and page the results.

```dart
// Filter by asset (Bitcoin or Token)
AssetFilter assetFilter = AssetFilter.token(tokenIdentifier: "token_identifier_here");
// To filter by Bitcoin instead:
// AssetFilter assetFilter = AssetFilter.bitcoin();

ListPaymentsRequest request = ListPaymentsRequest(
  // Filter by payment type
  typeFilter: [PaymentType.send, PaymentType.receive],
  // Filter by status
  statusFilter: [PaymentStatus.completed],
  assetFilter: assetFilter,
  // Time range filters
  fromTimestamp: BigInt.from(1704067200), // Unix timestamp
  toTimestamp: BigInt.from(1735689600),   // Unix timestamp
  // Pagination
  offset: 0,
  limit: 50,
  // Sort order (true = oldest first, false = newest first)
  sortAscending: false,
);
ListPaymentsResponse response = await sdk.listPayments(request: request);
List<Payment> payments = response.payments;
```



## Get Payment

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_payment

You can also retrieve a single payment using the payment id:

```dart
String paymentId = "<payment id>";
GetPaymentRequest request = GetPaymentRequest(paymentId: paymentId);
GetPaymentResponse response = await sdk.getPayment(request: request);
Payment payment = response.payment;
```
