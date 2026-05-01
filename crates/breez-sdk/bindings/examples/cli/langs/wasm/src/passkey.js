'use strict'

const crypto = require('crypto')
const fs = require('fs')
const path = require('path')
const readline = require('readline')

const { Passkey } = require('@breeztech/breez-sdk-spark/nodejs')

// ---------------------------------------------------------------------------
// Passkey provider constants
// ---------------------------------------------------------------------------

const PASSKEY_PROVIDER_FILE = 'file'
const PASSKEY_PROVIDER_YUBIKEY = 'yubikey'
const PASSKEY_PROVIDER_FIDO2 = 'fido2'

/**
 * Parse a passkey provider string into a validated provider name.
 *
 * @param {string} s - The provider name
 * @returns {string} The validated provider name
 */
function parsePasskeyProvider(s) {
  switch (s.toLowerCase()) {
    case 'file':
      return PASSKEY_PROVIDER_FILE
    case 'yubikey':
      return PASSKEY_PROVIDER_YUBIKEY
    case 'fido2':
      return PASSKEY_PROVIDER_FIDO2
    default:
      throw new Error(`Invalid passkey provider '${s}' (valid: file, yubikey, fido2)`)
  }
}

// ---------------------------------------------------------------------------
// PasskeyConfig
// ---------------------------------------------------------------------------

/**
 * @typedef {object} PasskeyConfig
 * @property {string} provider - The PRF provider to use (file, yubikey, fido2)
 * @property {string|undefined} label - Optional label for seed derivation
 * @property {boolean} listLabels - Whether to list and select from labels on Nostr
 * @property {boolean} storeLabel - Whether to publish the label to Nostr
 * @property {string|undefined} rpid - Optional relying party ID for FIDO2 provider
 */

// ---------------------------------------------------------------------------
// File-based PRF provider
// ---------------------------------------------------------------------------

const SECRET_FILE_NAME = 'seedless-restore-secret'

/**
 * File-based PRF provider using HMAC-SHA256 with a secret stored in a file.
 * The secret is generated randomly on first use and persisted to disk.
 *
 * This is less secure than hardware-backed solutions like YubiKey, but
 * suitable for development/testing or when hardware keys are unavailable.
 */
class FilePrfProvider {
  /**
   * @param {Buffer} secret - The 32-byte secret
   */
  constructor(secret) {
    this.secret = secret
  }

  /**
   * Create a new FilePrfProvider using a secret from the specified data directory.
   * If the secret file doesn't exist, a random 32-byte secret is generated and saved.
   *
   * @param {string} dataDir - The data directory where the secret file is stored
   * @returns {FilePrfProvider}
   */
  static create(dataDir) {
    const secretPath = path.join(dataDir, SECRET_FILE_NAME)

    try {
      const bytes = fs.readFileSync(secretPath)
      if (bytes.length !== 32) {
        throw new Error(`Invalid secret file: expected 32 bytes, got ${bytes.length}`)
      }
      return new FilePrfProvider(bytes)
    } catch (err) {
      if (err.code !== 'ENOENT') {
        throw new Error(`Failed to read secret file: ${err.message}`)
      }
    }

    // Generate new random secret
    const secret = crypto.randomBytes(32)

    // Ensure data directory exists
    fs.mkdirSync(dataDir, { recursive: true })

    // Save secret to file
    fs.writeFileSync(secretPath, secret, { mode: 0o600 })

    return new FilePrfProvider(secret)
  }

  /**
   * Derive a PRF seed using HMAC-SHA256(secret, salt).
   *
   * @param {string} salt - The salt string
   * @returns {Promise<Uint8Array>} 32-byte PRF output
   */
  derivePrfSeed = async (salt) => {
    const hmac = crypto.createHmac('sha256', this.secret)
    hmac.update(salt)
    return new Uint8Array(hmac.digest())
  }

  /**
   * File-based PRF is always available once initialized.
   *
   * @returns {Promise<boolean>}
   */
  isPrfAvailable = async () => {
    return true
  }
}

// ---------------------------------------------------------------------------
// Stub providers for hardware-dependent backends
// ---------------------------------------------------------------------------

class NotYetSupportedProvider {
  /**
   * @param {string} name - The provider name
   */
  constructor(name) {
    this.name = name
  }

  derivePrfSeed = async (_salt) => {
    throw new Error(`${this.name} passkey provider is not yet supported in the Node.js CLI`)
  }

  isPrfAvailable = async () => {
    return false
  }
}

// ---------------------------------------------------------------------------
// Build PRF provider
// ---------------------------------------------------------------------------

/**
 * Create a PrfProvider for the given provider type.
 *
 * @param {string} provider - The provider name (file, yubikey, fido2)
 * @param {string} dataDir - The data directory
 * @returns {object} A PrfProvider implementation
 */
function buildPrfProvider(provider, dataDir, rpid) {
  switch (provider) {
    case PASSKEY_PROVIDER_FILE:
      return FilePrfProvider.create(dataDir)
    case PASSKEY_PROVIDER_YUBIKEY:
      return new NotYetSupportedProvider('YubiKey')
    case PASSKEY_PROVIDER_FIDO2:
      return new NotYetSupportedProvider('FIDO2')
    default:
      throw new Error(`Unknown passkey provider: ${provider}`)
  }
}

// ---------------------------------------------------------------------------
// Passkey seed resolution (orchestration)
// ---------------------------------------------------------------------------

/**
 * Prompt the user for input on stdin (one-shot, no readline interface required).
 *
 * @param {string} prompt - The prompt to display
 * @returns {Promise<string>}
 */
function promptStdin(prompt) {
  return new Promise((resolve) => {
    const rl = readline.createInterface({
      input: process.stdin,
      output: process.stdout,
      terminal: false
    })
    process.stdout.write(prompt)
    rl.once('line', (line) => {
      rl.close()
      resolve(line.trim())
    })
  })
}

/**
 * Derive a wallet seed using the given PRF provider, matching the Rust CLI's
 * resolve_passkey_seed logic.
 *
 * @param {object} provider - A PrfProvider implementation
 * @param {string|undefined} breezApiKey - Optional Breez API key
 * @param {string|undefined} label - Optional label for seed derivation
 * @param {boolean} listLabels - Whether to list and select from labels on Nostr
 * @param {boolean} storeLabel - Whether to publish the label to Nostr
 * @returns {Promise<object>} The seed object for use with SdkBuilder
 */
async function resolvePasskeySeed(
  provider,
  breezApiKey,
  label,
  listLabels,
  storeLabel
) {
  const relayConfig = {
    breezApiKey
  }
  const passkey = new Passkey(provider, relayConfig)

  // --store-label: publish to Nostr
  if (storeLabel && label) {
    console.log(`Publishing label '${label}' to Nostr...`)
    await passkey.storeLabel(label)
    console.log(`Label '${label}' published successfully.`)
  }

  // --list-labels: query Nostr and prompt user to select
  let resolvedName = label
  if (listLabels) {
    console.log('Querying Nostr for available labels...')
    const labels = await passkey.listLabels()

    if (labels.length === 0) {
      throw new Error('No labels found on Nostr for this identity')
    }

    console.log('Available labels:')
    for (let i = 0; i < labels.length; i++) {
      console.log(`  ${i + 1}: ${labels[i]}`)
    }

    const input = await promptStdin(`Select label (1-${labels.length}): `)
    const idx = parseInt(input, 10)
    if (isNaN(idx) || idx < 1 || idx > labels.length) {
      throw new Error('Invalid selection')
    }

    resolvedName = labels[idx - 1]
  }

  const wallet = await passkey.getWallet(resolvedName)
  return wallet.seed
}

module.exports = {
  PASSKEY_PROVIDER_FILE,
  PASSKEY_PROVIDER_YUBIKEY,
  PASSKEY_PROVIDER_FIDO2,
  parsePasskeyProvider,
  buildPrfProvider,
  resolvePasskeySeed
}
