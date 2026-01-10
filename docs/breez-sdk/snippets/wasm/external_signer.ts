import {
  defaultExternalSigner,
  connectWithSigner,
  defaultConfig
} from '@breeztech/breez-sdk-spark'
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
  const signer = defaultExternalSigner(mnemonic, null, 'mainnet', keySetConfig)

  return signer
}
// ANCHOR_END: default-external-signer

// ANCHOR: connect-with-signer
const exampleConnectWithSigner = async (signer: ReturnType<typeof defaultExternalSigner>) => {
  // Create the config
  const config = defaultConfig('mainnet')
  config.apiKey = '<breez api key>'

  // Connect using the external signer
  const sdk = await connectWithSigner(
    config,
    signer,
    'breez_spark_db' // For WASM, this is the IndexedDB database name
  )
}
// ANCHOR_END: connect-with-signer

export { createSigner, exampleConnectWithSigner }
