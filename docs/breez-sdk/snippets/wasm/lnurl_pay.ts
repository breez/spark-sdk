import type {
  BreezSdk,
  LnurlPayRequestDetails,
  PrepareLnurlPayResponse,
  ConversionOptions,
  FeePolicy
} from '@breeztech/breez-sdk-spark'

const examplePrepareLnurlPay = async (sdk: BreezSdk) => {
  // ANCHOR: prepare-lnurl-pay
  // Endpoint can also be of the form:
  // lnurlp://domain.com/lnurl-pay?key=val
  // lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4excttsv9un7um9wdekjmmw84jxywf5x43rvv35xgmr2enrxanr2cfcvsmnwe3jxcukvde48qukgdec89snwde3vfjxvepjxpjnjvtpxd3kvdnxx5crxwpjvyunsephsz36jf
  const lnurlPayUrl = 'lightning@address.com'

  const input = await sdk.parse(lnurlPayUrl)
  if (input.type === 'lightningAddress') {
    const amountSats = 5_000
    const optionalComment = '<comment>'
    const payRequest = input.payRequest
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

    const prepareResponse = await sdk.prepareLnurlPay({
      amountSats,
      payRequest,
      comment: optionalComment,
      validateSuccessActionUrl: optionalValidateSuccessActionUrl,
      conversionOptions: optionalConversionOptions,
      feePolicy: undefined
    })

    // If the fees are acceptable, continue to create the LNURL Pay
    if (prepareResponse.conversionEstimate !== undefined) {
      const conversionEstimate = prepareResponse.conversionEstimate
      console.debug(`Estimated conversion amount: ${conversionEstimate.amount} token base units`)
      console.debug(`Estimated conversion fee: ${conversionEstimate.fee} token base units`)
    }

    const feeSats = prepareResponse.feeSats
    console.log(`Fees: ${feeSats} sats`)
  }
  // ANCHOR_END: prepare-lnurl-pay
}

const examplePrepareLnurlPayFeesIncluded = async (sdk: BreezSdk, payRequest: LnurlPayRequestDetails) => {
  // ANCHOR: prepare-lnurl-pay-fees-included
  // By default ({ type: 'feesExcluded' }), fees are added on top of the amount.
  // Use { type: 'feesIncluded' } to deduct fees from the amount instead.
  // The receiver gets amount minus fees.
  const optionalComment = '<comment>'
  const optionalValidateSuccessActionUrl = true
  const amountSats = 5_000
  const feePolicy: FeePolicy = 'feesIncluded'

  const prepareResponse = await sdk.prepareLnurlPay({
    amountSats,
    payRequest,
    comment: optionalComment,
    validateSuccessActionUrl: optionalValidateSuccessActionUrl,
    conversionOptions: undefined,
    feePolicy
  })

  // If the fees are acceptable, continue to create the LNURL Pay
  const feeSats = prepareResponse.feeSats
  console.log(`Fees: ${feeSats} sats`)
  // The receiver gets amountSats - feeSats
  // ANCHOR_END: prepare-lnurl-pay-fees-included
}

const exampleLnurlPay = async (sdk: BreezSdk, prepareResponse: PrepareLnurlPayResponse) => {
  // ANCHOR: lnurl-pay
  const optionalIdempotencyKey = '<idempotency key uuid>'
  const response = await sdk.lnurlPay({
    prepareResponse,
    idempotencyKey: optionalIdempotencyKey
  })
  // ANCHOR_END: lnurl-pay
  console.log(response)
}
