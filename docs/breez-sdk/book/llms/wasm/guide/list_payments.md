# Listing payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_payments

To view your payment history, you can list all the payments that have been sent and received.

```typescript
const response = await sdk.listPayments({})
const payments = response.payments
```



## Filtering Payments

When listing payments you can also filter and page the results.

```typescript
// Filter by asset (Bitcoin or Token)
const assetFilter: AssetFilter = { type: 'token', tokenIdentifier: 'token_identifier_here' }
// To filter by Bitcoin instead:
// const assetFilter: AssetFilter = { type: 'bitcoin' }

const response = await sdk.listPayments({
  // Filter by payment type
  typeFilter: ['send', 'receive'],
  // Filter by status
  statusFilter: ['completed'],
  assetFilter,
  // Time range filters
  fromTimestamp: 1704067200, // Unix timestamp
  toTimestamp: 1735689600, // Unix timestamp
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
