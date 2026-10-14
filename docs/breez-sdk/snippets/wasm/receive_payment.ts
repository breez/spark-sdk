import { type BreezSdk } from '@breeztech/breez-sdk-spark'

const exampleReceiveLightningPayment = async (sdk: BreezSdk) => {
  // ANCHOR: receive-payment-lightning-bolt11
  const description = '<invoice description>'
  // Optionally set the invoice amount you wish the payer to send
  const optionalAmountSats = 5_000
  // Optionally set the expiry duration in seconds
  const optionalExpirySecs = 3600
  // Set this to create an invoice for another Spark identity
  const optionalReceiverIdentityPublicKey = undefined
  // Set to 'feesIncluded' to deduct a partner fee from the amount
  // instead of adding it on top for the payer
  const optionalFeePolicy = undefined

  const response = await sdk.receivePayment({
    paymentMethod: {
      type: 'bolt11Invoice',
      description,
      amountSats: optionalAmountSats,
      expirySecs: optionalExpirySecs,
      paymentHash: undefined,
      receiverIdentityPublicKey: optionalReceiverIdentityPublicKey,
      feePolicy: optionalFeePolicy
    }
  })

  const paymentRequest = response.paymentRequest
  console.log(`Payment Request: ${paymentRequest}`)
  if (response.fee.type === 'fixed') {
    console.log(`Fees: ${response.fee.amount} sats`)
  } else if (response.fee.type === 'rate') {
    // Only on an amountless invoice: a share of what the payer sends
    const { bps, fixedSats } = response.fee
    console.log(`Fees: ${bps} bps of the amount sent plus ${fixedSats} sats`)
  }
  // ANCHOR_END: receive-payment-lightning-bolt11
}

const exampleReceiveOnchainPayment = async (sdk: BreezSdk) => {
  // ANCHOR: receive-payment-onchain
  const newAddress = undefined // Set to true to get a new address
  const response = await sdk.receivePayment({
    paymentMethod: { type: 'bitcoinAddress', newAddress }
  })

  const paymentRequest = response.paymentRequest
  console.log(`Payment Request: ${paymentRequest}`)
  if (response.fee.type === 'fixed') {
    console.log(`Fees: ${response.fee.amount} sats`)
  }
  // ANCHOR_END: receive-payment-onchain
}

const exampleReceiveSparkAddress = async (sdk: BreezSdk) => {
  // ANCHOR: receive-payment-spark-address
  const response = await sdk.receivePayment({
    paymentMethod: { type: 'sparkAddress' }
  })

  const paymentRequest = response.paymentRequest
  console.log(`Payment Request: ${paymentRequest}`)
  if (response.fee.type === 'fixed') {
    console.log(`Fees: ${response.fee.amount} sats`)
  }
  // ANCHOR_END: receive-payment-spark-address
}

const exampleReceiveSparkInvoice = async (sdk: BreezSdk) => {
  // ANCHOR: receive-payment-spark-invoice
  const optionalDescription = '<invoice description>'
  const optionalAmountSats = BigInt(5_000)
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
  if (response.fee.type === 'fixed') {
    console.log(`Fees: ${response.fee.amount} sats`)
  }
  // ANCHOR_END: receive-payment-spark-invoice
}
