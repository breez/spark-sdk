import type { NostrRelayConfig } from '@breeztech/breez-sdk-spark-react-native'
import {
  Passkey,
  PasskeyPrfProvider,
  connect,
  defaultConfig,
  Network
} from '@breeztech/breez-sdk-spark-react-native'

// ANCHOR: implement-prf-provider
// Use the built-in PasskeyPrfProvider, or implement the interface for custom logic.
class ExamplePasskeyPrfProvider {
  derivePrfSeed = async (salt: string): Promise<ArrayBuffer> => {
    // Call platform passkey API with PRF extension
    // Returns 32-byte PRF output
    throw new Error('Implement using WebAuthn or native passkey APIs')
  }

  isPrfAvailable = async (): Promise<boolean> => {
    // Check if PRF-capable passkey exists
    throw new Error('Check platform passkey availability')
  }
}
// ANCHOR_END: implement-prf-provider

const exampleConnectWithPasskey = async () => {
  // ANCHOR: connect-with-passkey
  // Use the built-in platform PRF provider (or pass a custom implementation)
  const prfProvider = new PasskeyPrfProvider()
  const passkey = new Passkey(prfProvider, undefined)

  // Construct the wallet using the passkey (pass undefined for the default wallet)
  const wallet = await passkey.getWallet('personal')

  const config = defaultConfig(Network.Mainnet)
  const sdk = await connect({ config, seed: wallet.seed, storageDir: './.data' })
  // ANCHOR_END: connect-with-passkey
  return sdk
}

const exampleListLabels = async (): Promise<string[]> => {
  // ANCHOR: list-labels
  const prfProvider = new PasskeyPrfProvider()
  const relayConfig: NostrRelayConfig = {
    breezApiKey: '<breez api key>',
    timeoutSecs: undefined
  }
  const passkey = new Passkey(prfProvider, relayConfig)

  // Query Nostr for labels associated with this passkey
  const labels = await passkey.listLabels()

  for (const label of labels) {
    console.log(`Found label: ${label}`)
  }
  // ANCHOR_END: list-labels
  return labels
}

const exampleStoreLabel = async () => {
  // ANCHOR: store-label
  const prfProvider = new PasskeyPrfProvider()
  const relayConfig: NostrRelayConfig = {
    breezApiKey: '<breez api key>',
    timeoutSecs: undefined
  }
  const passkey = new Passkey(prfProvider, relayConfig)

  // Publish the label to Nostr for later discovery
  await passkey.storeLabel('personal')
  // ANCHOR_END: store-label
}
