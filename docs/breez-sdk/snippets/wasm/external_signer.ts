import { BreezSdkSpark } from '@breeztech/breez-sdk-spark'
import type { KeySetConfig } from '@breeztech/breez-sdk-spark'

// ANCHOR: default-external-signer
const createSigner = () => {
  const mnemonic = '<mnemonic words>'
  const keySetConfig: KeySetConfig = {
    keySetType: 'default',
    useAddressIndex: false,
    accountNumber: 0
  }

  // Create the default signer from the SDK
  const signer = BreezSdkSpark.defaultExternalSigner(mnemonic, null, 'mainnet', keySetConfig)

  return signer
}
// ANCHOR_END: default-external-signer

// ANCHOR: connect-with-signer
const exampleConnectWithSigner = async (signer: ReturnType<typeof BreezSdkSpark.defaultExternalSigner>) => {
  // Create the config
  const config = BreezSdkSpark.defaultConfig('mainnet')
  config.apiKey = '<breez api key>'

  // Connect using the external signer
  const sdk = await BreezSdkSpark.connectWithSigner(
    config,
    signer,
    'breez_spark_db' // For WASM, this is the IndexedDB database name
  )
}
// ANCHOR_END: connect-with-signer

export { createSigner, exampleConnectWithSigner }
