/**
 * Passkey support for the Breez SDK React Native CLI.
 *
 * Provides:
 *   - PasskeyProvider enum (File, YubiKey, Fido2)
 *   - PasskeyConfig interface
 *   - File-based PRF provider using HMAC-SHA256
 *   - resolvePasskeySeed() async function matching the Rust CLI logic
 *
 * Mirrors crates/breez-sdk/cli/src/passkey/mod.rs and file_prf.rs,
 * following the same patterns as the Go CLI (passkey.go).
 */

import {
  Seed,
  type Seed as SeedType,
} from '@breeztech/breez-sdk-spark-react-native'
import RNFS from 'react-native-fs'
import { generateRandomBytes, hmacSha256 } from './crypto_utils'

// ---------------------------------------------------------------------------
// Provider Enum
// ---------------------------------------------------------------------------

/** Passkey PRF provider type, matching the Rust CLI's PasskeyProvider enum. */
export enum PasskeyProvider {
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
    case 'file':
      return PasskeyProvider.File
    case 'yubikey':
      return PasskeyProvider.YubiKey
    case 'fido2':
      return PasskeyProvider.Fido2
    default:
      throw new Error(`Invalid passkey provider '${s}' (valid: file, yubikey, fido2)`)
  }
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/** Configuration for passkey seed derivation, matching the Rust CLI's PasskeyConfig. */
export interface PasskeyConfig {
  /** The PRF provider to use. */
  provider: PasskeyProvider
  /** Optional wallet name for seed derivation. If omitted, the core uses the default name. */
  walletName?: string
  /** Whether to list and select from wallet names published to Nostr. */
  listWalletNames: boolean
  /** Whether to publish the wallet name to Nostr. */
  storeWalletName: boolean
  /** Optional relying party ID for FIDO2 provider (default: keys.breez.technology). */
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
 * File-based implementation of a PasskeyPrfProvider.
 *
 * Uses HMAC-SHA256 with a secret stored in a file. The secret is generated
 * randomly on first use and persisted to disk.
 *
 * Security note: This is less secure than hardware-backed solutions like
 * YubiKey. Suitable for development/testing or when hardware keys are unavailable.
 */
class FilePrfProvider {
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
   * Derive a PRF seed from a salt using HMAC-SHA256(secret, salt).
   */
  derivePrfSeed = async (salt: string): Promise<ArrayBuffer> => {
    const result = hmacSha256(this.secret, new TextEncoder().encode(salt))
    return (result.buffer as ArrayBuffer).slice(result.byteOffset, result.byteOffset + result.byteLength)
  }

  /**
   * File-based PRF is always available once initialized.
   */
  isPrfAvailable = async (): Promise<boolean> => {
    return true
  }
}

// ---------------------------------------------------------------------------
// Stub provider for unsupported hardware backends
// ---------------------------------------------------------------------------

/**
 * Stub provider that returns errors for hardware-dependent backends
 * not yet supported in the React Native CLI.
 */
class NotYetSupportedProvider {
  private name: string

  constructor(name: string) {
    this.name = name
  }

  derivePrfSeed = async (_salt: string): Promise<ArrayBuffer> => {
    throw new Error(`${this.name} passkey provider is not yet supported in the React Native CLI`)
  }

  isPrfAvailable = async (): Promise<boolean> => {
    return false
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
 * @returns The PRF provider instance
 */
export async function buildPrfProvider(
  provider: PasskeyProvider,
  dataDir: string,
): Promise<{ derivePrfSeed: (salt: string) => Promise<ArrayBuffer>; isPrfAvailable: () => Promise<boolean> }> {
  switch (provider) {
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
 * Resolve a wallet seed using the given PRF provider.
 *
 * Note: Full passkey functionality (Nostr wallet name listing/storing) is not
 * yet supported in the React Native SDK. This stub derives a seed from the
 * PRF provider and returns it as a Seed.Entropy variant.
 *
 * @param provider - The PRF provider to use
 * @param _breezApiKey - Optional Breez API key (unused - Nostr not yet supported)
 * @param walletName - Optional wallet name for seed derivation
 * @param _listWalletNames - Whether to list wallet names (not yet supported)
 * @param _storeWalletName - Whether to publish the wallet name (not yet supported)
 * @returns Object with { seed, walletNames? } - seed is the derived Seed,
 *          walletNames is populated when listWalletNames is true
 */
export async function resolvePasskeySeed(
  provider: { derivePrfSeed: (salt: string) => Promise<ArrayBuffer>; isPrfAvailable: () => Promise<boolean> },
  _breezApiKey: string | undefined,
  walletName: string | undefined,
  _listWalletNames: boolean,
  _storeWalletName: boolean,
): Promise<{ seed: SeedType; walletNames?: string[] }> {
  // Derive seed bytes from the PRF provider
  const seedBytes = await provider.derivePrfSeed(walletName ?? 'Default')

  // Note: Passkey wallet name listing/storing via Nostr is not yet supported
  // in React Native. Only basic seed derivation is available.

  // Use Entropy variant since we have raw bytes
  const seed = new Seed.Entropy(seedBytes)
  return { seed }
}
