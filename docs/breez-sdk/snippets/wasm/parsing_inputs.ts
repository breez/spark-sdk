import { defaultConfig, Seed, type BreezSdk } from '@breeztech/breez-sdk-spark'

const parseInputs = async (sdk: BreezSdk) => {
  // ANCHOR: parse-inputs
  const input = 'an input to be parsed...'

  const parsed = await sdk.parse(input)

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

    case 'sparkAddress':
      console.log(`Input is Spark address ${parsed.address}`)
      break

    case 'sparkInvoice':
      console.log('Input is Spark invoice:')
      if (parsed.tokenIdentifier != null) {
        console.log(
          `  Amount: ${parsed.amount} base units of token with id ${parsed.tokenIdentifier}`
        )
      } else {
        console.log(`  Amount: ${parsed.amount} sats`)
      }

      if (parsed.description != null) {
        console.log(`  Description: ${parsed.description}`)
      }

      if (parsed.expiryTime != null) {
        console.log(`  Expiry time: ${new Date(Number(parsed.expiryTime) * 1000).toISOString()}`)
      }

      if (parsed.senderPublicKey != null) {
        console.log(`  Sender public key: ${parsed.senderPublicKey}`)
      }
      break

    default:
      // Other input types are available
      break
  }
  // ANCHOR_END: parse-inputs
}

const exampleSetExternalInputParsers = async () => {
  // ANCHOR: set-external-input-parsers
  // Create the default config
  const config = defaultConfig('mainnet')
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
