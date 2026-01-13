import {
  ReceivePaymentMethod,
  type BreezSdk
} from '@breeztech/breez-sdk-spark-react-native'

const exampleReceiveLightningPayment = async (sdk: BreezSdk) => {
  // ANCHOR: receive-payment-lightning-bolt11
  const description = '<invoice description>'
  // Optionally set the invoice amount you wish the payer to send
  const optionalAmountSats = BigInt(5_000)
  // Optionally set the expiry duration in seconds
  const optionalExpiryDurationSecs = 3600

  const response = await sdk.receivePayment({
    paymentMethod: new ReceivePaymentMethod.Bolt11Invoice({
      description,
      amountSats: optionalAmountSats,
      expiryDurationSecs: optionalExpiryDurationSecs
    })
  })

  const paymentRequest = response.paymentRequest
  console.log(`Payment Request: ${paymentRequest}`)
  const receiveFeeSats = response.fee
  console.log(`Fees: ${receiveFeeSats} sats`)
  // ANCHOR_END: receive-payment-lightning-bolt11
}

const exampleReceiveOnchainPayment = async (sdk: BreezSdk) => {
  // ANCHOR: receive-payment-onchain
  const response = await sdk.receivePayment({
    paymentMethod: new ReceivePaymentMethod.BitcoinAddress()
  })

  const paymentRequest = response.paymentRequest
  console.log(`Payment Request: ${paymentRequest}`)
  const receiveFeeSats = response.fee
  console.log(`Fees: ${receiveFeeSats} sats`)
  // ANCHOR_END: receive-payment-onchain
}

const exampleReceiveSparkAddress = async (sdk: BreezSdk) => {
  // ANCHOR: receive-payment-spark-address
  const response = await sdk.receivePayment({
    paymentMethod: new ReceivePaymentMethod.SparkAddress()
  })

  const paymentRequest = response.paymentRequest
  console.log(`Payment Request: ${paymentRequest}`)
  const receiveFeeSats = response.fee
  console.log(`Fees: ${receiveFeeSats} sats`)
  // ANCHOR_END: receive-payment-spark-address
}

const exampleReceiveSparkInvoice = async (sdk: BreezSdk) => {
  // ANCHOR: receive-payment-spark-invoice
  const optionalDescription = '<invoice description>'
  const optionalAmountSats = BigInt(5_000)
  const optionalExpiresAt = BigInt(1716691200)
  const optionalSenderPublicKey = '<sender public key>'

  const response = await sdk.receivePayment({
    paymentMethod: new ReceivePaymentMethod.SparkInvoice({
      description: optionalDescription,
      amount: optionalAmountSats,
      expiresAt: optionalExpiresAt,
      senderPublicKey: optionalSenderPublicKey,
      tokenIdentifier: undefined
    })
  })

  const paymentRequest = response.paymentRequest
  console.log(`Payment Request: ${paymentRequest}`)
  const receiveFeeSats = response.fee
  console.log(`Fees: ${receiveFeeSats} sats`)
  // ANCHOR_END: receive-payment-spark-invoice
}
