import type {
  PasskeyConfig,
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
  createPasskeyClient,
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
    _excludeCredentialIds: Uint8Array[]
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
  // Pass `PasskeyProvider.BREEZ_RP_ID` instead of \'my-app.com\' if your
  // app is Breez-registered (shares credentials with other Breez apps).
  const config = { ...defaultConfig(Network.Mainnet), apiKey: '<breez api key>' }
  const passkey = createPasskeyClient('my-app.com', 'My App', config)

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

const connectWithPasskey = async () => {
  // ANCHOR: connect-with-passkey
  // Single-CTA onboarding: silent sign-in, fall through to register.
  const config = { ...defaultConfig(Network.Mainnet), apiKey: '<breez api key>' }
  const passkey = createPasskeyClient('my-app.com', 'My App', config)

  const response = await passkey.connectWithPasskey({
    label: 'personal',
    excludeCredentialIds: [],
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
  const config = { ...defaultConfig(Network.Mainnet), apiKey: '<breez api key>' }
  const passkey = createPasskeyClient('my-app.com', 'My App', config)

  return await passkey.signIn({ label: 'personal' })
  // ANCHOR_END: sign-in
}

const registerNewPasskey = async () => {
  // ANCHOR: register-passkey
  const config = { ...defaultConfig(Network.Mainnet), apiKey: '<breez api key>' }
  const passkey = createPasskeyClient('my-app.com', 'My App', config)

  const response = await passkey.register({ label: 'personal' })

  // Persist credentialId for future excludeCredentialIds.
  const _persist = {
    credentialId: response.credential.credentialId,
    userId: response.credential.userId,
  }

  const sdk = await connect({ config, seed: response.wallet.seed, storageDir: './.data' })
  // ANCHOR_END: register-passkey
  return sdk
}

const listLabels = async (): Promise<string[]> => {
  // ANCHOR: list-labels
  const sdkConfig = { ...defaultConfig(Network.Mainnet), apiKey: '<breez api key>' }
  const passkey = createPasskeyClient('my-app.com', 'My App', sdkConfig, {
    // Default wallet label when register / signIn receive no label.
    defaultLabel: 'personal',
  })

  const labels = await passkey.labels().list()

  for (const label of labels) {
    console.log(`Found label: ${label}`)
  }
  // ANCHOR_END: list-labels
  return labels
}

const storeLabel = async () => {
  // ANCHOR: store-label
  const config = { ...defaultConfig(Network.Mainnet), apiKey: '<breez api key>' }
  const passkey = createPasskeyClient('my-app.com', 'My App', config)

  // For a new label on an existing identity, sign in with that label first.
  await passkey.labels().store('personal')
  // ANCHOR_END: store-label
}


const isCredentialNotFound = (error: unknown): boolean => {
  // The SDK emits 'CredentialNotFound' both for genuine no-cred cases
  // and for the iOS <300ms fast-fail UserCancelled case (which is
  // no-cred in disguise). See uxguide_passkey.md for the full mapping.
  return (error as { name?: string })?.name === 'CredentialNotFound'
}

const checkDomain = async () => {
  // ANCHOR: domain-association
  // Lower-level provider call. Most hosts use `checkAvailability` instead.
  const prfProvider = new PasskeyProvider({ rpId: 'my-app.com', rpName: 'My App' })
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
  const config = { ...defaultConfig(Network.Mainnet), apiKey: '<breez api key>' }
  const passkey = createPasskeyClient('my-app.com', 'My App', config)

  try {
    await passkey.register({
      label: 'personal',
      excludeCredentialIds: [
        // app-persisted credential IDs from prior registrations
      ],
    })
  } catch (error) {
    if (error instanceof PasskeyAlreadyExistsError) {
      // Flip to sign-in. The existing credential's PRF output is
      // the same wallet seed the host would have minted on register.
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
  const config = { ...defaultConfig(Network.Mainnet), apiKey: '<breez api key>' }
  const passkey = createPasskeyClient('my-app.com', 'My App', config)

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
  // ANCHOR: with-credential-registry
  // Opt-in CredentialRegistry. The JS-side wrapper merges stored IDs
  // into allowCredentials on assertion and excludeCredentials on
  // registration, then auto-adds new credential IDs after success.
  // The native module never sees the registry: all bookkeeping is
  // done in JS.
  //
  // The SDK doesn't ship a default impl: copy the iOS Keychain or
  // Android Block Store reference impl from the passkey guide and
  // wire it up here. (Stubbed below so the snippet compiles.)
  const registry: import('@breeztech/breez-sdk-spark-react-native').CredentialRegistry =
    {
      async read(_rpId) { return [] },
      async add(_rpId, _credentialId) {},
      async remove(_rpId, _credentialId) {},
      async clear(_rpId) {},
    }

  const prfProvider = new PasskeyProvider({
    rpId: 'my-app.com',
    rpName: 'My App',
    credentialRegistry: registry,
    onRegistryError: (op, err) => console.warn('registry', op, err),
  })
  const passkey = new PasskeyClient(prfProvider as any, undefined, undefined)

  // signIn: registry IDs are auto-merged into allowCredentials.
  await passkey.signIn({ label: 'personal' })

  // register: registry IDs are auto-merged into excludeCredentials.
  await passkey.register({ label: 'personal' })

  // Inspect / mutate the registry via the credentials() sub-object.
  // get() returns the stored IDs; remove() / clear() drop entries.
  const known = await passkey.credentials().get()
  console.log(`Known credentials: ${known.length}`)

  // `clear()` drops the app's bookkeeping; existing credentials
  // stay on the OS / cloud authenticator and can be signed in with.
  await passkey.credentials().clear()
  // ANCHOR_END: with-credential-registry
}
