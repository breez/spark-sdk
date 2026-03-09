/**
 * Cryptographic utilities for the React Native CLI.
 *
 * Provides pure-JavaScript implementations of SHA-256, HMAC-SHA256, and
 * random byte generation that work in React Native without Node.js polyfills.
 *
 * Note: In a production app, consider using react-native-get-random-values
 * for cryptographically secure random number generation. The fallback here
 * uses Math.random() which is NOT cryptographically secure but is acceptable
 * for this CLI demo's file-based PRF provider and HODL preimage generation.
 */

// ---------------------------------------------------------------------------
// SHA-256 (pure JS implementation)
// ---------------------------------------------------------------------------

// SHA-256 constants: first 32 bits of the fractional parts of the cube roots
// of the first 64 primes.
const K: number[] = [
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5,
  0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
  0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
  0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
  0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc,
  0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
  0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
  0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
  0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
  0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3,
  0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5,
  0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
  0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
  0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
]

function rightRotate(value: number, amount: number): number {
  return (value >>> amount) | (value << (32 - amount))
}

/**
 * Compute SHA-256 hash of the input bytes.
 * Returns a 32-byte Uint8Array.
 */
export function sha256Hash(message: Uint8Array): Uint8Array {
  // Pre-processing: adding padding bits
  const msgLen = message.length
  const bitLen = msgLen * 8

  // Message needs to be padded to 512-bit (64-byte) blocks
  // Padding: 1 bit, then zeros, then 64-bit big-endian length
  const paddedLen = Math.ceil((msgLen + 9) / 64) * 64
  const padded = new Uint8Array(paddedLen)
  padded.set(message)
  padded[msgLen] = 0x80

  // Append length in bits as 64-bit big-endian
  const view = new DataView(padded.buffer)
  // We only support messages up to 2^32 bits (512 MB), so high 32 bits are 0
  view.setUint32(paddedLen - 4, bitLen, false)

  // Initialize hash values
  let h0 = 0x6a09e667
  let h1 = 0xbb67ae85
  let h2 = 0x3c6ef372
  let h3 = 0xa54ff53a
  let h4 = 0x510e527f
  let h5 = 0x9b05688c
  let h6 = 0x1f83d9ab
  let h7 = 0x5be0cd19

  // Process each 512-bit block
  const w = new Int32Array(64)

  for (let offset = 0; offset < paddedLen; offset += 64) {
    // Prepare message schedule
    for (let i = 0; i < 16; i++) {
      w[i] = view.getInt32(offset + i * 4, false)
    }
    for (let i = 16; i < 64; i++) {
      const s0 = rightRotate(w[i - 15], 7) ^ rightRotate(w[i - 15], 18) ^ (w[i - 15] >>> 3)
      const s1 = rightRotate(w[i - 2], 17) ^ rightRotate(w[i - 2], 19) ^ (w[i - 2] >>> 10)
      w[i] = (w[i - 16] + s0 + w[i - 7] + s1) | 0
    }

    // Initialize working variables
    let a = h0, b = h1, c = h2, d = h3
    let e = h4, f = h5, g = h6, h = h7

    // Compression function
    for (let i = 0; i < 64; i++) {
      const S1 = rightRotate(e, 6) ^ rightRotate(e, 11) ^ rightRotate(e, 25)
      const ch = (e & f) ^ (~e & g)
      const temp1 = (h + S1 + ch + K[i] + w[i]) | 0
      const S0 = rightRotate(a, 2) ^ rightRotate(a, 13) ^ rightRotate(a, 22)
      const maj = (a & b) ^ (a & c) ^ (b & c)
      const temp2 = (S0 + maj) | 0

      h = g
      g = f
      f = e
      e = (d + temp1) | 0
      d = c
      c = b
      b = a
      a = (temp1 + temp2) | 0
    }

    h0 = (h0 + a) | 0
    h1 = (h1 + b) | 0
    h2 = (h2 + c) | 0
    h3 = (h3 + d) | 0
    h4 = (h4 + e) | 0
    h5 = (h5 + f) | 0
    h6 = (h6 + g) | 0
    h7 = (h7 + h) | 0
  }

  // Produce the final hash value (big-endian)
  const result = new Uint8Array(32)
  const resultView = new DataView(result.buffer)
  resultView.setUint32(0, h0, false)
  resultView.setUint32(4, h1, false)
  resultView.setUint32(8, h2, false)
  resultView.setUint32(12, h3, false)
  resultView.setUint32(16, h4, false)
  resultView.setUint32(20, h5, false)
  resultView.setUint32(24, h6, false)
  resultView.setUint32(28, h7, false)

  return result
}

// ---------------------------------------------------------------------------
// HMAC-SHA256
// ---------------------------------------------------------------------------

/**
 * Compute HMAC-SHA256(key, message).
 * Returns a 32-byte Uint8Array.
 */
export function hmacSha256(key: Uint8Array, message: Uint8Array): Uint8Array {
  const blockSize = 64 // SHA-256 block size in bytes

  // If key is longer than block size, hash it first
  let keyBlock: Uint8Array
  if (key.length > blockSize) {
    keyBlock = sha256Hash(key)
  } else {
    keyBlock = key
  }

  // Pad key to block size
  const paddedKey = new Uint8Array(blockSize)
  paddedKey.set(keyBlock)

  // Create inner and outer pads
  const ipad = new Uint8Array(blockSize)
  const opad = new Uint8Array(blockSize)
  for (let i = 0; i < blockSize; i++) {
    ipad[i] = paddedKey[i] ^ 0x36
    opad[i] = paddedKey[i] ^ 0x5c
  }

  // Inner hash: SHA-256(ipad || message)
  const inner = new Uint8Array(blockSize + message.length)
  inner.set(ipad)
  inner.set(message, blockSize)
  const innerHash = sha256Hash(inner)

  // Outer hash: SHA-256(opad || inner_hash)
  const outer = new Uint8Array(blockSize + 32)
  outer.set(opad)
  outer.set(innerHash, blockSize)

  return sha256Hash(outer)
}

// ---------------------------------------------------------------------------
// Random Bytes
// ---------------------------------------------------------------------------

/**
 * Generate cryptographically secure random bytes.
 *
 * Requires 'react-native-get-random-values' to be imported at app entry
 * (polyfills global crypto.getRandomValues).
 */
export function generateRandomBytes(length: number): Uint8Array {
  const bytes = new Uint8Array(length)
  // eslint-disable-next-line no-undef
  crypto.getRandomValues(bytes)
  return bytes
}

// ---------------------------------------------------------------------------
// Hex encoding
// ---------------------------------------------------------------------------

/**
 * Convert a Uint8Array to a hex string.
 */
export function bytesToHex(bytes: Uint8Array): string {
  return Array.from(bytes)
    .map(b => b.toString(16).padStart(2, '0'))
    .join('')
}
