import type { NostrRelayConfig, RegisteredCredential } from '@breeztech/breez-sdk-spark-react-native'
import {
  PasskeyClient,
  connect,
  defaultConfig,
  Network
} from '@breeztech/breez-sdk-spark-react-native'
import { PasskeyProvider } from '@breeztech/breez-sdk-spark-react-native/passkey-prf-provider'

// ANCHOR: implement-prf-provider
// Implement the PrfProvider interface for custom logic if the built-in
// PasskeyProvider doesn't fit your needs. Single API surface: deriveSeeds
// for derivation, createPasskey for registration, isSupported for
// availability. Single-salt is the trivial 1-element bulk case.
class CustomPrfProvider {
  deriveSeeds = async (salts: string[]): Promise<Uint8Array[]> => {
    // Call platform passkey API with PRF extension. Returns one 32-byte
    // output per salt in input order.
    throw new Error('Implement using WebAuthn or native passkey APIs')
  }

  createPasskey = async (
    request: { excludeCredentialIds?: Uint8Array[]; userId?: Uint8Array; userName?: string; userDisplayName?: string }
  ): Promise<RegisteredCredential> => {
    throw new Error('Implement registration via native passkey API')
  }

  isSupported = async (): Promise<boolean> => {
    throw new Error('Check platform passkey availability')
  }
}
// ANCHOR_END: implement-prf-provider

const checkAvailability = async () => {
  // ANCHOR: check-availability
  const prfProvider = new PasskeyProvider()
  if (await prfProvider.isSupported()) {
    // Show passkey as primary option
  } else {
    // Fall back to mnemonic flow
  }
  // ANCHOR_END: check-availability
}

const connectWithPasskey = async () => {
  // ANCHOR: connect-with-passkey
  // Use the built-in platform PRF provider (or pass a custom implementation).
  const prfProvider = new PasskeyProvider()
  const passkey = new PasskeyClient(prfProvider as any, undefined)

  // signIn derives the wallet seed for an existing credential. With
  // bulk PRF on iOS+Android this is one OS prompt for master + label.
  const response = await passkey.signIn({ label: 'personal', extraSalts: [] })

  const config = defaultConfig(Network.Mainnet)
  const sdk = await connect({ config, seed: response.wallet.seed, storageDir: './.data' })
  // ANCHOR_END: connect-with-passkey
  return sdk
}

const registerNewPasskey = async () => {
  // ANCHOR: register-passkey
  // For a brand-new user: register() creates the credential AND derives
  // the wallet seed in one orchestrated call. 2 OS prompts on iOS+Android
  // (1 create + 1 dual-salt assert) thanks to the SDK's bulk-PRF path.
  const prfProvider = new PasskeyProvider()
  const passkey = new PasskeyClient(prfProvider as any, undefined)

  const response = await passkey.register({
    label: 'personal',
    extraSalts: [],
    excludeCredentialIds: [],
  })

  const config = defaultConfig(Network.Mainnet)
  const sdk = await connect({ config, seed: response.wallet.seed, storageDir: './.data' })
  // ANCHOR_END: register-passkey
  return sdk
}

const listLabels = async (): Promise<string[]> => {
  // ANCHOR: list-labels
  const prfProvider = new PasskeyProvider()
  const relayConfig: NostrRelayConfig = {
    breezApiKey: '<breez api key>',
    timeoutSecs: undefined
  }
  const passkey = new PasskeyClient(prfProvider as any, relayConfig)

  // signIn with discovery mode (no label) lists labels in the same
  // ceremony; subsequent listLabels reads from the cached identity.
  const labels = await passkey.listLabels()

  for (const label of labels) {
    console.log(`Found label: ${label}`)
  }
  // ANCHOR_END: list-labels
  return labels
}

const storeLabel = async () => {
  // ANCHOR: store-label
  const prfProvider = new PasskeyProvider()
  const relayConfig: NostrRelayConfig = {
    breezApiKey: '<breez api key>',
    timeoutSecs: undefined
  }
  const passkey = new PasskeyClient(prfProvider as any, relayConfig)

  // For a new label on an existing identity, signIn(newLabel) seeds the
  // identity cache via setup_wallet, then storeLabel runs free off the
  // cached identity (1 OS prompt total).
  await passkey.storeLabel('personal')
  // ANCHOR_END: store-label
}
