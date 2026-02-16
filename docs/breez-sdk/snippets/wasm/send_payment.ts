import {
  type BreezClient,
  type PaymentIntent,
  type ConversionOptions,
  type FeePolicy
} from '@breeztech/breez-sdk-spark'

const examplePrepareSendPaymentLightningBolt11 = async (client: BreezClient) => {
  // ANCHOR: prepare-send-payment-lightning-bolt11
  const destination = '<bolt11 invoice>'
  // Optionally set the amount you wish to pay the receiver (in sats)
  const optionalAmountSats = 5_000

  const payment = await client.preparePayment(destination, {
    amountSats: optionalAmountSats
  })

  // Inspect the fee breakdown
  console.debug(`Payment type: ${payment.paymentType}`)
  console.debug(`Amount: ${payment.amountSats} sats`)
  console.debug(`Fee: ${payment.feeSats} sats`)

  // For detailed fee info, use payment.fee (discriminated union)
  const fee = payment.fee
  if (fee.type === 'lightning') {
    console.debug(`Lightning fee: ${fee.feeSats} sats`)
  }
  // ANCHOR_END: prepare-send-payment-lightning-bolt11
}

const examplePrepareSendPaymentOnchain = async (client: BreezClient) => {
  // ANCHOR: prepare-send-payment-onchain
  const destination = '<bitcoin address>'
  // Set the amount you wish to pay the receiver (in sats)
  const amountSats = 50_000

  const payment = await client.preparePayment(destination, {
    amountSats
  })

  // Review the fee quote for each confirmation speed
  const fee = payment.fee
  if (fee.type === 'onchain') {
    const slowFeeSats = fee.speedSlow.userFeeSat + fee.speedSlow.l1BroadcastFeeSat
    const mediumFeeSats = fee.speedMedium.userFeeSat + fee.speedMedium.l1BroadcastFeeSat
    const fastFeeSats = fee.speedFast.userFeeSat + fee.speedFast.l1BroadcastFeeSat
    console.debug(`Slow fee: ${slowFeeSats} sats`)
    console.debug(`Medium fee: ${mediumFeeSats} sats`)
    console.debug(`Fast fee: ${fastFeeSats} sats`)
  }
  // ANCHOR_END: prepare-send-payment-onchain
}

const examplePrepareSendPaymentSparkAddress = async (client: BreezClient) => {
  // ANCHOR: prepare-send-payment-spark-address
  const destination = '<spark address>'
  // Set the amount you wish to pay the receiver (in sats)
  const amountSats = 50_000

  const payment = await client.preparePayment(destination, {
    amountSats
  })

  // If the fees are acceptable, continue to confirm the payment
  console.debug(`Fees: ${payment.feeSats} sats`)
  // ANCHOR_END: prepare-send-payment-spark-address
}

const examplePrepareSendPaymentSparkInvoice = async (client: BreezClient) => {
  // ANCHOR: prepare-send-payment-spark-invoice
  const destination = '<spark invoice>'
  // Optionally set the amount you wish to pay the receiver (in sats)
  const optionalAmountSats = 50_000

  const payment = await client.preparePayment(destination, {
    amountSats: optionalAmountSats
  })

  // If the fees are acceptable, continue to confirm the payment
  console.debug(`Fees: ${payment.feeSats} sats`)
  // ANCHOR_END: prepare-send-payment-spark-invoice
}

const examplePrepareSendPaymentTokenConversion = async (client: BreezClient) => {
  // ANCHOR: prepare-send-payment-with-conversion
  const destination = '<payment request>'
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

  const payment = await client.preparePayment(destination, {
    conversionOptions
  })

  // Inspect the fee and conversion estimate
  console.debug(`Fee: ${payment.feeSats} sats`)
  // ANCHOR_END: prepare-send-payment-with-conversion
}

const exampleSendPaymentLightningBolt11 = async (client: BreezClient, payment: PaymentIntent) => {
  // ANCHOR: send-payment-lightning-bolt11
  const optionalIdempotencyKey = '<idempotency key uuid>'
  const result = await payment.send({
    idempotencyKey: optionalIdempotencyKey,
    sendOptions: {
      type: 'bolt11Invoice',
      preferSpark: false,
      completionTimeoutSecs: 10
    }
  })
  const confirmedPayment = result.payment
  // ANCHOR_END: send-payment-lightning-bolt11
  console.log(confirmedPayment)
}

const exampleSendPaymentOnchain = async (client: BreezClient, payment: PaymentIntent) => {
  // ANCHOR: send-payment-onchain
  // Select the confirmation speed for the on-chain transaction
  const optionalIdempotencyKey = '<idempotency key uuid>'
  const result = await payment.send({
    idempotencyKey: optionalIdempotencyKey,
    sendOptions: {
      type: 'bitcoinAddress',
      confirmationSpeed: 'medium'
    }
  })
  const confirmedPayment = result.payment
  // ANCHOR_END: send-payment-onchain
  console.log(confirmedPayment)
}

const exampleSendPaymentSpark = async (client: BreezClient, payment: PaymentIntent) => {
  // ANCHOR: send-payment-spark
  const optionalIdempotencyKey = '<idempotency key uuid>'
  const result = await payment.send({
    idempotencyKey: optionalIdempotencyKey
  })
  const confirmedPayment = result.payment
  // ANCHOR_END: send-payment-spark
  console.log(confirmedPayment)
}

const examplePrepareSendPaymentFeesIncluded = async (client: BreezClient) => {
  // ANCHOR: prepare-send-payment-fees-included
  // By default, fees are added on top of the amount.
  // Use 'feesIncluded' to deduct fees from the amount instead.
  // The receiver gets amount minus fees.
  const destination = '<payment request>'
  const amountSats = 50_000
  const feePolicy: FeePolicy = 'feesIncluded'

  const payment = await client.preparePayment(destination, {
    amountSats,
    feePolicy
  })

  // Inspect the payment
  console.log(`Amount: ${payment.amountSats} sats`)
  console.log(`Fee: ${payment.feeSats} sats`)
  // The receiver gets amount - fees
  // ANCHOR_END: prepare-send-payment-fees-included
}
