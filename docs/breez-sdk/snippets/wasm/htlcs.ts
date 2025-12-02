import { createHash } from 'crypto'
import type { Payment, BreezSdk, PrepareSendPaymentResponse } from '@breeztech/breez-sdk-spark'

const exampleSendHtlcPayment = async (sdk: BreezSdk): Promise<Payment> => {
  // ANCHOR: send-htlc-payment
  const paymentRequest = '<spark address>'
  // Set the amount you wish the pay the receiver
  const amountSats = BigInt(50000)
  const prepareRequest = {
    paymentRequest,
    amount: amountSats
  }
  const prepareResponse = await sdk.prepareSendPayment(prepareRequest)

  // If the fees are acceptable, continue to create the HTLC Payment
  if (prepareResponse.paymentMethod.type === 'sparkAddress') {
    const fee = prepareResponse.paymentMethod.fee
    console.debug(`Fees: ${fee} sats`)
  }

  const preimage = '<32-byte unique preimage hex>'
  const preimageBuffer = Buffer.from(preimage, 'hex')
  const paymentHash = createHash('sha256').update(preimageBuffer).digest('hex')

  const sendResponse = await sdk.sendPayment({
    prepareResponse,
    options: {
      type: 'sparkAddress',
      htlcOptions: {
        paymentHash,
        expiryDurationSecs: 1000
      }
    }
  })
  const payment = sendResponse.payment
  // ANCHOR_END: send-htlc-payment
  return payment
}

const exampleListClaimableHtlcPayments = async (sdk: BreezSdk): Promise<Payment[]> => {
  // ANCHOR: list-claimable-htlc-payments
  const response = await sdk.listPayments({
    typeFilter: ['receive'],
    statusFilter: ['pending'],
    paymentDetailsFilter: {
      type: 'spark',
      htlcStatus: ['waitingForPreimage']
    },
    assetFilter: undefined
  })
  const payments = response.payments
  // ANCHOR_END: list-claimable-htlc-payments
  return payments
}

const exampleClaimHtlcPayment = async (sdk: BreezSdk): Promise<Payment> => {
  // ANCHOR: claim-htlc-payment
  const preimage = '<preimage hex>'
  const response = await sdk.claimHtlcPayment({
    preimage
  })
  const payment = response.payment
  // ANCHOR_END: claim-htlc-payment
  return payment
}
