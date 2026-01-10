import {
  defaultExternalSigner,
  connectWithSigner,
  defaultConfig,
  Network,
  KeySetType
} from '@breeztech/breez-sdk-spark-react-native'
import type { KeySetConfig } from '@breeztech/breez-sdk-spark-react-native'
import RNFS from 'react-native-fs'

// ANCHOR: default-external-signer
const createSigner = () => {
  const mnemonic = '<mnemonic words>'
  const keySetConfig: KeySetConfig = {
    keySetType: KeySetType.Default,
    useAddressIndex: false,
    accountNumber: 0
  }

  // Create the default signer from the SDK
  const signer = defaultExternalSigner(mnemonic, undefined, Network.Mainnet, keySetConfig)

  return signer
}
// ANCHOR_END: default-external-signer

// ANCHOR: connect-with-signer
const exampleConnectWithSigner = async (signer: ReturnType<typeof defaultExternalSigner>) => {
  // Create the config
  const config = defaultConfig(Network.Mainnet)
  config.apiKey = '<breez api key>'

  // Connect using the external signer
  const sdk = await connectWithSigner({
    config,
    signer,
    storageDir: `${RNFS.DocumentDirectoryPath}/data`
  })
}
// ANCHOR_END: connect-with-signer

export { createSigner, exampleConnectWithSigner }
