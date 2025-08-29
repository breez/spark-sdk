import { Payment, type BreezSdk } from '@breeztech/breez-sdk-spark'

const exampleGetPayment = async (sdk: BreezSdk): Payment => {
  // ANCHOR: get-payment
  const paymentId = '<payment id>'
  const response = await sdk.getPayment({
    paymentId
  })
  const payment = response.payment
  // ANCHOR_END: get-payment
  return payment
}

const exampleListPayments = async (sdk: BreezSdk): Payment[] => {
  // ANCHOR: list-payments
  const response = await sdk.listPayments({})
  const payments = response.payments
  // ANCHOR_END: list-payments
  return payments
}

const exampleListPaymentsFiltered = async (sdk: BreezSdk): Payment[] => {
  // ANCHOR: list-payments-filtered
  const response = await sdk.listPayments({
    offset: 0,
    limit: 50
  })
  const payments = response.payments
  // ANCHOR_END: list-payments-filtered
  return payments
}
