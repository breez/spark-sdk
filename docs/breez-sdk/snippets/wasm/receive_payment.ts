import { type BreezSdk } from '@breeztech/breez-sdk-spark'

const exampleReceiveLightningPayment = async (sdk: BreezSdk) => {
  // ANCHOR: receive-payment-lightning-bolt11
  const description = '<invoice description>'
  // Optionally set the invoice amount you wish the payer to send
  const optionalAmountSats = 5_000
  // Optionally set the expiry duration in seconds
  const optionalExpirySecs = 3600

  const response = await sdk.receivePayment({
    paymentMethod: {
      type: 'bolt11Invoice',
      description,
      amountSats: optionalAmountSats,
      expirySecs: optionalExpirySecs,
      paymentHash: undefined
    }
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
    paymentMethod: { type: 'bitcoinAddress' }
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
    paymentMethod: { type: 'sparkAddress' }
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
  const optionalAmountSats = '5000'
  // Optionally set the expiry UNIX timestamp in seconds
  const optionalExpiryTimeSeconds = 1716691200
  const optionalSenderPublicKey = '<sender public key>'

  const response = await sdk.receivePayment({
    paymentMethod: {
      type: 'sparkInvoice',
      description: optionalDescription,
      amount: optionalAmountSats,
      expiryTime: optionalExpiryTimeSeconds,
      senderPublicKey: optionalSenderPublicKey
    }
  })

  const paymentRequest = response.paymentRequest
  console.log(`Payment Request: ${paymentRequest}`)
  const receiveFeeSats = response.fee
  console.log(`Fees: ${receiveFeeSats} sats`)
  // ANCHOR_END: receive-payment-spark-invoice
}
