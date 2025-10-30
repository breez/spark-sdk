import { type BreezSdk } from '@breeztech/breez-sdk-spark'

const exampleLnurlWithdraw = async (sdk: BreezSdk) => {
  // ANCHOR: lnurl-withdraw
  // Endpoint can also be of the form:
  // lnurlw://domain.com/lnurl-withdraw?key=val
  const lnurlWithdrawUrl =
    'lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4exctthd96xserjv9mn7um9wdekjmmw843xxwpexdnxzen9vgunsvfexq6rvdecx93rgdmyxcuxverrvcursenpxvukzv3c8qunsdecx33nzwpnvg6ryc3hv93nzvecxgcxgwp3h33lxk'

  const input = await sdk.parse(lnurlWithdrawUrl)
  if (input.type === 'lnurlWithdraw') {
    // Amount to withdraw in sats between min/max withdrawable amounts
    const amountSats = 5_000
    const withdrawRequest = input
    const optionalCompletionTimeoutSecs = 30

    const response = await sdk.lnurlWithdraw({
      amountSats,
      withdrawRequest,
      completionTimeoutSecs: optionalCompletionTimeoutSecs
    })

    const payment = response.payment
    console.log(`Payment: ${JSON.stringify(payment)}`)
  }
  // ANCHOR_END: lnurl-withdraw
}
