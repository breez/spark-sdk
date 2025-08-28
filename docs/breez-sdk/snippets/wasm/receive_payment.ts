import { type PrepareReceivePaymentResponse, type BreezSdk } from '@breeztech/breez-sdk-spark'

const examplePrepareLightningPayment = (sdk: BreezSdk) => {
  // ANCHOR: prepare-receive-payment-lightning
  const description = '<invoice description>'
  // Optionally set the invoice amount you wish the payer to send
  const optionalAmountSats = 5_000

  const prepareResponse = sdk.prepareReceivePayment({
    paymentMethod: {
      type: 'bolt11Invoice',
      description,
      amountSats: optionalAmountSats
    }
  })

  const receiveFeeSats = prepareResponse.feeSats
  console.log(`Fees: ${receiveFeeSats} sats`)
  // ANCHOR_END: prepare-receive-payment-lightning
}

const examplePrepareOnchainPayment = (sdk: BreezSdk) => {
  // ANCHOR: prepare-receive-payment-onchain
  const prepareResponse = sdk.prepareReceivePayment({
    paymentMethod: { type: 'bitcoinAddress' }
  })

  const receiveFeeSats = prepareResponse.feeSats
  console.log(`Fees: ${receiveFeeSats} sats`)
  // ANCHOR_END: prepare-receive-payment-onchain
}

const examplePrepareSparkPayment = (sdk: BreezSdk) => {
  // ANCHOR: prepare-receive-payment-spark
  const prepareResponse = sdk.prepareReceivePayment({
    paymentMethod: { type: 'sparkAddress' }
  })

  const receiveFeeSats = prepareResponse.feeSats
  console.log(`Fees: ${receiveFeeSats} sats`)
  // ANCHOR_END: prepare-receive-payment-spark
}

const exampleReceivePayment = async (
  sdk: BreezSdk,
  prepareResponse: PrepareReceivePaymentResponse
) => {
  // ANCHOR: receive-payment
  const res = await sdk.receivePayment({
    prepareResponse
  })

  const paymentRequest = res.paymentRequest
  // ANCHOR_END: receive-payment
  console.log(paymentRequest)
}
