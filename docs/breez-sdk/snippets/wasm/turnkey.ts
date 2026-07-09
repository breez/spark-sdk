import {
  createTurnkeySigner,
  connectWithSigner,
  defaultConfig,
  type TurnkeyConfig
} from '@breeztech/breez-sdk-spark'

const exampleConnectWithTurnkey = async () => {
  // ANCHOR: turnkey-connect
  const turnkeyConfig: TurnkeyConfig = {
    baseUrl: undefined,
    organizationId: '<turnkey sub-organization id>',
    apiPublicKey: '<api public key hex>',
    apiPrivateKey: '<api private key hex>',
    walletId: '<turnkey wallet id>',
    network: 'mainnet',
    accountNumber: undefined,
    // Set after the first connect to make later signer setup network-free
    identityPublicKey: undefined,
    retry: undefined,
    maxRps: undefined
  }

  const signers = await createTurnkeySigner(turnkeyConfig)

  const config = defaultConfig('mainnet')
  config.apiKey = '<breez api key>'

  const sdk = await connectWithSigner(
    config,
    signers.breezSigner,
    signers.sparkSigner,
    'breez_spark_db' // For WASM, this is the IndexedDB database name
  )
  // ANCHOR_END: turnkey-connect
  return sdk
}

export { exampleConnectWithTurnkey }
