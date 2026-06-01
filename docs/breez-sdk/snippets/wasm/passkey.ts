import type {
  PasskeyCredential
} from '@breeztech/breez-sdk-spark'
import { PasskeyClient, connect, defaultConfig } from '@breeztech/breez-sdk-spark'
import {
  PasskeyAlreadyExistsError,
  PasskeyProvider,
  PasskeyTimedOutError
} from '@breeztech/breez-sdk-spark/passkey-prf-provider'

// ANCHOR: implement-prf-provider
// Implement the PrfProvider interface for custom logic if the built-in
// PasskeyProvider doesn't fit your needs (hardware key, FIDO2 transport,
// air-gapped backup file, etc.). Three required methods: deriveSeeds for
// derivation, isSupported for the capability probe; createPasskey for
// registration is optional.
class CustomPrfProvider {
  deriveSeeds = async (salts: string[]): Promise<{ seeds: Uint8Array[], credentialId: Uint8Array | null }> => {
    // Call platform passkey API with PRF extension. Use the dual-salt
    // ceremony when the authenticator supports it (one OS prompt for N
    // salts) and fall back to per-salt assertions otherwise. Returns
    // one 32-byte PRF output per salt in input order.
    throw new Error('Implement using WebAuthn or native passkey APIs')
  }

  createPasskey = async (
    _excludeCredentials: Uint8Array[]
  ): Promise<PasskeyCredential> => {
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
  const response = await passkey.signIn({ label: 'personal', allowCredentials: [] })

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

  return await passkey.signIn({ label: 'personal', allowCredentials: [] })
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

  const response = await passkey.register({ label: 'personal', excludeCredentials: [] })

  // Hosts SHOULD persist credentialId (for excludeCredentials
  // bookkeeping) and userId (for server-side correlation).
  // The SDK generates userId; it is never host-supplied.
  const _persist = {
    credentialId: response.credential?.credentialId,
    userId: response.credential?.userId
  }

  const config = defaultConfig('mainnet')
  const sdk = await connect({ config, seed: response.wallet.seed, storageDir: './.data' })
  // ANCHOR_END: register-passkey
  return sdk
}

const credentialMetadata = async () => {
  // ANCHOR: credential-metadata
  const prfProvider = new PasskeyProvider({ rpId: '<your-rp-domain>', rpName: 'Your App' })
  const passkey = new PasskeyClient(prfProvider as any, undefined, undefined)

  const response = await passkey.register({ label: 'personal', excludeCredentials: [] })

  // Persist these in synced storage (iCloud Keychain / Block Store) so they
  // survive reinstall and reach the user's other devices. aaguid and
  // backupEligible are only available here, on registration.
  if (response.credential != null) {
    const _meta = {
      credentialId: response.credential.credentialId,
      aaguid: response.credential.aaguid,
      backupEligible: response.credential.backupEligible
    }
  }

  // On a later sign-in, pin the stored credential ID via allowCredentials so
  // the OS cannot substitute a sibling credential, which would derive a
  // different wallet seed.
  const _signedIn = await passkey.signIn({
    label: 'personal',
    allowCredentials: [/* stored credentialId bytes */]
  })
  // ANCHOR_END: credential-metadata
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
      break
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
    const response = await passkey.register({
      label: 'personal',
      excludeCredentials: [
        // app-persisted credential IDs from prior registrations
      ]
    })
    return response.wallet
  } catch (error) {
    if (error instanceof PasskeyAlreadyExistsError) {
      // Flip to sign-in. The existing credential's PRF output is
      // the same seed the host would have minted on register.
      const response = await passkey.signIn({ label: 'personal', allowCredentials: [] })
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
    const response = await passkey.signIn({ label: 'personal', allowCredentials: [] })
    return response
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
