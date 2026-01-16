import {
  type BreezSdk,
  type PrepareSendPaymentResponse,
  type SendPaymentOptions,
  type ConversionOptions,
  type PayAmount
} from '@breeztech/breez-sdk-spark'

const examplePrepareSendPaymentLightningBolt11 = async (sdk: BreezSdk) => {
  // ANCHOR: prepare-send-payment-lightning-bolt11
  const paymentRequest = '<bolt11 invoice>'
  // Optionally set the amount you wish to pay the receiver
  const optionalPayAmount: PayAmount = {
    type: 'bitcoin',
    amountSats: 5_000
  }

  const prepareResponse = await sdk.prepareSendPayment({
    paymentRequest,
    payAmount: optionalPayAmount
  })

  // If the fees are acceptable, continue to create the Send Payment
  if (prepareResponse.paymentMethod.type === 'bolt11Invoice') {
    // Fees to pay via Lightning
    const lightningFeeSats = prepareResponse.paymentMethod.lightningFeeSats
    // Or fees to pay (if available) via a Spark transfer
    const sparkTransferFeeSats = prepareResponse.paymentMethod.sparkTransferFeeSats
    console.debug(`Lightning Fees: ${lightningFeeSats} sats`)
    console.debug(`Spark Transfer Fees: ${sparkTransferFeeSats} sats`)
  }
  // ANCHOR_END: prepare-send-payment-lightning-bolt11
}

const examplePrepareSendPaymentOnchain = async (sdk: BreezSdk) => {
  // ANCHOR: prepare-send-payment-onchain
  const paymentRequest = '<bitcoin address>'
  // Set the amount you wish to pay the receiver
  const payAmount: PayAmount = {
    type: 'bitcoin',
    amountSats: 50_000
  }

  const prepareResponse = await sdk.prepareSendPayment({
    paymentRequest,
    payAmount
  })

  // Review the fee quote for each confirmation speed
  if (prepareResponse.paymentMethod.type === 'bitcoinAddress') {
    const feeQuote = prepareResponse.paymentMethod.feeQuote
    const slowFeeSats = feeQuote.speedSlow.userFeeSat + feeQuote.speedSlow.l1BroadcastFeeSat
    const mediumFeeSats = feeQuote.speedMedium.userFeeSat + feeQuote.speedMedium.l1BroadcastFeeSat
    const fastFeeSats = feeQuote.speedFast.userFeeSat + feeQuote.speedFast.l1BroadcastFeeSat
    console.debug(`Slow fee: ${slowFeeSats} sats`)
    console.debug(`Medium fee: ${mediumFeeSats} sats`)
    console.debug(`Fast fee: ${fastFeeSats} sats`)
  }
  // ANCHOR_END: prepare-send-payment-onchain
}

const examplePrepareSendPaymentSparkAddress = async (sdk: BreezSdk) => {
  // ANCHOR: prepare-send-payment-spark-address
  const paymentRequest = '<spark address>'
  // Set the amount you wish to pay the receiver
  const payAmount: PayAmount = {
    type: 'bitcoin',
    amountSats: 50_000
  }

  const prepareResponse = await sdk.prepareSendPayment({
    paymentRequest,
    payAmount
  })

  // If the fees are acceptable, continue to create the Send Payment
  if (prepareResponse.paymentMethod.type === 'sparkAddress') {
    const feeSats = prepareResponse.paymentMethod.fee
    console.debug(`Fees: ${feeSats} sats`)
  }
  // ANCHOR_END: prepare-send-payment-spark-address
}

const examplePrepareSendPaymentSparkInvoice = async (sdk: BreezSdk) => {
  // ANCHOR: prepare-send-payment-spark-invoice
  const paymentRequest = '<spark invoice>'
  // Optionally set the amount you wish to pay the receiver
  const optionalPayAmount: PayAmount = {
    type: 'bitcoin',
    amountSats: 50_000
  }

  const prepareResponse = await sdk.prepareSendPayment({
    paymentRequest,
    payAmount: optionalPayAmount
  })

  // If the fees are acceptable, continue to create the Send Payment
  if (prepareResponse.paymentMethod.type === 'sparkInvoice') {
    const feeSats = prepareResponse.paymentMethod.fee
    console.debug(`Fees: ${feeSats} sats`)
  }
  // ANCHOR_END: prepare-send-payment-spark-invoice
}

const examplePrepareSendPaymentTokenConversion = async (sdk: BreezSdk) => {
  // ANCHOR: prepare-send-payment-with-conversion
  const paymentRequest = '<bolt11 invoice>'
  // Set to use token funds to pay via conversion
  const optionalMaxSlippageBps = 50
  const optionalCompletionTimeoutSecs = 30
  const conversionOptions: ConversionOptions = {
    conversionType: {
      type: 'toBitcoin',
      fromTokenIdentifier: '<token identifier>'
    },
    maxSlippageBps: optionalMaxSlippageBps,
    completionTimeoutSecs: optionalCompletionTimeoutSecs
  }

  const prepareResponse = await sdk.prepareSendPayment({
    paymentRequest,
    conversionOptions
  })

  // If the fees are acceptable, continue to create the Send Payment
  if (prepareResponse.conversionEstimate !== undefined) {
    const conversionEstimate = prepareResponse.conversionEstimate
    console.debug(`Estimated conversion amount: ${conversionEstimate.amount} token base units`)
    console.debug(`Estimated conversion fee: ${conversionEstimate.fee} token base units`)
  }
  // ANCHOR_END: prepare-send-payment-with-conversion
}

const exampleSendPaymentLightningBolt11 = async (
  sdk: BreezSdk,
  prepareResponse: PrepareSendPaymentResponse
) => {
  // ANCHOR: send-payment-lightning-bolt11
  const options: SendPaymentOptions = {
    type: 'bolt11Invoice',
    preferSpark: false,
    completionTimeoutSecs: 10
  }
  const optionalIdempotencyKey = '<idempotency key uuid>'
  const sendResponse = await sdk.sendPayment({
    prepareResponse,
    options,
    idempotencyKey: optionalIdempotencyKey
  })
  const payment = sendResponse.payment
  // ANCHOR_END: send-payment-lightning-bolt11
  console.log(payment)
}

const exampleSendPaymentOnchain = async (
  sdk: BreezSdk,
  prepareResponse: PrepareSendPaymentResponse
) => {
  // ANCHOR: send-payment-onchain
  // Select the confirmation speed for the on-chain transaction
  const options: SendPaymentOptions = {
    type: 'bitcoinAddress',
    confirmationSpeed: 'medium'
  }
  const optionalIdempotencyKey = '<idempotency key uuid>'
  const sendResponse = await sdk.sendPayment({
    prepareResponse,
    options,
    idempotencyKey: optionalIdempotencyKey
  })
  const payment = sendResponse.payment
  // ANCHOR_END: send-payment-onchain
  console.log(payment)
}

const exampleSendPaymentSpark = async (
  sdk: BreezSdk,
  prepareResponse: PrepareSendPaymentResponse
) => {
  // ANCHOR: send-payment-spark
  const optionalIdempotencyKey = '<idempotency key uuid>'
  const sendResponse = await sdk.sendPayment({
    prepareResponse,
    idempotencyKey: optionalIdempotencyKey
  })
  const payment = sendResponse.payment
  // ANCHOR_END: send-payment-spark
  console.log(payment)
}

const examplePrepareSendPaymentDrain = async (sdk: BreezSdk) => {
  // ANCHOR: prepare-send-payment-drain
  // Use PayAmount Drain to send all available funds
  const paymentRequest = '<payment request>'
  const payAmount: PayAmount = { type: 'drain' }

  const prepareResponse = await sdk.prepareSendPayment({
    paymentRequest,
    payAmount
  })

  // The response contains PayAmount Drain to indicate this is a drain operation
  console.log(`Pay amount: ${JSON.stringify(prepareResponse.payAmount)}`)
  // ANCHOR_END: prepare-send-payment-drain
}
