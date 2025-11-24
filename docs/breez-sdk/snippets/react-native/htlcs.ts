import { createHash } from 'crypto'
import type { Payment, BreezSdk, PrepareSendPaymentResponse } from '@breeztech/breez-sdk-spark-react-native'
import {
  SendPaymentOptions,
  SparkHtlcOptions,
  SparkHtlcStatus,
  PaymentType,
  PaymentStatus,
  ClaimHtlcPaymentRequest,
  SendPaymentMethod
} from '@breeztech/breez-sdk-spark-react-native'

const exampleSendHtlcPayment = async (sdk: BreezSdk): Promise<Payment> => {
  // ANCHOR: send-htlc-payment
  const paymentRequest = '<spark address>'
  // Set the amount you wish the pay the receiver
  const amountSats = BigInt(50000)
  const prepareRequest = {
    paymentRequest,
    amount: amountSats,
    tokenIdentifier: undefined
  }
  const prepareResponse = await sdk.prepareSendPayment(prepareRequest)

  // If the fees are acceptable, continue to create the HTLC Payment
  if (prepareResponse.paymentMethod instanceof SendPaymentMethod.SparkAddress) {
    const fee = prepareResponse.paymentMethod.inner.fee
    console.debug(`Fees: ${fee} sats`)
  }

  const preimage = '<32-byte unique preimage hex>'
  const preimageBuffer = Buffer.from(preimage, 'hex')
  const paymentHash = createHash('sha256').update(preimageBuffer).digest('hex')

  // Set the HTLC options
  const options = new SendPaymentOptions.SparkAddress({
    htlcOptions: {
      paymentHash,
      expiryDurationSecs: BigInt(1000)
    }
  })

  const request = {
    prepareResponse,
    options,
    idempotencyKey: undefined
  }
  const sendResponse = await sdk.sendPayment(request)
  const payment = sendResponse.payment
  // ANCHOR_END: send-htlc-payment
  return payment
}

const exampleListClaimableHtlcPayments = async (sdk: BreezSdk): Promise<Payment[]> => {
  // ANCHOR: list-claimable-htlc-payments
  const request = {
    typeFilter: [PaymentType.Receive],
    statusFilter: [PaymentStatus.Pending],
    sparkHtlcStatusFilter: [SparkHtlcStatus.WaitingForPreimage],
    assetFilter: undefined,
    fromTimestamp: undefined,
    toTimestamp: undefined,
    offset: undefined,
    limit: undefined,
    sortAscending: undefined
  }

  const response = await sdk.listPayments(request)
  const payments = response.payments
  // ANCHOR_END: list-claimable-htlc-payments
  return payments
}

const exampleClaimHtlcPayment = async (sdk: BreezSdk): Promise<Payment> => {
  // ANCHOR: claim-htlc-payment
  const preimage = '<preimage hex>'
  const response = await sdk.claimHtlcPayment(
    { preimage }
  )
  const payment = response.payment
  // ANCHOR_END: claim-htlc-payment
  return payment
}
