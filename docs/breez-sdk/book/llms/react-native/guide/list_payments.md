# Listing payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_payments

To view your payment history, you can list all the payments that have been sent and received.

```typescript
const response = await sdk.listPayments({
  typeFilter: undefined,
  statusFilter: undefined,
  assetFilter: undefined,
  paymentDetailsFilter: undefined,
  fromTimestamp: undefined,
  toTimestamp: undefined,
  offset: undefined,
  limit: undefined,
  sortAscending: undefined
})
const payments = response.payments
```



## Filtering Payments

When listing payments you can also filter and page the results.

```typescript
// Filter by asset (Bitcoin or Token)
const assetFilter = new AssetFilter.Token({ tokenIdentifier: 'token_identifier_here' })
// To filter by Bitcoin instead:
// const assetFilter = new AssetFilter.Bitcoin()

const response = await sdk.listPayments({
  // Filter by payment type
  typeFilter: [PaymentType.Send, PaymentType.Receive],
  // Filter by status
  statusFilter: [PaymentStatus.Completed],
  assetFilter,
  paymentDetailsFilter: undefined,
  // Time range filters
  fromTimestamp: 1704067200n, // Unix timestamp
  toTimestamp: 1735689600n, // Unix timestamp
  // Pagination
  offset: 0,
  limit: 50,
  // Sort order (true = oldest first, false = newest first)
  sortAscending: false
})
const payments = response.payments
```



## Get Payment

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_payment

You can also retrieve a single payment using the payment id:

```typescript
const paymentId = '<payment id>'
const response = await sdk.getPayment({
  paymentId
})
const payment = response.payment
```
