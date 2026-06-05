/**
 * Passkey support for the Breez SDK React Native CLI.
 *
 * Provides:
 *   - PasskeyProvider enum (Platform, File, YubiKey, Fido2)
 *   - PasskeyConfig interface
 *   - File-based PRF provider using HMAC-SHA256
 *   - resolvePasskeySeed() async function matching the Rust CLI logic
 *
 * Mirrors crates/breez-sdk/cli/src/passkey/mod.rs and file_prf.rs,
 * following the same patterns as the Go CLI (passkey.go).
 */

import {
  type Seed as SeedType,
  PasskeyClient,
  type PrfProvider,
  type DeriveSeedsRequest,
  type DeriveSeedsOutput,
  type PasskeyCredential,
  DomainAssociation,
} from '@breeztech/breez-sdk-spark-react-native'
import { PasskeyProvider as PlatformPasskeyProvider } from '@breeztech/breez-sdk-spark-react-native/passkey-prf-provider'
import RNFS from 'react-native-fs'
import { generateRandomBytes, hmacSha256 } from './crypto_utils'

// ---------------------------------------------------------------------------
// Provider Enum
// ---------------------------------------------------------------------------

/** Passkey PRF provider type, matching the Rust CLI's PasskeyProvider enum. */
export enum PasskeyProvider {
  /** Platform-native passkey (iOS AuthenticationServices / Android CredentialManager). */
  Platform = 'platform',
  File = 'file',
  YubiKey = 'yubikey',
  Fido2 = 'fido2',
}

/**
 * Parse a provider name string into a PasskeyProvider.
 *
 * @param s - The provider name (case-insensitive)
 * @returns The parsed PasskeyProvider
 * @throws Error if the provider name is invalid
 */
export function parsePasskeyProvider(s: string): PasskeyProvider {
  switch (s.toLowerCase()) {
    case 'platform':
      return PasskeyProvider.Platform
    case 'file':
      return PasskeyProvider.File
    case 'yubikey':
      return PasskeyProvider.YubiKey
    case 'fido2':
      return PasskeyProvider.Fido2
    default:
      throw new Error(`Invalid passkey provider '${s}' (valid: platform, file, yubikey, fido2)`)
  }
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/** Configuration for passkey seed derivation, matching the Rust CLI's PasskeyConfig. */
export interface PasskeyConfig {
  /** The PRF provider to use. */
  provider: PasskeyProvider
  /** Optional label for seed derivation. If omitted, the core uses the default name. */
  label?: string
  /** Whether to list and select from labels published to Nostr. */
  listLabels: boolean
  /** Whether to publish the label to Nostr. */
  storeLabel: boolean
  /** Optional relying party ID for the Platform provider (default: keys.breez.technology). */
  rpid?: string
}

// ---------------------------------------------------------------------------
// File-based PRF Provider
// ---------------------------------------------------------------------------

/** File name for the seed restore secret, matching the Rust constant. */
const SECRET_FILE_NAME = 'seedless-restore-secret'

/** Convert a base64 string to a Uint8Array. */
function base64ToUint8Array(base64: string): Uint8Array {
  const chars = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/'
  const bytes: number[] = []
  let buffer = 0
  let bits = 0
  for (const ch of base64) {
    if (ch === '=') break
    const val = chars.indexOf(ch)
    if (val === -1) continue
    buffer = (buffer << 6) | val
    bits += 6
    if (bits >= 8) {
      bits -= 8
      bytes.push((buffer >> bits) & 0xff)
    }
  }
  return new Uint8Array(bytes)
}

/** Convert a Uint8Array to a base64 string. */
function uint8ArrayToBase64(bytes: Uint8Array): string {
  const chars = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/'
  let result = ''
  let buffer = 0
  let bits = 0
  for (const byte of bytes) {
    buffer = (buffer << 8) | byte
    bits += 8
    while (bits >= 6) {
      bits -= 6
      result += chars[(buffer >> bits) & 0x3f]
    }
  }
  if (bits > 0) {
    result += chars[(buffer << (6 - bits)) & 0x3f]
    while (result.length % 4 !== 0) {
      result += '='
    }
  }
  return result
}

/**
 * File-based implementation of a PrfProvider.
 *
 * Uses HMAC-SHA256 with a secret stored in a file. The secret is generated
 * randomly on first use and persisted to disk.
 *
 * Security note: This is less secure than hardware-backed solutions like
 * YubiKey. Suitable for development/testing or when hardware keys are unavailable.
 */
class FilePrfProvider implements PrfProvider {
  private secret: Uint8Array

  private constructor(secret: Uint8Array) {
    this.secret = secret
  }

  /**
   * Create a new FilePrfProvider using a secret from the specified data directory.
   * If the secret file does not exist, a random 32-byte secret is generated and saved.
   */
  static async create(dataDir: string): Promise<FilePrfProvider> {
    const secretPath = `${dataDir}/${SECRET_FILE_NAME}`

    const exists = await RNFS.exists(secretPath)
    if (exists) {
      const base64Data = await RNFS.readFile(secretPath, 'base64')
      const secretBytes = base64ToUint8Array(base64Data)
      if (secretBytes.length !== 32) {
        throw new Error(`Invalid secret file: expected 32 bytes, got ${secretBytes.length}`)
      }
      return new FilePrfProvider(secretBytes)
    }

    // Generate new random secret
    const secret = generateRandomBytes(32)

    // Ensure data directory exists
    const dirExists = await RNFS.exists(dataDir)
    if (!dirExists) {
      await RNFS.mkdir(dataDir)
    }

    // Save secret to file (as raw bytes in base64 encoding for RNFS)
    await RNFS.writeFile(secretPath, uint8ArrayToBase64(secret), 'base64')

    return new FilePrfProvider(secret)
  }

  /**
   * Derive one 32-byte PRF output per salt via HMAC-SHA256(secret, salt),
   * preserving input order. No credential ID is surfaced by this backend.
   */
  deriveSeeds = async (request: DeriveSeedsRequest): Promise<DeriveSeedsOutput> => {
    const seeds = request.salts.map((salt: string) => {
      const result = hmacSha256(this.secret, new TextEncoder().encode(salt))
      return (result.buffer as ArrayBuffer).slice(result.byteOffset, result.byteOffset + result.byteLength)
    })
    return { seeds, credentialId: undefined }
  }

  /** File-based PRF is always available once initialized. */
  isSupported = async (): Promise<boolean> => {
    return true
  }

  /** File-backed seeds derive lazily; there is no credential to register. */
  createPasskey = async (_excludeCredentials: ArrayBuffer[]): Promise<PasskeyCredential> => {
    throw new Error('file-backed PRF provider does not support credential creation; use sign-in by label instead')
  }

  /** No out-of-band verification source for a file secret. */
  checkDomainAssociation = async (): Promise<DomainAssociation> => {
    return new DomainAssociation.Skipped({ reason: 'FilePrfProvider does not verify domain association' })
  }
}

// ---------------------------------------------------------------------------
// Stub provider for unsupported hardware backends
// ---------------------------------------------------------------------------

/**
 * Stub provider that returns errors for hardware-dependent backends
 * not yet supported in the React Native CLI.
 */
class NotYetSupportedProvider implements PrfProvider {
  private name: string

  constructor(name: string) {
    this.name = name
  }

  private notYet(): Error {
    return new Error(`${this.name} passkey provider is not yet supported in the React Native CLI`)
  }

  deriveSeeds = async (_request: DeriveSeedsRequest): Promise<DeriveSeedsOutput> => {
    throw this.notYet()
  }

  isSupported = async (): Promise<boolean> => {
    return false
  }

  createPasskey = async (_excludeCredentials: ArrayBuffer[]): Promise<PasskeyCredential> => {
    throw this.notYet()
  }

  checkDomainAssociation = async (): Promise<DomainAssociation> => {
    return new DomainAssociation.Skipped({ reason: `${this.name} does not verify domain association` })
  }
}

// ---------------------------------------------------------------------------
// Provider Factory
// ---------------------------------------------------------------------------

/**
 * Build a PRF provider for the given provider type.
 *
 * @param provider - The provider type
 * @param dataDir - The data directory (used by File provider)
 * @param rpid - Relying party ID for the Platform provider (default: keys.breez.technology)
 * @returns The PRF provider instance
 */
export async function buildPrfProvider(
  provider: PasskeyProvider,
  dataDir: string,
  rpid?: string,
): Promise<PrfProvider> {
  switch (provider) {
    case PasskeyProvider.Platform:
      // The built-in provider's TS types are Uint8Array-based; the SDK's
      // PrfProvider interface is ArrayBuffer-based. The cast bridges that
      // surface difference, same as the RN passkey snippet.
      return new PlatformPasskeyProvider({
        rpId: rpid ?? PlatformPasskeyProvider.BREEZ_RP_ID,
        rpName: 'Breez SDK CLI',
      }) as unknown as PrfProvider
    case PasskeyProvider.File:
      return FilePrfProvider.create(dataDir)
    case PasskeyProvider.YubiKey:
      return new NotYetSupportedProvider('YubiKey')
    case PasskeyProvider.Fido2:
      return new NotYetSupportedProvider('FIDO2')
    default:
      throw new Error(`Unknown passkey provider: ${provider}`)
  }
}

// ---------------------------------------------------------------------------
// Passkey Seed Resolution
// ---------------------------------------------------------------------------

/**
 * Resolve a wallet seed using the given PRF provider via PasskeyClient. The
 * client handles PRF -> 12-word BIP39 mnemonic derivation and Nostr label
 * storage / discovery, matching the Go, Python, and Rust CLIs.
 *
 * Uses connectWithPasskey (silent sign-in, falling through to registration on
 * the Platform provider when no credential exists): the file / YubiKey / FIDO2
 * backends always resolve via the sign-in path.
 *
 * @param provider - The PRF provider to use
 * @param breezApiKey - Optional Breez API key (enables NIP-42 auth on the Breez relay)
 * @param label - Optional label for seed derivation
 * @param listLabels - Whether to query Nostr for labels published by this passkey
 * @param storeLabel - Whether to publish the label to Nostr
 * @returns Object with { seed, labels? } - seed is a 12-word mnemonic Seed,
 *          labels is populated when listLabels is true
 */
export async function resolvePasskeySeed(
  provider: PrfProvider,
  breezApiKey: string | undefined,
  label: string | undefined,
  listLabels: boolean,
  storeLabel: boolean,
): Promise<{ seed: SeedType; labels?: string[] }> {
  const passkey = new PasskeyClient(provider, breezApiKey, undefined)

  // --list-labels: discovery sign-in (no cached label) returns the published
  // label set. App.tsx does not prompt for selection; default to the explicit
  // label if provided, otherwise the first discovered label.
  let returnedLabels: string[] | undefined
  let resolvedLabel = label
  if (listLabels) {
    const { labels } = await passkey.signIn({ label: undefined })
    returnedLabels = labels
    resolvedLabel = label ?? labels[0]
  }

  // --store-label: publish to Nostr
  if (storeLabel && resolvedLabel) {
    await passkey.labels().store(resolvedLabel)
  }

  const response = await passkey.connectWithPasskey({
    label: resolvedLabel,
    allowCredentials: [],
    excludeCredentials: [],
  })
  return { seed: response.wallet.seed, labels: returnedLabels }
}
