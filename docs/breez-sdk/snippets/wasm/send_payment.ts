import {
  type Wallet,
  type PaymentIntent,
  type ConversionOptions,
  type FeePolicy
} from '@breeztech/breez-sdk-spark'

const examplePrepareSendPaymentLightningBolt11 = async (wallet: Wallet) => {
  // ANCHOR: prepare-send-payment-lightning-bolt11
  const destination = '<bolt11 invoice>'
  // Optionally set the amount you wish to pay the receiver
  const optionalAmountSats = BigInt(5_000)

  const intent = await wallet.createPayment(destination, {
    amount: optionalAmountSats
  })

  // Inspect the fee breakdown
  console.debug(`Payment type: ${intent.paymentType}`)
  console.debug(`Amount: ${intent.amount}`)
  console.debug(`Fee: ${intent.feeSats} sats`)

  // For detailed fee info, use intent.fee (discriminated union)
  const fee = intent.fee
  if (fee.type === 'lightning') {
    console.debug(`Lightning fee: ${fee.feeSats} sats`)
  }
  // ANCHOR_END: prepare-send-payment-lightning-bolt11
}

const examplePrepareSendPaymentOnchain = async (wallet: Wallet) => {
  // ANCHOR: prepare-send-payment-onchain
  const destination = '<bitcoin address>'
  // Set the amount you wish to pay the receiver
  const amountSats = BigInt(50_000)

  const intent = await wallet.createPayment(destination, {
    amount: amountSats
  })

  // Review the fee quote for each confirmation speed
  const fee = intent.fee
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

const examplePrepareSendPaymentSparkAddress = async (wallet: Wallet) => {
  // ANCHOR: prepare-send-payment-spark-address
  const destination = '<spark address>'
  // Set the amount you wish to pay the receiver
  const amountSats = BigInt(50_000)

  const intent = await wallet.createPayment(destination, {
    amount: amountSats
  })

  // If the fees are acceptable, continue to confirm the payment
  console.debug(`Fees: ${intent.feeSats} sats`)
  // ANCHOR_END: prepare-send-payment-spark-address
}

const examplePrepareSendPaymentSparkInvoice = async (wallet: Wallet) => {
  // ANCHOR: prepare-send-payment-spark-invoice
  const destination = '<spark invoice>'
  // Optionally set the amount you wish to pay the receiver
  const optionalAmountSats = BigInt(50_000)

  const intent = await wallet.createPayment(destination, {
    amount: optionalAmountSats
  })

  // If the fees are acceptable, continue to confirm the payment
  console.debug(`Fees: ${intent.feeSats} sats`)
  // ANCHOR_END: prepare-send-payment-spark-invoice
}

const examplePrepareSendPaymentTokenConversion = async (wallet: Wallet) => {
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

  const intent = await wallet.createPayment(destination, {
    conversionOptions
  })

  // Inspect the fee and conversion estimate
  console.debug(`Fee: ${intent.feeSats} sats`)
  // ANCHOR_END: prepare-send-payment-with-conversion
}

const exampleSendPaymentLightningBolt11 = async (wallet: Wallet, intent: PaymentIntent) => {
  // ANCHOR: send-payment-lightning-bolt11
  const optionalIdempotencyKey = '<idempotency key uuid>'
  const result = await intent.confirm({
    idempotencyKey: optionalIdempotencyKey,
    sendOptions: {
      type: 'bolt11Invoice',
      preferSpark: false,
      completionTimeoutSecs: 10
    }
  })
  const payment = result.payment
  // ANCHOR_END: send-payment-lightning-bolt11
  console.log(payment)
}

const exampleSendPaymentOnchain = async (wallet: Wallet, intent: PaymentIntent) => {
  // ANCHOR: send-payment-onchain
  // Select the confirmation speed for the on-chain transaction
  const optionalIdempotencyKey = '<idempotency key uuid>'
  const result = await intent.confirm({
    idempotencyKey: optionalIdempotencyKey,
    sendOptions: {
      type: 'bitcoinAddress',
      confirmationSpeed: 'medium'
    }
  })
  const payment = result.payment
  // ANCHOR_END: send-payment-onchain
  console.log(payment)
}

const exampleSendPaymentSpark = async (wallet: Wallet, intent: PaymentIntent) => {
  // ANCHOR: send-payment-spark
  const optionalIdempotencyKey = '<idempotency key uuid>'
  const result = await intent.confirm({
    idempotencyKey: optionalIdempotencyKey
  })
  const payment = result.payment
  // ANCHOR_END: send-payment-spark
  console.log(payment)
}

const examplePrepareSendPaymentFeesIncluded = async (wallet: Wallet) => {
  // ANCHOR: prepare-send-payment-fees-included
  // By default, fees are added on top of the amount.
  // Use 'feesIncluded' to deduct fees from the amount instead.
  // The receiver gets amount minus fees.
  const destination = '<payment request>'
  const amountSats = BigInt(50_000)
  const feePolicy: FeePolicy = 'feesIncluded'

  const intent = await wallet.createPayment(destination, {
    amount: amountSats,
    feePolicy
  })

  // Inspect the payment intent
  console.log(`Amount: ${intent.amount}`)
  console.log(`Fee: ${intent.feeSats} sats`)
  // The receiver gets amount - fees
  // ANCHOR_END: prepare-send-payment-fees-included
}
