import {
  type BreezClient,
  type PaymentIntent,
  type ConversionOptions,
  type FeePolicy,
  type LnurlPayOptions,
  parseInput
} from '@breeztech/breez-sdk-spark'

const examplePrepareLnurlPay = async (client: BreezClient) => {
  // ANCHOR: prepare-lnurl-pay
  // Endpoint can also be of the form:
  // lnurlp://domain.com/lnurl-pay?key=val
  // lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4excttsv9un7um9wdekjmmw84jxywf5x43rvv35xgmr2enrxanr2cfcvsmnwe3jxcukvde48qukgdec89snwde3vfjxvepjxpjnjvtpxd3kvdnxx5crxwpjvyunsephsz36jf
  const lnurlPayUrl = 'lightning@address.com'

  // Step 1: Parse the LNURL/Lightning Address to discover service metadata.
  // Per LUD-06, the wallet must fetch and display min/max sendable, description, etc.
  const input = await parseInput(lnurlPayUrl)

  // Step 2: Extract pay request details for UI.
  // Show min/max sendable bounds, description, and optional comment field to user.
  const payRequest = input.type === 'lnurlPay' ? input
    : input.type === 'lightningAddress' ? input.payRequest : null
  if (!payRequest) throw new Error('Not an LNURL-Pay destination')
  const minSats = Math.ceil(payRequest.minSendable / 1000)
  const maxSats = Math.floor(payRequest.maxSendable / 1000)
  const commentAllowed = payRequest.commentAllowed ?? 0
  // Display these constraints in your UI...

  // Step 3: After user selects an amount and optional comment, prepare the payment.
  // Passing the parsed `input` avoids a redundant network round-trip to re-fetch
  // the LNURL metadata (the parse step already did that).
  const userChosenAmount = 5_000 // User's selected amount within [minSats, maxSats]
  const optionalComment = '<comment>'
  const optionalValidateSuccessActionUrl = true
  // Optionally set to use token funds to pay via token conversion
  const optionalMaxSlippageBps = 50
  const optionalCompletionTimeoutSecs = 30
  const optionalConversionOptions: ConversionOptions = {
    conversionType: {
      type: 'toBitcoin',
      fromTokenIdentifier: '<token identifier>'
    },
    maxSlippageBps: optionalMaxSlippageBps,
    completionTimeoutSecs: optionalCompletionTimeoutSecs
  }

  const lnurl: LnurlPayOptions = {
    comment: optionalComment,
    validateSuccessActionUrl: optionalValidateSuccessActionUrl
  }

  const payment = await client.preparePayment(input, {
    amountSats: userChosenAmount,
    lnurl,
    conversionOptions: optionalConversionOptions
  })

  // Inspect the payment
  console.log(`Payment type: ${payment.paymentType}`)
  console.log(`Is LNURL: ${payment.isLnurl}`)
  console.log(`Fees: ${payment.feeSats} sats`)
  // ANCHOR_END: prepare-lnurl-pay
}

const examplePrepareLnurlPayFeesIncluded = async (client: BreezClient) => {
  // ANCHOR: prepare-lnurl-pay-fees-included
  // By default, fees are added on top of the amount.
  // Use 'feesIncluded' to deduct fees from the amount instead.
  // The receiver gets amount minus fees.
  const lnurlPayUrl = 'lightning@address.com'

  // Parse first to discover min/max and display to user (required per LUD-06)
  const input = await parseInput(lnurlPayUrl)

  const optionalComment = '<comment>'
  const amountSats = 5_000
  const feePolicy: FeePolicy = 'feesIncluded'

  const payment = await client.preparePayment(input, {
    amountSats,
    lnurl: { comment: optionalComment },
    feePolicy
  })

  // If the fees are acceptable, continue to confirm the payment
  console.log(`Fees: ${payment.feeSats} sats`)
  // The receiver gets amountSats - feeSats
  // ANCHOR_END: prepare-lnurl-pay-fees-included
}

const exampleLnurlPay = async (client: BreezClient, payment: PaymentIntent) => {
  // ANCHOR: lnurl-pay
  const optionalIdempotencyKey = '<idempotency key uuid>'
  const result = await payment.send({
    idempotencyKey: optionalIdempotencyKey
  })
  // ANCHOR_END: lnurl-pay
  console.log(result)
}
