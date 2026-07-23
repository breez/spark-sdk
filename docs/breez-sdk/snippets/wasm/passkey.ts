import type {
  PasskeyCredential
} from '@breeztech/breez-sdk-spark'
import { connect, defaultConfig } from '@breeztech/breez-sdk-spark'
import {
  PasskeyAlreadyExistsError,
  PasskeyClient,
  PasskeyProvider,
  PasskeyTimedOutError
} from '@breeztech/breez-sdk-spark/passkey-prf-provider'

// ANCHOR: implement-prf-provider
// Implement PrfProvider for a custom authenticator (hardware key, FIDO2,
// file-backed). Only deriveSeeds and isSupported are required.
class CustomPrfProvider {
  deriveSeeds = async (
    salts: string[]
  ): Promise<{ seeds: Uint8Array[], credentialId: Uint8Array | null }> => {
    // Return one 32-byte PRF output per salt, in input order.
    throw new Error('Implement using WebAuthn or native passkey APIs')
  }

  createPasskey = async (
    _excludeCredentials: Uint8Array[]
  ): Promise<PasskeyCredential> => {
    // Register a credential and return its ID plus attestation.
    throw new Error('Implement registration via WebAuthn create() / native API')
  }

  isSupported = async (): Promise<boolean> => {
    throw new Error('Check platform passkey availability')
  }
}
// ANCHOR_END: implement-prf-provider

const checkAvailability = async () => {
  const passkey = new PasskeyClient('<breez api key>', {
    providerOptions: { rpId: '<your-rp-domain>', rpName: 'Your App' }
  })

  // ANCHOR: check-availability
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
  const passkey = new PasskeyClient('<breez api key>', {
    providerOptions: { rpId: '<your-rp-domain>', rpName: 'Your App' }
  })
  // ANCHOR_END: setup-client
  return passkey
}

const connectWithPasskey = async () => {
  const passkey = new PasskeyClient('<breez api key>', {
    providerOptions: { rpId: '<your-rp-domain>', rpName: 'Your App' }
  })

  // ANCHOR: connect-with-passkey
  // Single-button flow. On web it works only where the browser supports
  // immediate mediation; supportsImmediateMediation() reports it. Otherwise
  // use the two-button flow (register / signIn).
  const availability = await passkey.checkAvailability()
  if (availability.type !== 'available' || !(await passkey.supportsImmediateMediation())) {
    throw new Error('Use the two-button flow (register / signIn) on this browser')
  }
  // No label: a returning user's wallets are discovered (response.labels,
  // with wallet being the default); a new user gets a freshly registered
  // default wallet.
  const response = await passkey.connectWithPasskey({})
  if (response.labels.length > 1) {
    // Multiple wallets: let the user pick, then signIn to the chosen label.
  }

  const config = defaultConfig('mainnet')
  const sdk = await connect({ config, seed: response.wallet.seed, storageDir: './.data' })
  // ANCHOR_END: connect-with-passkey
  return sdk
}

const signInExistingUser = async () => {
  const passkey = new PasskeyClient('<breez api key>', {
    providerOptions: { rpId: '<your-rp-domain>', rpName: 'Your App' }
  })

  // ANCHOR: sign-in
  // Returning-user sign-in. No fall-through to register.
  return await passkey.signIn({ label: 'personal' })
  // ANCHOR_END: sign-in
}

const registerNewPasskey = async () => {
  const passkey = new PasskeyClient('<breez api key>', {
    providerOptions: { rpId: '<your-rp-domain>', rpName: 'Your App' }
  })

  // ANCHOR: register-passkey
  const response = await passkey.register({ label: 'personal' })

  const config = defaultConfig('mainnet')
  const sdk = await connect({ config, seed: response.wallet.seed, storageDir: './.data' })
  // ANCHOR_END: register-passkey
  return sdk
}

const credentialMetadata = async () => {
  const passkey = new PasskeyClient('<breez api key>', {
    providerOptions: { rpId: '<your-rp-domain>', rpName: 'Your App' }
  })

  // ANCHOR: credential-metadata
  const response = await passkey.register({ label: 'personal' })

  if (response.credential != null) {
    // Persist to reopen the same wallet on sign-in
    console.log(response.credential.credentialId)
    // Authenticator model (display hint, unverified)
    console.log(response.credential.aaguid)
    // Whether the passkey syncs across devices
    console.log(response.credential.backupEligible)
  }

  // Pin the stored credential ID so the OS can't substitute a sibling
  // credential, which would derive a different wallet.
  const signInResponse = await passkey.signIn({
    label: 'personal',
    allowCredentials: [/* stored credentialId bytes */]
  })
  // Pass to connect() to open the wallet
  console.log(signInResponse.wallet.seed)
  // Label this wallet was derived from
  console.log(signInResponse.wallet.label)
  // This passkey's labels (populated on discovery sign-in)
  console.log(signInResponse.labels)
  // Credential signed in with (credential_id only)
  console.log(signInResponse.credential)
  // ANCHOR_END: credential-metadata
}

const listLabels = async (): Promise<string[]> => {
  const passkey = new PasskeyClient('<breez api key>', {
    providerOptions: { rpId: '<your-rp-domain>', rpName: 'Your App' }
  })
  // ANCHOR: list-labels
  const labels = await passkey.labels().list()
  for (const label of labels) {
    console.log(`Found label: ${label}`)
  }
  // ANCHOR_END: list-labels
  return labels
}

const storeLabel = async () => {
  const passkey = new PasskeyClient('<breez api key>', {
    providerOptions: { rpId: '<your-rp-domain>', rpName: 'Your App' }
  })
  // ANCHOR: store-label
  await passkey.labels().store('personal')
  // ANCHOR_END: store-label
}

const checkDomain = async () => {
  // ANCHOR: domain-association
  // Diagnostic only: never blocks the ceremony.
  const prfProvider = new PasskeyProvider({ rpId: '<your-rp-domain>', rpName: 'Your App' })
  const result = await prfProvider.checkDomainAssociation()

  switch (result.kind) {
    case 'Associated':
      // Safe to proceed.
      break
    case 'NotAssociated':
      // Misconfigured (entitlement, AASA, or assetlinks). Surface a dev error.
      console.error(
        `Domain association failed (source=${result.source}): ${result.reason}`
      )
      break
    case 'Skipped':
      // Could not verify (offline, no public-suffix match). Not a failure.
      break
  }
  // ANCHOR_END: domain-association
}

const recoverFromAlreadyExists = async () => {
  const passkey = new PasskeyClient('<breez api key>', {
    providerOptions: { rpId: '<your-rp-domain>', rpName: 'Your App' }
  })

  // ANCHOR: recover-already-exists
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
      // A matching credential already exists; sign in to it instead.
      const response = await passkey.signIn({ label: 'personal' })
      return response.wallet
    }
    throw error
  }
  // ANCHOR_END: recover-already-exists
}

const handleTimeout = async () => {
  const passkey = new PasskeyClient('<breez api key>', {
    providerOptions: { rpId: '<your-rp-domain>', rpName: 'Your App' }
  })

  // ANCHOR: handle-timeout
  // Biometric inactivity timeout, distinct from a user cancel.
  try {
    const response = await passkey.signIn({ label: 'personal' })
    return response
  } catch (error) {
    if (error instanceof PasskeyTimedOutError) {
      // Show a retry UI. Do NOT auto-retry without user input.
      console.log('Sign-in timed out: show "Try Again" UI.')
    }
    throw error
  }
  // ANCHOR_END: handle-timeout
}
