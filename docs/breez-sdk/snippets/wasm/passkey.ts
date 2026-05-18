import type {
  PasskeyConfig,
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
// air-gapped backup file, etc.). Single API surface: deriveSeeds for
// derivation, createPasskey for registration, isSupported for the
// platform probe. Single-salt derivation is the trivial 1-element bulk case.
class CustomPrfProvider {
  deriveSeeds = async (salts: string[]): Promise<Uint8Array[]> => {
    // Call platform passkey API with PRF extension. Use the dual-salt
    // ceremony when the authenticator supports it (one OS prompt for N
    // salts) and fall back to per-salt assertions otherwise. Returns
    // one 32-byte PRF output per salt in input order.
    throw new Error('Implement using WebAuthn or native passkey APIs')
  }

  createPasskey = async (
    request: { excludeCredentialIds?: Uint8Array[]; userName?: string; userDisplayName?: string }
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
  // Use the built-in passkey PRF provider (or pass a custom implementation).
  const prfProvider = new PasskeyProvider()
  const passkey = new PasskeyClient(prfProvider as any, undefined)

  // signIn derives the wallet seed for an existing credential. With
  // bulk PRF on iOS+Android this is a single OS prompt that derives
  // master + label seeds in one ceremony.
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

  const config = defaultConfig('mainnet')
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

  // signIn with no label runs in discovery mode: it derives the
  // master seed AND lists labels in the same ceremony, so a follow-up
  // labels().list() reads from the cached identity for free.
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

  // For a new label on an existing identity, call signIn(newLabel)
  // first to seed the SDK's identity cache via setup_wallet, THEN
  // labels().store() uses the cached identity for free (1 OS prompt total).
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
    // default label in a single ceremony.
    // preferImmediatelyAvailableCredentials means a fresh-device user
    // fast-fails in <300ms with no UI shown.
    const response = await passkey.signIn({ label: undefined, extraSalts: [] })
    return response.wallet
  } catch (error) {
    // CredentialNotFound is the SDK's classification for "no matching
    // credential on this device", including iOS's <300ms fast-fail
    // case where the platform conflates no-cred with user-cancel.
    // The error now carries a String payload with diagnostic detail
    // (e.g. the `CredentialRegistry` help suffix when applicable).
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
  // Hosts can branch on the SDK's typed error name. The Web JS layer
  // emits 'CredentialNotFound' both for genuine no-cred cases and for
  // the iOS <300ms fast-fail UserCancelled case (which is no-cred in
  // disguise). See uxguide_passkey.md for the full mapping table.
  return (error as { name?: string })?.name === 'CredentialNotFound'
}

const checkDomain = async () => {
  // ANCHOR: domain-association
  // Verify Apple AASA / Android Asset Links / Web Related Origins
  // before the first WebAuthn ceremony. Diagnostic only: never blocks.
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
      // Verification could not be performed (offline, endpoint
      // timeout, no public-suffix match). Proceed normally — this
      // is NOT a negative signal.
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
  // out. The SDK fires PasskeyTimedOutError when assertion or
  // register elapsed time crosses 55_000ms.
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
  // Opt-in CredentialRegistry. The SDK auto-merges stored IDs into
  // allowCredentials on assertion and excludeCredentials on
  // registration, then auto-adds new credential IDs after success.
  // Reference impl (LocalStorageCredentialRegistry) is in the
  // passkey guide; copy-paste into your app code.
  const registry = new LocalStorageCredentialRegistry()
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

// LocalStorageCredentialRegistry is a copy-paste reference impl;
// see the passkey guide. Defined here for the snippet to compile.
declare class LocalStorageCredentialRegistry {
  read(rpId: string): Promise<Uint8Array[]>
  add(rpId: string, credentialId: Uint8Array): Promise<void>
  remove(rpId: string, credentialId: Uint8Array): Promise<void>
  clear(rpId: string): Promise<void>
}
