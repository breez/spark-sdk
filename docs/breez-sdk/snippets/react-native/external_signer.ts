import {
  defaultExternalSigners,
  connectWithSigner,
  defaultConfig,
  Network
} from '@breeztech/breez-sdk-spark-react-native'
import type { KeySetConfig } from '@breeztech/breez-sdk-spark-react-native'
import RNFS from 'react-native-fs'

// ANCHOR: default-external-signer
const createSigners = () => {
  const mnemonic = '<mnemonic words>'
  const keySetConfig: KeySetConfig = {
    accountNumber: 0
  }

  // Create the default signers from the SDK
  const signers = defaultExternalSigners(mnemonic, undefined, Network.Mainnet, keySetConfig)

  return signers
}
// ANCHOR_END: default-external-signer

// ANCHOR: connect-with-signer
const exampleConnectWithSigner = async (
  signers: ReturnType<typeof defaultExternalSigners>
) => {
  // Create the config
  const config = defaultConfig(Network.Mainnet)
  config.apiKey = '<breez api key>'

  // Connect using the external signers
  const sdk = await connectWithSigner({
    config,
    breezSigner: signers.breezSigner,
    sparkSigner: signers.sparkSigner,
    storageDir: `${RNFS.DocumentDirectoryPath}/data`
  })
}
// ANCHOR_END: connect-with-signer

export { createSigners, exampleConnectWithSigner }
