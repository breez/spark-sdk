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
} from '@breeztech/breez-sdk-spark-react-native/passkey-prf-provider'

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
    request: { excludeCredentialIds?: Uint8Array[]; userName?: string; userDisplayName?: string }
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
  const prfProvider = new PasskeyProvider()
  const passkey = new PasskeyClient(prfProvider as any, undefined)

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

const connectWithPasskey = async () => {
  // ANCHOR: connect-with-passkey
  // Use the built-in platform PRF provider (or pass a custom implementation).
  const prfProvider = new PasskeyProvider()
  const passkey = new PasskeyClient(prfProvider as any, undefined)

  // signIn derives the wallet seed for an existing credential. With
  // bulk PRF on iOS+Android this is one OS prompt for master + label.
  //
  // Per-call shaping:
  // - allowCredentialIds: server-driven sign-in passes the user's known
  //   credential IDs from /passkey/options here so the assertion is
  //   pinned to credentials the server has on record. Empty (default)
  //   lets the platform pick any matching credential.
  // - preferImmediatelyAvailableCredentials: true (default) suppresses
  //   the cross-device QR / hybrid picker so a missing local credential
  //   surfaces as CredentialNotFound; pass false to allow cross-device
  //   sign-in.
  const response = await passkey.signIn({
    label: 'personal',
    extraSalts: [],
    allowCredentialIds: [],
    preferImmediatelyAvailableCredentials: true,
  })

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

  // Hosts SHOULD persist credential.credentialId (for excludeCredentialIds
  // bookkeeping) and credential.userId (for server-side correlation).
  // The SDK generates userId; it is never host-supplied.
  const _persist = {
    credentialId: response.credential.credentialId,
    userId: response.credential.userId,
  }

  const config = defaultConfig(Network.Mainnet)
  const sdk = await connect({ config, seed: response.wallet.seed, storageDir: './.data' })
  // ANCHOR_END: register-passkey
  return sdk
}

const listLabels = async (): Promise<string[]> => {
  // ANCHOR: list-labels
  const prfProvider = new PasskeyProvider()
  const config: PasskeyConfig = {
    breezApiKey: '<breez api key>',
    // Optional: override the default wallet label used when register /
    // signIn receive `label = undefined`. Falls back to the SDK's
    // internal "Default" when unset.
    defaultLabel: 'personal',
  }
  const passkey = new PasskeyClient(prfProvider as any, config)

  // signIn with discovery mode (no label) lists labels in the same
  // ceremony; subsequent labels().list() reads from the cached identity.
  const labels = await passkey.labels().list()

  for (const label of labels) {
    console.log(`Found label: ${label}`)
  }
  // ANCHOR_END: list-labels
  return labels
}

const storeLabel = async () => {
  // ANCHOR: store-label
  const prfProvider = new PasskeyProvider()
  const config: PasskeyConfig = {
    breezApiKey: '<breez api key>',
  }
  const passkey = new PasskeyClient(prfProvider as any, config)

  // For a new label on an existing identity, signIn(newLabel) seeds the
  // identity cache via setup_wallet, then labels().store() runs free off
  // the cached identity (1 OS prompt total).
  await passkey.labels().store('personal')
  // ANCHOR_END: store-label
}

const singleCtaOnboarding = async () => {
  // ANCHOR: signin-fallback-register
  // Single-CTA onboarding: try silent signIn first, fall through to
  // register on CredentialNotFound. The OS shows ONE prompt for a
  // returning user (silent assertion succeeds), TWO for a new user
  // (silent assertion fast-fails, then create + dual-salt assert).
  const prfProvider = new PasskeyProvider()
  const passkey = new PasskeyClient(prfProvider as any, undefined)

  try {
    // Discovery mode (label undefined): derives master + configured
    // default label in a single ceremony. The fresh-device user
    // fast-fails in <300ms with no UI shown.
    const response = await passkey.signIn({ label: undefined, extraSalts: [] })
    return response.wallet
  } catch (error) {
    // CredentialNotFound now carries a diagnostic message (including
    // the `CredentialRegistry` help suffix when applicable); branching
    // on the typed name still works regardless of payload.
    if (!isCredentialNotFound(error)) throw error

    // No credential. Onboard a new user.
    const response = await passkey.register({
      label: 'personal',
      extraSalts: [],
      excludeCredentialIds: [],
    })
    return response.wallet
  }
  // ANCHOR_END: signin-fallback-register
}

const isCredentialNotFound = (error: unknown): boolean => {
  // The SDK emits 'CredentialNotFound' both for genuine no-cred cases
  // and for the iOS <300ms fast-fail UserCancelled case (which is
  // no-cred in disguise). See uxguide_passkey.md for the full mapping.
  return (error as { name?: string })?.name === 'CredentialNotFound'
}

const checkDomain = async () => {
  // ANCHOR: domain-association
  // Verify Apple AASA / Android Asset Links before the first WebAuthn
  // ceremony. Diagnostic only: never blocks.
  const prfProvider = new PasskeyProvider()
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
      // Proceed normally — this is NOT a negative signal.
      break
  }
  // ANCHOR_END: domain-association
}

const recoverFromAlreadyExists = async () => {
  // ANCHOR: recover-already-exists
  // The OS rejected register because the user's password manager
  // already holds a credential matching `excludeCredentialIds`.
  // Route the user to the sign-in path: the OS picker will surface
  // the existing credential and the SDK's identity cache will warm
  // up on the assertion.
  const prfProvider = new PasskeyProvider()
  const passkey = new PasskeyClient(prfProvider as any, undefined)

  try {
    await passkey.register({
      label: 'personal',
      extraSalts: [],
      excludeCredentialIds: [
        // app-persisted credential IDs from prior registrations
      ],
    })
  } catch (error) {
    if (error instanceof PasskeyAlreadyExistsError) {
      // Flip to sign-in. The existing credential's PRF output is
      // the same wallet seed the host would have minted on register.
      const response = await passkey.signIn({
        label: 'personal',
        extraSalts: [],
      })
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
  // out. The SDK fires PasskeyTimedOutError when assertion or register
  // elapsed time crosses 55_000ms.
  const prfProvider = new PasskeyProvider()
  const passkey = new PasskeyClient(prfProvider as any, undefined)

  try {
    return await passkey.signIn({ label: 'personal', extraSalts: [] })
  } catch (error) {
    if (error instanceof PasskeyTimedOutError) {
      // Show a sticky retry screen with timeout-specific copy.
      // Do NOT auto-retry without user input.
      console.log('Sign-in timed out — show "Try Again" UI.')
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
    credentialRegistry: registry,
    onRegistryError: (op, err) => console.warn('registry', op, err),
  })
  const passkey = new PasskeyClient(prfProvider as any, undefined)

  // signIn: registry IDs are auto-merged into allowCredentials.
  await passkey.signIn({ label: 'personal', extraSalts: [] })

  // register: registry IDs are auto-merged into excludeCredentials.
  await passkey.register({ label: 'personal', extraSalts: [] })

  // Inspect / mutate the registry via the credentials() sub-object.
  // get() returns the stored IDs; remove() / clear() drop entries.
  const known = await passkey.credentials().get()
  console.log(`Known credentials: ${known.length}`)

  // On logout, clear the registry so a fresh device-pairing
  // wouldn't pin to the old credential.
  await passkey.credentials().clear()
  // ANCHOR_END: with-credential-registry
}
