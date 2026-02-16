import { type BreezClient } from '@breeztech/breez-sdk-spark'

const exampleReceiveLightningPayment = async (client: BreezClient) => {
  // ANCHOR: receive-payment-lightning-bolt11
  const description = '<invoice description>'
  // Optionally set the invoice amount you wish the payer to send
  const optionalAmountSats = 5_000
  // Optionally set the expiry duration in seconds
  const optionalExpirySecs = 3600

  const result = await client.receive({
    paymentType: 'lightning',
    amountSats: optionalAmountSats,
    description,
    expiry: optionalExpirySecs
  })

  const destination = result.destination
  console.log(`Payment Request: ${destination}`)
  const receiveFeeSats = result.feeSats
  console.log(`Fees: ${receiveFeeSats} sats`)
  // ANCHOR_END: receive-payment-lightning-bolt11
}

const exampleReceiveOnchainPayment = async (client: BreezClient) => {
  // ANCHOR: receive-payment-onchain
  const result = await client.receive({
    paymentType: 'onchain'
  })

  const destination = result.destination
  console.log(`Payment Request: ${destination}`)
  const receiveFeeSats = result.feeSats
  console.log(`Fees: ${receiveFeeSats} sats`)
  // ANCHOR_END: receive-payment-onchain
}

const exampleReceiveSparkAddress = async (client: BreezClient) => {
  // ANCHOR: receive-payment-spark-address
  const result = await client.receive({
    paymentType: 'sparkAddress'
  })

  const destination = result.destination
  console.log(`Payment Request: ${destination}`)
  const receiveFeeSats = result.feeSats
  console.log(`Fees: ${receiveFeeSats} sats`)
  // ANCHOR_END: receive-payment-spark-address
}

const exampleReceiveSparkInvoice = async (client: BreezClient) => {
  // ANCHOR: receive-payment-spark-invoice
  const optionalDescription = '<invoice description>'
  const optionalAmountSats = 5_000
  // Optionally set the expiry UNIX timestamp in seconds
  const optionalExpiryTimeSeconds = 1716691200
  const optionalSenderPublicKey = '<sender public key>'

  const result = await client.receive({
    paymentType: 'sparkInvoice',
    description: optionalDescription,
    amountSats: optionalAmountSats,
    expiry: optionalExpiryTimeSeconds,
    senderPublicKey: optionalSenderPublicKey
  })

  const destination = result.destination
  console.log(`Payment Request: ${destination}`)
  const receiveFeeSats = result.feeSats
  console.log(`Fees: ${receiveFeeSats} sats`)
  // ANCHOR_END: receive-payment-spark-invoice
}
