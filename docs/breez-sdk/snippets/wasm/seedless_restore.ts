import type { Seed } from '@breeztech/breez-sdk-spark'
import { SeedlessRestore, defaultConfig, SdkBuilder } from '@breeztech/breez-sdk-spark'

// ANCHOR: implement-prf-provider
// In practice, implement PRF provider using WebAuthn API
class ExamplePasskeyPrfProvider {
  derivePrfSeed = async (salt: string): Promise<Uint8Array> => {
    // Call native passkey module with PRF extension
    // Returns 32-byte PRF output
    throw new Error('Implement using native passkey module')
  }

  isPrfAvailable = async (): Promise<boolean> => {
    // Check if PRF-capable passkey exists via native module
    throw new Error('Check platform passkey availability')
  }
}
// ANCHOR_END: implement-prf-provider

const exampleCreateSeed = async (): Promise<Seed> => {
  // ANCHOR: create-seed
  const prfProvider = new ExamplePasskeyPrfProvider()
  const seedless = new SeedlessRestore(prfProvider)

  // Create a new seed with user-chosen salt
  // The salt is published to Nostr for later discovery
  const seed = await seedless.createSeed('personal')

  // Use the seed to initialize the SDK
  const config = defaultConfig('mainnet')
  let builder = SdkBuilder.new(config, seed)
  builder = await builder.withDefaultStorage('./.data')
  const sdk = await builder.build()
  // ANCHOR_END: create-seed
  return seed
}

const exampleListSalts = async (): Promise<string[]> => {
  // ANCHOR: list-salts
  const prfProvider = new ExamplePasskeyPrfProvider()
  const seedless = new SeedlessRestore(prfProvider)

  // Query Nostr for salts associated with this passkey
  const salts = await seedless.listSalts()

  for (const salt of salts) {
    console.log(`Found wallet: ${salt}`)
  }
  // ANCHOR_END: list-salts
  return salts
}

const exampleRestoreSeed = async (): Promise<Seed> => {
  // ANCHOR: restore-seed
  const prfProvider = new ExamplePasskeyPrfProvider()
  const seedless = new SeedlessRestore(prfProvider)

  // Restore seed using a known salt
  const seed = await seedless.restoreSeed('personal')

  // Use the seed to initialize the SDK
  const config = defaultConfig('mainnet')
  let builder = SdkBuilder.new(config, seed)
  builder = await builder.withDefaultStorage('./.data')
  const sdk = await builder.build()
  // ANCHOR_END: restore-seed
  return seed
}
