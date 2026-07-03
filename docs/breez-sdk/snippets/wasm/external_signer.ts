import {
  defaultExternalSigners,
  connectWithSigner,
  defaultConfig
} from '@breeztech/breez-sdk-spark'

// ANCHOR: default-external-signer
const createSigners = () => {
  const mnemonic = '<mnemonic words>'
  const accountNumber = 0

  // Create the default signers from the SDK
  const signers = defaultExternalSigners(mnemonic, null, 'mainnet', accountNumber)

  return signers
}
// ANCHOR_END: default-external-signer

// ANCHOR: connect-with-signer
const exampleConnectWithSigner = async (
  signers: ReturnType<typeof defaultExternalSigners>
) => {
  // Create the config
  const config = defaultConfig('mainnet')
  config.apiKey = '<breez api key>'

  // Connect using the external signers
  const sdk = await connectWithSigner(
    config,
    signers.breezSigner,
    signers.sparkSigner,
    true, // supportsEciesHmac: the default external signer can perform it
    'breez_spark_db' // For WASM, this is the IndexedDB database name
  )
}
// ANCHOR_END: connect-with-signer

export { createSigners, exampleConnectWithSigner }
