import { parse } from '@breeztech/breez-sdk-spark'

const parseInputs = async () => {
  // ANCHOR: parse-inputs
  const input = 'an input to be parsed...'

  const parsed = await parse(input)

  switch (parsed.type) {
    case 'bitcoinAddress':
      console.log(`Input is Bitcoin address ${parsed.address}`)
      break

    case 'bolt11Invoice':
      console.log(
        `Input is BOLT11 invoice for ${
          parsed.amountMsat != null ? parsed.amountMsat.toString() : 'unknown'
        } msats`
      )
      break

    case 'lnurlPay':
      console.log(
        `Input is LNURL-Pay/Lightning address accepting min/max ${parsed.minSendable}/${parsed.maxSendable} msats`
      )
      break

    case 'lnurlWithdraw':
      console.log(
        `Input is LNURL-Withdraw for min/max ${parsed.minWithdrawable}/${parsed.maxWithdrawable} msats`
      )
      break

    default:
      // Other input types are available
      break
  }
  // ANCHOR_END: parse-inputs
}
