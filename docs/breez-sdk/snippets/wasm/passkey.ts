import type { NostrRelayConfig, RegisteredCredential } from '@breeztech/breez-sdk-spark'
import { PasskeyClient, connect, defaultConfig } from '@breeztech/breez-sdk-spark'
import { PasskeyProvider } from '@breeztech/breez-sdk-spark/passkey-prf-provider'

// ANCHOR: implement-prf-provider
// Implement the PrfProvider interface for custom logic if the built-in
// PasskeyProvider doesn't fit your needs (hardware key, FIDO2 transport,
// air-gapped backup file, etc.). Single API surface: deriveSeeds for
// derivation, createPasskey for registration, isSupported / checkDomainAssociation
// for diagnostics. Single-salt derivation is the trivial 1-element bulk case.
class CustomPrfProvider {
  deriveSeeds = async (salts: string[]): Promise<Uint8Array[]> => {
    // Call platform passkey API with PRF extension. Use the dual-salt
    // ceremony when the authenticator supports it (one OS prompt for N
    // salts) and fall back to per-salt assertions otherwise. Returns
    // one 32-byte PRF output per salt in input order.
    throw new Error('Implement using WebAuthn or native passkey APIs')
  }

  createPasskey = async (
    request: { excludeCredentialIds?: Uint8Array[]; userId?: Uint8Array; userName?: string; userDisplayName?: string }
  ): Promise<RegisteredCredential> => {
    // Register a new credential and return its ID + AAGUID + BE flag.
    throw new Error('Implement registration via WebAuthn create() / native API')
  }

  isSupported = async (): Promise<boolean> => {
    // Check if a PRF-capable authenticator is reachable from this
    // platform / browser / device.
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
  // Use the built-in passkey PRF provider (or pass a custom implementation).
  const prfProvider = new PasskeyProvider()
  const passkey = new PasskeyClient(prfProvider, undefined)

  // signIn derives the wallet seed for an existing credential. With
  // bulk PRF on iOS+Android this is a single OS prompt that derives
  // master + label seeds in one ceremony.
  const response = await passkey.signIn({ label: 'personal', extraSalts: [] })

  const config = defaultConfig('mainnet')
  const sdk = await connect({ config, seed: response.wallet.seed, storageDir: './.data' })
  // ANCHOR_END: connect-with-passkey
  return sdk
}

const registerNewPasskey = async () => {
  // ANCHOR: register-passkey
  // For a brand-new user with no existing passkey: register() creates
  // the credential AND derives the wallet seed in one orchestrated call.
  // On iOS+Android this is 2 OS prompts total (1 create + 1 dual-salt
  // assert) thanks to the SDK's bulk-PRF setup_wallet path.
  const prfProvider = new PasskeyProvider()
  const passkey = new PasskeyClient(prfProvider, undefined)

  const response = await passkey.register({
    label: 'personal',
    extraSalts: [],
    excludeCredentialIds: [],
  })

  const config = defaultConfig('mainnet')
  const sdk = await connect({ config, seed: response.wallet.seed, storageDir: './.data' })
  // ANCHOR_END: register-passkey
  return sdk
}

const listLabels = async (): Promise<string[]> => {
  // ANCHOR: list-labels
  const prfProvider = new PasskeyProvider()
  const relayConfig: NostrRelayConfig = {
    breezApiKey: '<breez api key>'
  }
  const passkey = new PasskeyClient(prfProvider, relayConfig)

  // signIn with no label runs in discovery mode: it derives the
  // master seed AND lists labels in the same ceremony, so a follow-up
  // listLabels() reads from the cached identity for free.
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
    breezApiKey: '<breez api key>'
  }
  const passkey = new PasskeyClient(prfProvider, relayConfig)

  // For a new label on an existing identity, call signIn(newLabel)
  // first to seed the SDK's identity cache via setup_wallet, THEN
  // storeLabel uses the cached identity for free (1 OS prompt total).
  await passkey.storeLabel('personal')
  // ANCHOR_END: store-label
}
