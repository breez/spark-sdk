import { type BreezClient, InputType_Tags } from '@breeztech/breez-sdk-spark-react-native'

const exampleLnurlWithdraw = async (client: BreezClient) => {
  // ANCHOR: lnurl-withdraw
  // Endpoint can also be of the form:
  // lnurlw://domain.com/lnurl-withdraw?key=val
  const lnurlWithdrawUrl =
    'lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4exctthd96xserjv9mn7um9wdekjmmw843xxwpexdnxzen9vgunsvfexq6rvdecx93rgdmyxcuxverrvcursenpxvukzv3c8qunsdecx33nzwpnvg6ryc3hv93nzvecxgcxgwp3h33lxk'

  const input = await client.parse(lnurlWithdrawUrl)
  if (input.tag === InputType_Tags.LnurlWithdraw) {
    // Amount to withdraw in sats between min/max withdrawable amounts
    const amountSats = BigInt(5_000)
    const withdrawRequest = input.inner[0]
    const optionalCompletionTimeoutSecs = 30

    const response = await client.lnurl().withdraw({
      amountSats,
      withdrawRequest,
      completionTimeoutSecs: optionalCompletionTimeoutSecs
    })

    const payment = response.payment
    console.log(`Payment: ${JSON.stringify(payment)}`)
  }
  // ANCHOR_END: lnurl-withdraw
}
