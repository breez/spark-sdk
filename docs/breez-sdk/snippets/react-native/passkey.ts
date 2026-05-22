import type {
  RegisteredCredential,
} from '@breeztech/breez-sdk-spark-react-native'
import {
  PasskeyClient,
  connect,
  defaultConfig,
  Network,
} from '@breeztech/breez-sdk-spark-react-native'
import {
  PasskeyAlreadyExistsError,
  PasskeyProvider,
  PasskeyTimedOutError,
} from '@breeztech/breez-sdk-spark-react-native/passkey-prf-provider'

// ANCHOR: implement-prf-provider
// Implement the PrfProvider interface for custom logic if the built-in
// PasskeyProvider doesn't fit your needs. Three required methods:
// deriveSeeds for derivation, isSupported for the capability probe;
// createPasskey for registration is optional.
class CustomPrfProvider {
  deriveSeeds = async (_salts: string[]): Promise<Uint8Array[]> => {
    // Call platform passkey API with PRF extension. Returns one 32-byte
    // output per salt in input order.
    throw new Error('Implement using WebAuthn or native passkey APIs')
  }

  createPasskey = async (
    _excludeCredentials: Uint8Array[]
  ): Promise<RegisteredCredential> => {
    // Register a new credential and return its ID, the WebAuthn user.id
    // the native plugin minted for it (returned for host-side
    // correlation, never host-supplied), AAGUID, and BE flag.
    throw new Error('Implement registration via native passkey API')
  }

  isSupported = async (): Promise<boolean> => {
    throw new Error('Check platform passkey availability')
  }
}
// ANCHOR_END: implement-prf-provider

const checkAvailability = async () => {
  // ANCHOR: check-availability
  // Pass `PasskeyProvider.BREEZ_RP_ID` instead of \'<your-rp-domain>\' if your
  // app is Breez-registered (shares credentials with other Breez apps).
  const prfProvider = new PasskeyProvider({ rpId: '<your-rp-domain>', rpName: 'Your App' })
  const passkey = new PasskeyClient(prfProvider as any, '<breez api key>', undefined)

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
  // Single-CTA onboarding: silent sign-in, fall through to register.
  const config = { ...defaultConfig(Network.Mainnet), apiKey: '<breez api key>' }
  const prfProvider = new PasskeyProvider({ rpId: '<your-rp-domain>', rpName: 'Your App' })
  const passkey = new PasskeyClient(prfProvider as any, config.apiKey, undefined)

  const response = await passkey.connectWithPasskey({
    label: 'personal',
    excludeCredentials: [],
  })

  // `registeredCredential` is the path discriminator (undefined on sign-in).
  if (response.registeredCredential) {
    const _persist = response.registeredCredential.credentialId
  }

  const sdk = await connect({ config, seed: response.wallet.seed, storageDir: './.data' })
  // ANCHOR_END: connect-with-passkey
  return sdk
}

const signInExistingUser = async () => {
  // ANCHOR: sign-in
  // Returning-user-only sign-in. No fall-through to register: use
  // `connectWithPasskey` when you also want the new-user path.
  const prfProvider = new PasskeyProvider({ rpId: '<your-rp-domain>', rpName: 'Your App' })
  const passkey = new PasskeyClient(prfProvider as any, '<breez api key>', undefined)

  return await passkey.signIn({ label: 'personal' })
  // ANCHOR_END: sign-in
}

const registerNewPasskey = async () => {
  // ANCHOR: register-passkey
  const config = { ...defaultConfig(Network.Mainnet), apiKey: '<breez api key>' }
  const prfProvider = new PasskeyProvider({ rpId: '<your-rp-domain>', rpName: 'Your App' })
  const passkey = new PasskeyClient(prfProvider as any, config.apiKey, undefined)

  const response = await passkey.register({ label: 'personal' })

  // Persist credentialId for future excludeCredentials.
  const _persist = {
    credentialId: response.credential.credentialId,
    userId: response.credential.userId,
  }

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
  // Lower-level provider call. Most hosts use `checkAvailability` instead.
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
      // Verification could not be performed (offline, endpoint timeout).
      // Proceed normally: this is NOT a negative signal.
      break
  }
  // ANCHOR_END: domain-association
}

const recoverFromAlreadyExists = async () => {
  // ANCHOR: recover-already-exists
  // Recovery: flip to sign-in so the OS picker surfaces the existing credential.
  const prfProvider = new PasskeyProvider({ rpId: '<your-rp-domain>', rpName: 'Your App' })
  const passkey = new PasskeyClient(prfProvider as any, '<breez api key>', undefined)

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
  // Timeout is distinct from a cancel: surface a re-prompt UI.
  const prfProvider = new PasskeyProvider({ rpId: '<your-rp-domain>', rpName: 'Your App' })
  const passkey = new PasskeyClient(prfProvider as any, '<breez api key>', undefined)

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
  const registry: import('@breeztech/breez-sdk-spark-react-native').CredentialRegistry =
    {
      async read(_rpId) { return [] },
      async add(_rpId, _credentialId) {},
      async remove(_rpId, _credentialId) {},
      async clear(_rpId) {},
    }

  const prfProvider = new PasskeyProvider({
    rpId: '<your-rp-domain>',
    rpName: 'Your App',
    credentialRegistry: registry,
    onRegistryError: (op, err) => console.warn('registry', op, err),
  })
  const passkey = new PasskeyClient(prfProvider as any, undefined, undefined)
  // ANCHOR: with-credential-registry
  const known = await passkey.credentials().get()
  console.log(`Known credentials: ${known.length}`)
  // ANCHOR_END: with-credential-registry
}
