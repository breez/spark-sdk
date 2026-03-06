import type { NostrRelayConfig } from '@breeztech/breez-sdk-spark-react-native'
import {
  Passkey,
  connect,
  defaultConfig,
  Network
} from '@breeztech/breez-sdk-spark-react-native'

// ANCHOR: implement-prf-provider
// In practice, implement PRF provider using platform passkey APIs
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
  const prfProvider = new ExamplePasskeyPrfProvider()
  const passkey = new Passkey(prfProvider, undefined)

  // Construct the wallet using the passkey (pass undefined for the default wallet)
  const wallet = await passkey.getWallet('personal')

  const config = defaultConfig(Network.Mainnet)
  const sdk = await connect({ config, seed: wallet.seed, storageDir: './.data' })
  // ANCHOR_END: connect-with-passkey
  return sdk
}

const exampleListWalletNames = async (): Promise<string[]> => {
  // ANCHOR: list-wallet-names
  const prfProvider = new ExamplePasskeyPrfProvider()
  const relayConfig: NostrRelayConfig = {
    breezApiKey: '<breez api key>',
    timeoutSecs: undefined
  }
  const passkey = new Passkey(prfProvider, relayConfig)

  // Query Nostr for wallet names associated with this passkey
  const walletNames = await passkey.listWalletNames()

  for (const walletName of walletNames) {
    console.log(`Found wallet: ${walletName}`)
  }
  // ANCHOR_END: list-wallet-names
  return walletNames
}

const exampleStoreWalletName = async () => {
  // ANCHOR: store-wallet-name
  const prfProvider = new ExamplePasskeyPrfProvider()
  const relayConfig: NostrRelayConfig = {
    breezApiKey: '<breez api key>',
    timeoutSecs: undefined
  }
  const passkey = new Passkey(prfProvider, relayConfig)

  // Publish the wallet name to Nostr for later discovery
  await passkey.storeWalletName('personal')
  // ANCHOR_END: store-wallet-name
}
