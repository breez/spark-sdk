import type { Payment, BreezClient, AssetFilter } from '@breeztech/breez-sdk-spark'

const exampleGetPayment = async (client: BreezClient): Promise<Payment> => {
  // ANCHOR: get-payment
  const paymentId = '<payment id>'
  const response = await client.payments.get(paymentId)
  const payment = response.payment
  // ANCHOR_END: get-payment
  return payment
}

const exampleListPayments = async (client: BreezClient): Promise<Payment[]> => {
  // ANCHOR: list-payments
  const payments = await client.payments.list()
  // ANCHOR_END: list-payments
  return payments
}

const exampleListPaymentsFiltered = async (client: BreezClient): Promise<Payment[]> => {
  // ANCHOR: list-payments-filtered
  // For filtered listing, use the full listPayments method which supports
  // type, status, asset, time range filters and sorting.

  // Filter by asset (Bitcoin or Token)
  const assetFilter: AssetFilter = { type: 'token', tokenIdentifier: 'token_identifier_here' }
  // To filter by Bitcoin instead:
  // const assetFilter: AssetFilter = { type: 'bitcoin' }

  const response = await client.listPayments({
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
  // ANCHOR_END: list-payments-filtered
  return payments
}
