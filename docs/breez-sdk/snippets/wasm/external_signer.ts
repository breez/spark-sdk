import {
  defaultExternalSigners,
  connectWithSigner,
  defaultConfig,
  SdkBuilder,
  type BreezSdk,
  type Config,
  type ExternalSigners,
  type SigningOnlyExternalSigners
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
    'breez_spark_db' // For WASM, this is the IndexedDB database name
  )
}
// ANCHOR_END: connect-with-signer

// ANCHOR: sdk-builder-with-signer
const exampleBuildWithSigner = async (
  signers: ExternalSigners
): Promise<BreezSdk> => {
  const config = defaultConfig('mainnet')
  config.apiKey = '<breez api key>'
  const builder = SdkBuilder.newWithSigner(
    config,
    signers.breezSigner,
    signers.sparkSigner
  )
  // builder = builder.withStorageBackend(<your storage backend>)
  // builder = builder.withSharedContext(<your shared context>)
  const sdk = await builder.build()
  return sdk
}
// ANCHOR_END: sdk-builder-with-signer

// ANCHOR: sdk-builder-with-signing-only-signer
const exampleBuildWithSigningOnlySigner = async (
  config: Config,
  signers: SigningOnlyExternalSigners
): Promise<BreezSdk> => {
  const builder = SdkBuilder.newWithSigningOnlySigner(
    config,
    signers.breezSigner,
    signers.sparkSigner
  )
  const sdk = await builder.build()
  return sdk
}
// ANCHOR_END: sdk-builder-with-signing-only-signer

export {
  createSigners,
  exampleConnectWithSigner,
  exampleBuildWithSigner,
  exampleBuildWithSigningOnlySigner
}
