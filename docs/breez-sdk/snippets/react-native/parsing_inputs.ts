import {
  defaultConfig,
  Network,
  type BreezSdk,
  InputType_Tags
} from '@breeztech/breez-sdk-spark-react-native'

const parseInputs = async (sdk: BreezSdk) => {
  // ANCHOR: parse-inputs
  const inputStr = 'an input to be parsed...'

  const input = await sdk.parse(inputStr)

  if (input.tag === InputType_Tags.BitcoinAddress) {
    console.log(`Input is Bitcoin address ${input.inner[0].address}`)
  } else if (input.tag === InputType_Tags.Bolt11Invoice) {
    console.log(
      `Input is BOLT11 invoice for ${
        input.inner[0].amountMsat != null ? input.inner[0].amountMsat.toString() : 'unknown'
      } msats`
    )
  } else if (input.tag === InputType_Tags.LnurlPay) {
    console.log(
      'Input is LNURL-Pay/Lightning address accepting min/max ' +
        `${input.inner[0].minSendable}/${input.inner[0].maxSendable} msats`
    )
  } else if (input.tag === InputType_Tags.LnurlWithdraw) {
    console.log(
      'Input is LNURL-Withdraw for min/max ' +
        `${input.inner[0].minWithdrawable}/${input.inner[0].maxWithdrawable} msats`
    )
  } else if (input.tag === InputType_Tags.SparkAddress) {
    console.log(`Input is Spark address ${input.inner[0].address}`)
  } else if (input.tag === InputType_Tags.SparkInvoice) {
    const invoice = input.inner[0]
    console.log('Input is Spark invoice:')
    if (invoice.tokenIdentifier != null) {
      console.log(
        `  Amount: ${invoice.amount} base units of token with id ${invoice.tokenIdentifier}`
      )
    } else {
      console.log(`  Amount: ${invoice.amount} sats`)
    }

    if (invoice.description != null) {
      console.log(`  Description: ${invoice.description}`)
    }

    if (invoice.expiryTime != null) {
      console.log(`  Expiry time: ${new Date(Number(invoice.expiryTime) * 1000).toISOString()}`)
    }

    if (invoice.senderPublicKey != null) {
      console.log(`  Sender public key: ${invoice.senderPublicKey}`)
    }
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
