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

  // The modern API handles LNURL resolution internally via preparePayment
  const amountSats = 5_000
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

  const payment = await client.preparePayment(lnurlPayUrl, {
    amountSats,
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
  const optionalComment = '<comment>'
  const amountSats = 5_000
  const feePolicy: FeePolicy = 'feesIncluded'

  const payment = await client.preparePayment(lnurlPayUrl, {
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
