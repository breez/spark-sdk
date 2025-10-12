import { InputType, parse } from '@breeztech/breez-sdk-spark-react-native'

const parseInputs = async () => {
  // ANCHOR: parse-inputs
  const input = 'an input to be parsed...'

  const parsed = await parse(input)

  if (parsed instanceof InputType.BitcoinAddress) {
    console.log(`Input is Bitcoin address ${parsed.inner[0].address}`)
  } else if (parsed instanceof InputType.Bolt11Invoice) {
    console.log(
      `Input is BOLT11 invoice for ${
        parsed.inner[0].amountMsat != null ? parsed.inner[0].amountMsat.toString() : 'unknown'
      } msats`
    )
  } else if (parsed instanceof InputType.LnurlPay) {
    console.log(
      'Input is LNURL-Pay/Lightning address accepting min/max ' +
        `${parsed.inner[0].minSendable}/${parsed.inner[0].maxSendable} msats`
    )
  } else if (parsed instanceof InputType.LnurlWithdraw) {
    console.log(
      'Input is LNURL-Withdraw for min/max ' +
        `${parsed.inner[0].minWithdrawable}/${parsed.inner[0].maxWithdrawable} msats`
    )
  } else {
    // Other input types are available
  }
  // ANCHOR_END: parse-inputs
}
