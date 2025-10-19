import {
  InputType,
  defaultConfig,
  Network,
  type BreezSdk
} from '@breeztech/breez-sdk-spark-react-native'

const parseInputs = async (sdk: BreezSdk) => {
  // ANCHOR: parse-inputs
  const input = 'an input to be parsed...'

  const parsed = await sdk.parse(input)

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

const exampleSetExternalInputParsers = async () => {
  // ANCHOR: set-external-input-parsers
  // Create the default config
  const config = defaultConfig(Network.Mainnet)
  config.apiKey = '<breez api key>'

  // Configure external parsers
  config.externalInputParsers = [
    {
      providerId: 'provider_a',
      inputRegex: '^provider_a',
      parserUrl: 'https://parser-domain.com/parser?input=<input>'
    },
    {
      providerId: 'provider_b',
      inputRegex: '^provider_b',
      parserUrl: 'https://parser-domain.com/parser?input=<input>'
    }
  ]
  // ANCHOR_END: set-external-input-parsers
}
