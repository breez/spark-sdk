import type { NostrRelayConfig } from '@breeztech/breez-sdk-spark'
import { Passkey, connect, defaultConfig } from '@breeztech/breez-sdk-spark'

// ANCHOR: implement-prf-provider
// In practice, implement PRF provider using WebAuthn API
class ExamplePasskeyPrfProvider {
  derivePrfSeed = async (salt: string): Promise<Uint8Array> => {
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
  const prfProvider = new ExamplePasskeyPrfProvider()
  const passkey = new Passkey(prfProvider, undefined)

  // Construct the wallet using the passkey (pass undefined for the default wallet)
  const wallet = await passkey.getWallet('personal')

  const config = defaultConfig('mainnet')
  const sdk = await connect({ config, seed: wallet.seed, storageDir: './.data' })
  // ANCHOR_END: connect-with-passkey
  return sdk
}

const exampleListLabels = async (): Promise<string[]> => {
  // ANCHOR: list-labels
  const prfProvider = new ExamplePasskeyPrfProvider()
  const relayConfig: NostrRelayConfig = {
    breezApiKey: '<breez api key>'
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
  const prfProvider = new ExamplePasskeyPrfProvider()
  const relayConfig: NostrRelayConfig = {
    breezApiKey: '<breez api key>'
  }
  const passkey = new Passkey(prfProvider, relayConfig)

  // Publish the label to Nostr for later discovery
  await passkey.storeLabel('personal')
  // ANCHOR_END: store-label
}
