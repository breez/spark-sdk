import { createHash } from 'crypto'
import type { Payment, Wallet, PaymentIntent } from '@breeztech/breez-sdk-spark'

const exampleSendHtlcPayment = async (wallet: Wallet): Promise<Payment> => {
  // ANCHOR: send-htlc-payment
  const destination = '<spark address>'
  // Set the amount you wish to pay the receiver
  const amountSats = BigInt(50_000)

  const intent = await wallet.createPayment(destination, {
    amount: amountSats
  })

  // If the fees are acceptable, continue to create the HTLC Payment
  const feeSats = intent.feeSats
  console.debug(`Fees: ${feeSats} sats`)

  const preimage = '<32-byte unique preimage hex>'
  const preimageBuffer = Buffer.from(preimage, 'hex')
  const paymentHash = createHash('sha256').update(preimageBuffer).digest('hex')

  const sendResponse = await intent.confirm({
    sendOptions: {
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

const exampleListClaimableHtlcPayments = async (wallet: Wallet): Promise<Payment[]> => {
  // ANCHOR: list-claimable-htlc-payments
  const response = await wallet.listPayments({
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

const exampleClaimHtlcPayment = async (wallet: Wallet): Promise<Payment> => {
  // ANCHOR: claim-htlc-payment
  const preimage = '<preimage hex>'
  const response = await wallet.claimHtlcPayment({
    preimage
  })
  const payment = response.payment
  // ANCHOR_END: claim-htlc-payment
  return payment
}
