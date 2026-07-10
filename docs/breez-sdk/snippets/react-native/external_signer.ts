import {
  type Config,
  type ExternalSigners,
  type SigningOnlyExternalSigners,
  SdkBuilder,
  defaultExternalSigners,
  connectWithSigner,
  defaultConfig,
  Network
} from '@breeztech/breez-sdk-spark-react-native'
import RNFS from 'react-native-fs'

// ANCHOR: default-external-signer
const createSigners = () => {
  const mnemonic = '<mnemonic words>'
  const accountNumber = 0

  // Create the default signers from the SDK
  const signers = defaultExternalSigners(mnemonic, undefined, Network.Mainnet, accountNumber)

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

// ANCHOR: sdk-builder-with-signer
const exampleBuildWithSigner = async (signers: ExternalSigners) => {
  // Create the config
  const config = defaultConfig(Network.Mainnet)
  config.apiKey = '<breez api key>'

  const builder = SdkBuilder.newWithSigner(config, signers.breezSigner, signers.sparkSigner)
  // await builder.withStorage(<your storage implementation>)
  // await builder.withAccountNumber(<account number>)
  const sdk = await builder.build()
}
// ANCHOR_END: sdk-builder-with-signer

// ANCHOR: sdk-builder-with-signing-only-signer
const exampleBuildWithSigningOnlySigner = async (
  config: Config,
  signers: SigningOnlyExternalSigners
) => {
  const builder = SdkBuilder.newWithSigningOnlySigner(
    config,
    signers.breezSigner,
    signers.sparkSigner
  )
  const sdk = await builder.build()
}
// ANCHOR_END: sdk-builder-with-signing-only-signer

export {
  createSigners,
  exampleConnectWithSigner,
  exampleBuildWithSigner,
  exampleBuildWithSigningOnlySigner
}
