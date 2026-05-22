import type {
  RegisteredCredential,
} from '@breeztech/breez-sdk-spark'
import { PasskeyClient, connect, defaultConfig } from '@breeztech/breez-sdk-spark'
import {
  PasskeyAlreadyExistsError,
  PasskeyProvider,
  PasskeyTimedOutError,
} from '@breeztech/breez-sdk-spark/passkey-prf-provider'

// ANCHOR: implement-prf-provider
// Implement the PrfProvider interface for custom logic if the built-in
// PasskeyProvider doesn't fit your needs (hardware key, FIDO2 transport,
// air-gapped backup file, etc.). Three required methods: deriveSeeds for
// derivation, isSupported for the capability probe; createPasskey for
// registration is optional.
class CustomPrfProvider {
  deriveSeeds = async (salts: string[]): Promise<Uint8Array[]> => {
    // Call platform passkey API with PRF extension. Use the dual-salt
    // ceremony when the authenticator supports it (one OS prompt for N
    // salts) and fall back to per-salt assertions otherwise. Returns
    // one 32-byte PRF output per salt in input order.
    throw new Error('Implement using WebAuthn or native passkey APIs')
  }

  createPasskey = async (
    _excludeCredentials: Uint8Array[]
  ): Promise<RegisteredCredential> => {
    // Register a new credential and return its ID, the WebAuthn
    // user.id the provider minted for it (returned for host-side
    // correlation, never host-supplied), AAGUID, and BE flag.
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
  // `rpId` is required. Pass your app's domain, or
  // `PasskeyProvider.BREEZ_RP_ID` if your app is Breez-registered.
  const prfProvider = new PasskeyProvider({ rpId: '<your-rp-domain>', rpName: 'Your App' })
  const passkey = new PasskeyClient(prfProvider as any, undefined, undefined)

  // checkAvailability collapses isSupported + checkDomainAssociation
  // into a single tagged value. Branch on the variant the host needs.
  const availability = await passkey.checkAvailability()
  switch (availability.type) {
    case 'available':
      // Show passkey as primary option.
      break
    case 'prfUnsupported':
      // Fall back to mnemonic flow.
      break
    case 'notAssociated':
      console.error(
        `Domain association failed (source=${availability.source}): ${availability.reason}`
      )
      break
    case 'skipped':
      // No verification source on this platform; proceed normally.
      break
  }
  // ANCHOR_END: check-availability
}

const setupPasskeyClient = () => {
  // ANCHOR: setup-client
  const prfProvider = new PasskeyProvider({ rpId: '<your-rp-domain>', rpName: 'Your App' })
  const passkey = new PasskeyClient(prfProvider as any, '<breez api key>', undefined)
  // ANCHOR_END: setup-client
  return passkey
}

const connectWithPasskey = async () => {
  // ANCHOR: connect-with-passkey
  const prfProvider = new PasskeyProvider({ rpId: '<your-rp-domain>', rpName: 'Your App' })
  const passkey = new PasskeyClient(prfProvider as any, undefined, undefined)

  // signIn derives the seed for an existing credential. With
  // bulk PRF on iOS+Android this is a single OS prompt that derives
  // master + label seeds in one ceremony.
  const response = await passkey.signIn({ label: 'personal' })

  const config = defaultConfig('mainnet')
  const sdk = await connect({ config, seed: response.wallet.seed, storageDir: './.data' })
  // ANCHOR_END: connect-with-passkey
  return sdk
}

const signInExistingUser = async () => {
  // ANCHOR: sign-in
  // Returning-user-only sign-in. No fall-through to register.
  const prfProvider = new PasskeyProvider({ rpId: '<your-rp-domain>', rpName: 'Your App' })
  const passkey = new PasskeyClient(prfProvider as any, undefined, undefined)

  return await passkey.signIn({ label: 'personal' })
  // ANCHOR_END: sign-in
}

const registerNewPasskey = async () => {
  // ANCHOR: register-passkey
  // For a brand-new user with no existing passkey: register() creates
  // the credential AND derives the seed in one orchestrated call.
  // On iOS+Android this is 2 OS prompts total (1 create + 1 dual-salt
  // assert) thanks to the SDK's bulk-PRF path.
  const prfProvider = new PasskeyProvider({ rpId: '<your-rp-domain>', rpName: 'Your App' })
  const passkey = new PasskeyClient(prfProvider as any, undefined, undefined)

  const response = await passkey.register({ label: 'personal' })

  // Hosts SHOULD persist credential.credentialId (for excludeCredentials
  // bookkeeping) and credential.userId (for server-side correlation).
  // The SDK generates userId; it is never host-supplied.
  const _persist = {
    credentialId: response.credential.credentialId,
    userId: response.credential.userId,
  }

  const config = defaultConfig('mainnet')
  const sdk = await connect({ config, seed: response.wallet.seed, storageDir: './.data' })
  // ANCHOR_END: register-passkey
  return sdk
}

const listLabels = async (): Promise<string[]> => {
  const prfProvider = new PasskeyProvider({ rpId: '<your-rp-domain>', rpName: 'Your App' })
  const passkey = new PasskeyClient(prfProvider as any, '<breez api key>', undefined)
  // ANCHOR: list-labels
  const labels = await passkey.labels().list()
  for (const label of labels) {
    console.log(`Found label: ${label}`)
  }
  // ANCHOR_END: list-labels
  return labels
}

const storeLabel = async () => {
  const prfProvider = new PasskeyProvider({ rpId: '<your-rp-domain>', rpName: 'Your App' })
  const passkey = new PasskeyClient(prfProvider as any, '<breez api key>', undefined)
  // ANCHOR: store-label
  await passkey.labels().store('personal')
  // ANCHOR_END: store-label
}

const checkDomain = async () => {
  // ANCHOR: domain-association
  // Verify Apple AASA / Android Asset Links / Web Related Origins
  // before the first WebAuthn ceremony. Diagnostic only: never blocks.
  const prfProvider = new PasskeyProvider({ rpId: '<your-rp-domain>', rpName: 'Your App' })
  const result = await prfProvider.checkDomainAssociation()

  switch (result.kind) {
    case 'Associated':
      // Safe to proceed.
      break
    case 'NotAssociated':
      // Configuration is wrong (entitlement missing, AASA stale,
      // assetlinks malformed). Surface a developer-facing error.
      console.error(
        `Domain association failed (source=${result.source}): ${result.reason}`
      )
      return
    case 'Skipped':
      // Verification could not be performed (offline, endpoint
      // timeout, no public-suffix match). Proceed normally: this
      // is NOT a negative signal.
      break
  }
  // ANCHOR_END: domain-association
}

const recoverFromAlreadyExists = async () => {
  // ANCHOR: recover-already-exists
  // The OS rejected register because the user's password manager
  // already holds a credential matching `excludeCredentials`.
  // Route the user to the sign-in path: the OS picker will surface
  // the existing credential and the SDK's identity cache will warm
  // up on the assertion.
  const prfProvider = new PasskeyProvider({ rpId: '<your-rp-domain>', rpName: 'Your App' })
  const passkey = new PasskeyClient(prfProvider as any, undefined, undefined)

  try {
    await passkey.register({
      label: 'personal',
      excludeCredentials: [
        // app-persisted credential IDs from prior registrations
      ],
    })
  } catch (error) {
    if (error instanceof PasskeyAlreadyExistsError) {
      // Flip to sign-in. The existing credential's PRF output is
      // the same seed the host would have minted on register.
      const response = await passkey.signIn({ label: 'personal' })
      return response.wallet
    }
    throw error
  }
  // ANCHOR_END: recover-already-exists
}

const handleTimeout = async () => {
  // ANCHOR: handle-timeout
  // The OS biometric inactivity timeout (~55s+) tore down the prompt
  // without user intent. Distinct from a real cancel: hosts may
  // surface a re-prompt UI without treating it as the user opting
  // out. The SDK fires PasskeyTimedOutError when assertion or
  // register elapsed time crosses 55_000ms.
  const prfProvider = new PasskeyProvider({ rpId: '<your-rp-domain>', rpName: 'Your App' })
  const passkey = new PasskeyClient(prfProvider as any, undefined, undefined)

  try {
    return await passkey.signIn({ label: 'personal' })
  } catch (error) {
    if (error instanceof PasskeyTimedOutError) {
      // Show a sticky retry screen with timeout-specific copy.
      // Do NOT auto-retry without user input.
      console.log('Sign-in timed out: show "Try Again" UI.')
    }
    throw error
  }
  // ANCHOR_END: handle-timeout
}


const withCredentialRegistry = async () => {
  const registry = new LocalStorageCredentialRegistry()
  const prfProvider = new PasskeyProvider({
    rpId: '<your-rp-domain>',
    rpName: 'Your App',
    credentialRegistry: registry,
    onRegistryError: (op, err) => console.warn('registry', op, err),
  })
  const passkey = new PasskeyClient(prfProvider as any, undefined, undefined)

  await passkey.signIn({ label: 'personal' })
  await passkey.register({ label: 'personal' })
  // ANCHOR: with-credential-registry
  const known = await passkey.credentials().get()
  console.log(`Known credentials: ${known.length}`)
  // ANCHOR_END: with-credential-registry
}

// LocalStorageCredentialRegistry is a copy-paste reference impl;
// see the passkey guide. Defined here for the snippet to compile.
declare class LocalStorageCredentialRegistry {
  read(rpId: string): Promise<Uint8Array[]>
  add(rpId: string, credentialId: Uint8Array): Promise<void>
  remove(rpId: string, credentialId: Uint8Array): Promise<void>
  clear(rpId: string): Promise<void>
}
