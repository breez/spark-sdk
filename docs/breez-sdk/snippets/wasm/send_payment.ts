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
  // Optionally set the amount you wish the pay the receiver
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
  // Set the amount you wish the pay the receiver
  const payAmount: PayAmount = {
    type: 'bitcoin',
    amountSats: 50_000
  }
  // Select the confirmation speed (required for Bitcoin addresses)
  const onchainSpeed = 'medium'

  const prepareResponse = await sdk.prepareSendPayment({
    paymentRequest,
    payAmount,
    onchainSpeed
  })

  // If the fees are acceptable, continue to create the Send Payment
  if (prepareResponse.paymentMethod.type === 'bitcoinAddress') {
    const feeSats = prepareResponse.paymentMethod.feeSats
    const selectedSpeed = prepareResponse.paymentMethod.selectedSpeed
    console.debug(`Fee for ${selectedSpeed} speed: ${feeSats} sats`)
  }
  // ANCHOR_END: prepare-send-payment-onchain
}

const examplePrepareSendPaymentSparkAddress = async (sdk: BreezSdk) => {
  // ANCHOR: prepare-send-payment-spark-address
  const paymentRequest = '<spark address>'
  // Set the amount you wish the pay the receiver
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
  // Optionally set the amount you wish the pay the receiver
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
  const optionalIdempotencyKey = '<idempotency key uuid>'
  const sendResponse = await sdk.sendPayment({
    prepareResponse,
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

const exampleEstimateOnchainSendFeeQuotes = async (sdk: BreezSdk) => {
  // ANCHOR: estimate-onchain-send-fee-quotes
  const address = '<bitcoin address>'
  // Optionally set the amount, omit for drain
  const optionalAmountSats = 50_000

  const response = await sdk.estimateOnchainSendFeeQuotes({
    address,
    amountSats: optionalAmountSats
  })

  const feeQuote = response.feeQuote
  const slowFeeSats = feeQuote.speedSlow.userFeeSat + feeQuote.speedSlow.l1BroadcastFeeSat
  const mediumFeeSats = feeQuote.speedMedium.userFeeSat + feeQuote.speedMedium.l1BroadcastFeeSat
  const fastFeeSats = feeQuote.speedFast.userFeeSat + feeQuote.speedFast.l1BroadcastFeeSat
  console.debug(`Slow Fees: ${slowFeeSats} sats`)
  console.debug(`Medium Fees: ${mediumFeeSats} sats`)
  console.debug(`Fast Fees: ${fastFeeSats} sats`)
  // ANCHOR_END: estimate-onchain-send-fee-quotes
}

const examplePrepareSendPaymentDrainOnchain = async (sdk: BreezSdk) => {
  // ANCHOR: prepare-send-payment-drain-onchain
  const paymentRequest = '<bitcoin address>'
  // Select the confirmation speed (required for Bitcoin addresses)
  const onchainSpeed = 'medium'
  // Use Drain to send all available funds
  const payAmount: PayAmount = { type: 'drain' }

  const prepareResponse = await sdk.prepareSendPayment({
    paymentRequest,
    payAmount,
    onchainSpeed
  })

  // The amount is calculated as balance minus the fee for the selected speed
  if (prepareResponse.paymentMethod.type === 'bitcoinAddress') {
    const drainAmount = prepareResponse.amount
    const feeSats = prepareResponse.paymentMethod.feeSats
    const selectedSpeed = prepareResponse.paymentMethod.selectedSpeed
    console.debug(`Drain amount: ${drainAmount} sats`)
    console.debug(`Fee for ${selectedSpeed} speed: ${feeSats} sats`)
  }
  // ANCHOR_END: prepare-send-payment-drain-onchain
}
