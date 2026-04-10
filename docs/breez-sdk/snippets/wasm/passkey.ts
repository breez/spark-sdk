import type { NostrRelayConfig } from '@breeztech/breez-sdk-spark'
import { Passkey, WebAuthnPrfProvider, connect, defaultConfig } from '@breeztech/breez-sdk-spark'

// ANCHOR: implement-prf-provider
// Implement the interface for custom logic if the built-in WebAuthnPrfProvider doesn't fit your needs.
class CustomPasskeyPrfProvider {
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

const connectWithPasskey = async () => {
  // ANCHOR: connect-with-passkey
  // Use the built-in WebAuthn PRF provider (or pass a custom implementation)
  const prfProvider = new WebAuthnPrfProvider()
  const passkey = new Passkey(prfProvider, undefined)

  // Construct the wallet using the passkey (pass undefined for the default wallet)
  const wallet = await passkey.getWallet('personal')

  const config = defaultConfig('mainnet')
  const sdk = await connect({ config, seed: wallet.seed, storageDir: './.data' })
  // ANCHOR_END: connect-with-passkey
  return sdk
}

const listLabels = async (): Promise<string[]> => {
  // ANCHOR: list-labels
  const prfProvider = new WebAuthnPrfProvider()
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

const storeLabel = async () => {
  // ANCHOR: store-label
  const prfProvider = new WebAuthnPrfProvider()
  const relayConfig: NostrRelayConfig = {
    breezApiKey: '<breez api key>'
  }
  const passkey = new Passkey(prfProvider, relayConfig)

  // Publish the label to Nostr for later discovery
  await passkey.storeLabel('personal')
  // ANCHOR_END: store-label
}
