import type { Payment, BreezSdk } from '@breeztech/breez-sdk-spark-react-native'
import { PaymentType, PaymentStatus, AssetFilter } from '@breeztech/breez-sdk-spark-react-native'

const exampleGetPayment = async (sdk: BreezSdk): Promise<Payment> => {
  // ANCHOR: get-payment
  const paymentId = '<payment id>'
  const response = await sdk.getPayment({
    paymentId
  })
  const payment = response.payment
  // ANCHOR_END: get-payment
  return payment
}

const exampleListPayments = async (sdk: BreezSdk): Promise<Payment[]> => {
  // ANCHOR: list-payments
  const response = await sdk.listPayments({
    typeFilter: undefined,
    statusFilter: undefined,
    assetFilter: undefined,
    fromTimestamp: undefined,
    toTimestamp: undefined,
    offset: undefined,
    limit: undefined,
    sortAscending: undefined,
    sparkHtlcStatusFilter: undefined
  })
  const payments = response.payments
  // ANCHOR_END: list-payments
  return payments
}

const exampleListPaymentsFiltered = async (sdk: BreezSdk): Promise<Payment[]> => {
  // ANCHOR: list-payments-filtered
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
    // Time range filters
    fromTimestamp: 1704067200n, // Unix timestamp
    toTimestamp: 1735689600n, // Unix timestamp
    // Pagination
    offset: 0,
    limit: 50,
    // Sort order (true = oldest first, false = newest first)
    sortAscending: false,
    sparkHtlcStatusFilter: undefined
  })
  const payments = response.payments
  // ANCHOR_END: list-payments-filtered
  return payments
}
