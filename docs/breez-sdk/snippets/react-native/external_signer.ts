import {
  defaultExternalSigner,
  connectWithSigner,
  defaultConfig,
  Network,
  KeySetType
} from '@breeztech/react-native-breez-sdk-spark'

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
  const signer = defaultExternalSigner(
    "<mnemonic words>",
    null, // passphrase
    Network.Mainnet,
    {
      keySetType: KeySetType.Default,
      useAddressIndex: false,
      accountNumber: 0,
    },
  );

  // Create the config
  const config = defaultConfig(Network.MAINNET)
  config.apiKey = '<breez api key>'

  // Connect using the external signer
  const sdk = await connectWithSigner({
    config,
    signer,
    storageDir: './.data'
  })

  return sdk
}
// ANCHOR_END: connect-with-signer

export { createSigner, connectWithSigner }
