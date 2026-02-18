import { createHash } from 'crypto'
import type { Payment, BreezSdk, PrepareSendPaymentResponse } from '@breeztech/breez-sdk-spark-react-native'
import {
  SendPaymentOptions,
  SparkHtlcOptions,
  SparkHtlcStatus,
  PaymentDetailsFilter,
  PaymentType,
  PaymentStatus,
  SendPaymentMethod_Tags,
  ReceivePaymentMethod,
  PaymentDetails_Tags
} from '@breeztech/breez-sdk-spark-react-native'

const exampleSendHtlcPayment = async (sdk: BreezSdk): Promise<Payment> => {
  // ANCHOR: send-htlc-payment
  const paymentRequest = '<spark address>'
  // Set the amount you wish to pay the receiver
  const amountSats = BigInt(50_000)
  const prepareRequest = {
    paymentRequest,
    amount: amountSats,
    tokenIdentifier: undefined,
    conversionOptions: undefined,
    feePolicy: undefined
  }
  const prepareResponse = await sdk.prepareSendPayment(prepareRequest)

  // If the fees are acceptable, continue to create the HTLC Payment
  if (prepareResponse.paymentMethod?.tag === SendPaymentMethod_Tags.SparkAddress) {
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

const exampleReceiveHodlInvoicePayment = async (sdk: BreezSdk) => {
  // ANCHOR: receive-hodl-invoice-payment
  const preimage = '<32-byte unique preimage hex>'
  const preimageBuffer = Buffer.from(preimage, 'hex')
  const paymentHash = createHash('sha256').update(preimageBuffer).digest('hex')

  const response = await sdk.receivePayment({
    paymentMethod: new ReceivePaymentMethod.Bolt11Invoice({
      description: 'HODL invoice',
      amountSats: BigInt(50_000),
      expirySecs: undefined,
      paymentHash
    })
  })

  const invoice = response.paymentRequest
  console.log(`HODL invoice: ${invoice}`)
  // ANCHOR_END: receive-hodl-invoice-payment
}

const exampleListClaimableHtlcPayments = async (sdk: BreezSdk): Promise<Payment[]> => {
  // ANCHOR: list-claimable-htlc-payments
  const request = {
    typeFilter: [PaymentType.Receive],
    statusFilter: [PaymentStatus.Pending],
    paymentDetailsFilter: [new PaymentDetailsFilter.Spark({
      htlcStatus: [SparkHtlcStatus.WaitingForPreimage],
      conversionRefundNeeded: undefined
    }), new PaymentDetailsFilter.Lightning({
      htlcStatus: [SparkHtlcStatus.WaitingForPreimage]
    })],
    assetFilter: undefined,
    fromTimestamp: undefined,
    toTimestamp: undefined,
    offset: undefined,
    limit: undefined,
    sortAscending: undefined
  }

  const response = await sdk.listPayments(request)
  const payments = response.payments

  for (const payment of payments) {
    if (payment.details?.tag === PaymentDetails_Tags.Spark) {
      const htlc = payment.details.inner.htlcDetails
      if (htlc != null) {
        console.log(`Spark HTLC expiry time: ${htlc.expiryTime}`)
      }
    } else if (payment.details?.tag === PaymentDetails_Tags.Lightning) {
      const htlc = payment.details.inner.htlcDetails
      console.log(`Lightning HTLC expiry time: ${htlc.expiryTime}`)
    }
  }
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
