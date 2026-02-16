import { createHash } from 'crypto'
import type { Payment, BreezClient, PaymentIntent } from '@breeztech/breez-sdk-spark'

const exampleSendHtlcPayment = async (client: BreezClient): Promise<Payment> => {
  // ANCHOR: send-htlc-payment
  const destination = '<spark address>'
  // Set the amount you wish to pay the receiver (in sats)
  const amountSats = 50_000

  const payment = await client.preparePayment(destination, {
    amountSats
  })

  // If the fees are acceptable, continue to create the HTLC Payment
  const feeSats = payment.feeSats
  console.debug(`Fees: ${feeSats} sats`)

  const preimage = '<32-byte unique preimage hex>'
  const preimageBuffer = Buffer.from(preimage, 'hex')
  const paymentHash = createHash('sha256').update(preimageBuffer).digest('hex')

  const sendResponse = await payment.send({
    sendOptions: {
      type: 'sparkAddress',
      htlcOptions: {
        paymentHash,
        expiryDurationSecs: 1000
      }
    }
  })
  const confirmedPayment = sendResponse.payment
  // ANCHOR_END: send-htlc-payment
  return confirmedPayment
}

const exampleListClaimableHtlcPayments = async (client: BreezClient): Promise<Payment[]> => {
  // ANCHOR: list-claimable-htlc-payments
  const response = await client.payments.list({
    typeFilter: ['receive'],
    statusFilter: ['pending'],
    paymentDetailsFilter: [{
      type: 'spark',
      htlcStatus: ['waitingForPreimage']
    }],
    assetFilter: undefined
  })
  const payments = response.payments
  // ANCHOR_END: list-claimable-htlc-payments
  return payments
}

const exampleClaimHtlcPayment = async (client: BreezClient): Promise<Payment> => {
  // ANCHOR: claim-htlc-payment
  const preimage = '<preimage hex>'
  const response = await client.payments.claimHtlc({
    preimage
  })
  const payment = response.payment
  // ANCHOR_END: claim-htlc-payment
  return payment
}
