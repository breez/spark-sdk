import type {
  PasskeyCredential
} from '@breeztech/breez-sdk-spark-react-native'
import {
  PasskeyAvailability_Tags,
  PasskeyConfig,
  PasskeyProviderOptions,
  connect,
  defaultConfig,
  Network
} from '@breeztech/breez-sdk-spark-react-native'
import {
  PasskeyClient,
  PasskeyPrfException,
  PasskeyProvider
} from '@breeztech/breez-sdk-spark-react-native/passkey-prf-provider'

// ANCHOR: implement-prf-provider
// Implement PrfProvider for a custom authenticator (hardware key, FIDO2,
// file-backed). Only deriveSeeds and isSupported are required.
class CustomPrfProvider {
  deriveSeeds = async (_request: { salts: string[] }): Promise<{ seeds: Uint8Array[], credentialId?: Uint8Array }> => {
    // Return one 32-byte PRF output per salt, in input order.
    throw new Error('Implement using WebAuthn or native passkey APIs')
  }

  createPasskey = async (
    _excludeCredentials: Uint8Array[]
  ): Promise<PasskeyCredential> => {
    // Register a credential and return its ID plus attestation.
    throw new Error('Implement registration via native passkey API')
  }

  isSupported = async (): Promise<boolean> => {
    throw new Error('Check platform passkey availability')
  }
}
// ANCHOR_END: implement-prf-provider

const checkAvailability = async () => {
  const passkey = new PasskeyClient(
    '<breez api key>',
    PasskeyConfig.create({
      providerOptions: PasskeyProviderOptions.create({ rpId: '<your-rp-domain>', rpName: 'Your App' })
    })
  )

  // ANCHOR: check-availability
  const availability = await passkey.checkAvailability()
  switch (availability.tag) {
    case PasskeyAvailability_Tags.Available:
      // Show passkey as primary option.
      break
    case PasskeyAvailability_Tags.PrfUnsupported:
      // Fall back to mnemonic flow.
      break
    case PasskeyAvailability_Tags.NotAssociated:
      console.error(
        `Domain association failed (source=${availability.inner.source}): ${availability.inner.reason}`
      )
      break
    case PasskeyAvailability_Tags.Skipped:
      // No verification source on this platform; proceed normally.
      break
  }
  // ANCHOR_END: check-availability
}

const setupPasskeyClient = () => {
  // ANCHOR: setup-client
  const passkey = new PasskeyClient(
    '<breez api key>',
    PasskeyConfig.create({
      providerOptions: PasskeyProviderOptions.create({ rpId: '<your-rp-domain>', rpName: 'Your App' })
    })
  )
  // ANCHOR_END: setup-client
  return passkey
}

const connectWithPasskey = async () => {
  const passkey = new PasskeyClient(
    '<breez api key>',
    PasskeyConfig.create({
      providerOptions: PasskeyProviderOptions.create({ rpId: '<your-rp-domain>', rpName: 'Your App' })
    })
  )

  // ANCHOR: connect-with-passkey
  // Silent sign-in, fall through to register.
  const config = { ...defaultConfig(Network.Mainnet), apiKey: '<breez api key>' }
  const response = await passkey.connectWithPasskey({ label: 'personal', allowCredentials: undefined, excludeCredentials: undefined })

  const sdk = await connect({ config, seed: response.wallet.seed, storageDir: './.data' })
  // ANCHOR_END: connect-with-passkey
  return sdk
}

const signInExistingUser = async () => {
  const passkey = new PasskeyClient(
    '<breez api key>',
    PasskeyConfig.create({
      providerOptions: PasskeyProviderOptions.create({ rpId: '<your-rp-domain>', rpName: 'Your App' })
    })
  )

  // ANCHOR: sign-in
  // Returning-user sign-in. No fall-through to register.
  return await passkey.signIn({ label: 'personal', allowCredentials: undefined, preferImmediatelyAvailableCredentials: undefined })
  // ANCHOR_END: sign-in
}

const registerNewPasskey = async () => {
  const passkey = new PasskeyClient(
    '<breez api key>',
    PasskeyConfig.create({
      providerOptions: PasskeyProviderOptions.create({ rpId: '<your-rp-domain>', rpName: 'Your App' })
    })
  )

  // ANCHOR: register-passkey
  const config = { ...defaultConfig(Network.Mainnet), apiKey: '<breez api key>' }
  const response = await passkey.register({ label: 'personal', excludeCredentials: undefined })

  const sdk = await connect({ config, seed: response.wallet.seed, storageDir: './.data' })
  // ANCHOR_END: register-passkey
  return sdk
}

const credentialMetadata = async () => {
  const passkey = new PasskeyClient(
    '<breez api key>',
    PasskeyConfig.create({
      providerOptions: PasskeyProviderOptions.create({ rpId: '<your-rp-domain>', rpName: 'Your App' })
    })
  )

  // ANCHOR: credential-metadata
  const response = await passkey.register({ label: 'personal', excludeCredentials: undefined })

  if (response.credential !== undefined) {
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
    allowCredentials: [/* stored credentialId bytes */],
    preferImmediatelyAvailableCredentials: undefined
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
  const passkey = new PasskeyClient(
    '<breez api key>',
    PasskeyConfig.create({
      providerOptions: PasskeyProviderOptions.create({ rpId: '<your-rp-domain>', rpName: 'Your App' })
    })
  )
  // ANCHOR: list-labels
  const labels = await passkey.labels().list()
  for (const label of labels) {
    console.log(`Found label: ${label}`)
  }
  // ANCHOR_END: list-labels
  return labels
}

const storeLabel = async () => {
  const passkey = new PasskeyClient(
    '<breez api key>',
    PasskeyConfig.create({
      providerOptions: PasskeyProviderOptions.create({ rpId: '<your-rp-domain>', rpName: 'Your App' })
    })
  )
  // ANCHOR: store-label
  await passkey.labels().store('personal')
  // ANCHOR_END: store-label
}

const checkDomain = async () => {
  // ANCHOR: domain-association
  // Diagnostic only: never blocks the ceremony.
  const prfProvider = new PasskeyProvider(PasskeyProviderOptions.create({ rpId: '<your-rp-domain>', rpName: 'Your App' }))
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
  const passkey = new PasskeyClient(
    '<breez api key>',
    PasskeyConfig.create({
      providerOptions: PasskeyProviderOptions.create({ rpId: '<your-rp-domain>', rpName: 'Your App' })
    })
  )

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
    if (error instanceof PasskeyPrfException && error.code === 'credentialAlreadyExists') {
      // A matching credential already exists; sign in to it instead.
      const response = await passkey.signIn({ label: 'personal', allowCredentials: undefined, preferImmediatelyAvailableCredentials: undefined })
      return response.wallet
    }
    throw error
  }
  // ANCHOR_END: recover-already-exists
}

const handleTimeout = async () => {
  const passkey = new PasskeyClient(
    '<breez api key>',
    PasskeyConfig.create({
      providerOptions: PasskeyProviderOptions.create({ rpId: '<your-rp-domain>', rpName: 'Your App' })
    })
  )

  // ANCHOR: handle-timeout
  // Biometric inactivity timeout, distinct from a user cancel.
  try {
    const response = await passkey.signIn({ label: 'personal', allowCredentials: undefined, preferImmediatelyAvailableCredentials: undefined })
    return response
  } catch (error) {
    if (error instanceof PasskeyPrfException && error.code === 'userTimedOut') {
      // Show a retry UI. Do NOT auto-retry without user input.
      console.log('Sign-in timed out: show "Try Again" UI.')
    }
    throw error
  }
  // ANCHOR_END: handle-timeout
}
