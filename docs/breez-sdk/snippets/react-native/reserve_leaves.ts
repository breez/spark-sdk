import { type BreezSdk } from '@breeztech/breez-sdk-spark-react-native'

const examplePrepareSendPaymentReserveLeaves = async (sdk: BreezSdk) => {
  // ANCHOR: prepare-send-payment-reserve-leaves
  const paymentRequest = '<payment request>'
  const amountSats = BigInt(50_000)

  const prepareResponse = await sdk.prepareSendPayment({
    paymentRequest,
    amount: amountSats,
    tokenIdentifier: undefined,
    conversionOptions: undefined,
    feePolicy: undefined,
    reserveLeaves: true
  })

  // The reservation ID can be used to cancel the reservation if needed
  if (prepareResponse.reservationId !== undefined) {
    console.log(`Reservation ID: ${prepareResponse.reservationId}`)
  }

  // Send payment as usual using the prepare response
  // await sdk.sendPayment({ prepareResponse, options: undefined, idempotencyKey: undefined })
  // ANCHOR_END: prepare-send-payment-reserve-leaves
}

const exampleCancelPrepareSendPayment = async (sdk: BreezSdk) => {
  // ANCHOR: cancel-prepare-send-payment
  const reservationId = '<reservation id from prepare response>'

  await sdk.cancelPrepareSendPayment({ reservationId })
  // ANCHOR_END: cancel-prepare-send-payment
}
