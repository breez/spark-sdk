import { type BreezSdk, parse, type PrepareLnurlPayResponse } from '@breeztech/breez-sdk-spark'

const examplePrepareLnurlPay = async (sdk: BreezSdk) => {
  // ANCHOR: prepare-lnurl-pay
  // Endpoint can also be of the
  // lnurlp://domain.com/lnurl-pay?key=val
  // lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4excttsv9un7um9wdekjmmw84jxywf5x43rvv35xgmr2enrxanr2cfcvsmnwe3jxcukvde48qukgdec89snwde3vfjxvepjxpjnjvtpxd3kvdnxx5crxwpjvyunsephsz36jf
  const lnurlPayUrl = 'lightning@address.com'

  const input = await parse(lnurlPayUrl)
  if (input.type === 'lightningAddress') {
    const amountSats = 5_000
    const optionalComment = '<comment>'
    const payRequest = input.payRequest
    const optionalValidateSuccessActionUrl = true

    const prepareResponse = await sdk.prepareLnurlPay({
      amountSats,
      payRequest,
      comment: optionalComment,
      validateSuccessActionUrl: optionalValidateSuccessActionUrl
    })

    // If the fees are acceptable, continue to create the LNURL Pay
    const feeSats = prepareResponse.feeSats
    console.log(`Fees: ${feeSats} sats`)
  }
  // ANCHOR_END: prepare-lnurl-pay
}

const exampleLnurlPay = async (sdk: BreezSdk, prepareResponse: PrepareLnurlPayResponse) => {
  // ANCHOR: lnurl-pay
  const response = await sdk.lnurlPay({
    prepareResponse
  })
  // ANCHOR_END: lnurl-pay
  console.log(response)
}
