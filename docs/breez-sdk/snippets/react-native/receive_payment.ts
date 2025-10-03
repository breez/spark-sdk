import { ReceivePaymentMethod, WaitForPaymentIdentifier, type BreezSdk } from '@breeztech/breez-sdk-spark-react-native'

const exampleReceiveLightningPayment = async (sdk: BreezSdk) => {
  // ANCHOR: receive-payment-lightning-bolt11
  const description = '<invoice description>'
  // Optionally set the invoice amount you wish the payer to send
  const optionalAmountSats = BigInt(5_000)

  const response = await sdk.receivePayment({
    paymentMethod: new ReceivePaymentMethod.Bolt11Invoice({
      description,
      amountSats: optionalAmountSats
    })
  })

  const paymentRequest = response.paymentRequest
  console.log(`Payment Request: ${paymentRequest}`)
  const receiveFeeSats = response.feeSats
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
  const receiveFeeSats = response.feeSats
  console.log(`Fees: ${receiveFeeSats} sats`)
  // ANCHOR_END: receive-payment-onchain
}

const exampleReceiveSparkPayment = async (sdk: BreezSdk) => {
  // ANCHOR: receive-payment-spark
  const response = await sdk.receivePayment({
    paymentMethod: new ReceivePaymentMethod.SparkAddress()
  })

  const paymentRequest = response.paymentRequest
  console.log(`Payment Request: ${paymentRequest}`)
  const receiveFeeSats = response.feeSats
  console.log(`Fees: ${receiveFeeSats} sats`)
  // ANCHOR_END: receive-payment-spark
}

const exampleWaitForPayment = async (sdk: BreezSdk, paymentRequest: string) => {
  // ANCHOR: wait-for-payment
  // Wait for a payment to be completed using a payment request
  const response = await sdk.waitForPayment({
    identifier: new WaitForPaymentIdentifier.PaymentRequest(paymentRequest)
  })

  console.log(`Payment received with ID: ${response.payment.id}`)
  // ANCHOR_END: wait-for-payment
}
