import {
  createTurnkeySigner,
  connectWithSigner,
  defaultConfig,
  Network,
  type TurnkeyConfig
} from '@breeztech/breez-sdk-spark-react-native'
import RNFS from 'react-native-fs'

const connectWithTurnkey = async () => {
  // ANCHOR: turnkey-connect
  const turnkeyConfig: TurnkeyConfig = {
    baseUrl: undefined,
    organizationId: '<turnkey sub-organization id>',
    apiPublicKey: '<api public key hex>',
    apiPrivateKey: '<api private key hex>',
    walletId: '<turnkey wallet id>',
    network: Network.Mainnet,
    accountNumber: undefined,
    // Set after the first connect to make later signer setup network-free
    identityPublicKey: undefined,
    retry: undefined,
    maxRps: undefined
  }

  const signers = await createTurnkeySigner(turnkeyConfig)

  const config = defaultConfig(Network.Mainnet)
  config.apiKey = '<breez api key>'

  const sdk = await connectWithSigner({
    config,
    breezSigner: signers.breezSigner,
    sparkSigner: signers.sparkSigner,
    storageDir: `${RNFS.DocumentDirectoryPath}/data`
  })
  // ANCHOR_END: turnkey-connect
  return sdk
}

export { connectWithTurnkey }
