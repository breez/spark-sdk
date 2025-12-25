import {
  defaultExternalSigner,
  connectWithSigner,
  defaultConfig,
  Network,
  KeySetType
} from '@breeztech/breez-sdk-spark-wasm'

// ANCHOR: default-external-signer
async function createSigner() {
  const mnemonic = '<mnemonic words>'
  const network = Network.MAINNET
  const keySetType = KeySetType.DEFAULT
  const useAddressIndex = false
  const accountNumber = 0

  const signer = await defaultExternalSigner({
    mnemonic,
    passphrase: null,
    network,
    keySetType,
    useAddressIndex,
    accountNumber
  })

  return signer
}
// ANCHOR_END: default-external-signer

// ANCHOR: connect-with-signer
async function connectWithSigner() {
  // Create the signer
  const signer = await defaultExternalSigner({
    mnemonic: '<mnemonic words>',
    passphrase: null,
    network: Network.MAINNET,
    keySetType: KeySetType.DEFAULT,
    useAddressIndex: false,
    accountNumber: 0
  })

  // Create the config
  const config = defaultConfig(Network.MAINNET)
  config.apiKey = '<breez api key>'

  // Connect using the external signer
  const sdk = await connectWithSigner({
    config,
    signer,
    storageDir: 'breez_spark_db' // For WASM, this is the IndexedDB database name
  })

  return sdk
}
// ANCHOR_END: connect-with-signer

export { createSigner, connectWithSigner }
