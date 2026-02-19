import {
  Breez,
  type BreezSdk,
  type SdkCredentials,
  type ConnectOptions
} from '@breeztech/breez-sdk-spark'

// Init stub
const init = async () => {}

// ANCHOR: connect-simple
const connectSimple = async (): Promise<BreezSdk> => {
  await init()

  const credentials: SdkCredentials = {
    type: 'mnemonic',
    apiKey: '<breez api key>',
    mnemonic: '<mnemonic words>',
    passphrase: undefined
  }
  const sdk = await Breez.connect(credentials)
  return sdk
}
// ANCHOR_END: connect-simple

// ANCHOR: connect-with-options
const connectWithOptions = async (): Promise<BreezSdk> => {
  await init()

  const credentials: SdkCredentials = {
    type: 'mnemonic',
    apiKey: '<breez api key>',
    mnemonic: '<mnemonic words>',
    passphrase: undefined
  }
  const options: ConnectOptions = {
    network: 'regtest',
    storageDir: './.data'
  }
  const sdk = await Breez.connect(credentials, options)
  return sdk
}
// ANCHOR_END: connect-with-options

export { connectSimple, connectWithOptions }
