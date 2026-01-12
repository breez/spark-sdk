import type { Seed, BreezSdk, PasskeyPrfProvider } from '@breeztech/breez-sdk-spark-react-native'
import {
  SeedlessRestore,
  defaultConfig,
  SdkBuilder,
  Network
} from '@breeztech/breez-sdk-spark-react-native'

// ANCHOR: implement-prf-provider
// In practice, implement PRF provider using platform passkey APIs
class ExamplePasskeyPrfProvider {
  derivePrfSeed = async (salt: string): Promise<ArrayBuffer> => {
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
  const seedless = new SeedlessRestore(prfProvider, undefined)

  // Create a new seed with user-chosen salt
  // The salt is published to Nostr for later discovery
  const seed = await seedless.createSeed('personal')

  // Use the seed to initialize the SDK
  const config = defaultConfig(Network.Mainnet)
  const builder = new SdkBuilder(config, seed)
  await builder.withDefaultStorage('./.data')
  const sdk = await builder.build()
  // ANCHOR_END: create-seed
  return seed
}

const exampleListSalts = async (): Promise<string[]> => {
  // ANCHOR: list-salts
  const prfProvider = new ExamplePasskeyPrfProvider()
  const seedless = new SeedlessRestore(prfProvider, undefined)

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
  const seedless = new SeedlessRestore(prfProvider, undefined)

  // Restore seed using a known salt
  const seed = await seedless.restoreSeed('personal')

  // Use the seed to initialize the SDK
  const config = defaultConfig(Network.Mainnet)
  const builder = new SdkBuilder(config, seed)
  await builder.withDefaultStorage('./.data')
  const sdk = await builder.build()
  // ANCHOR_END: restore-seed
  return seed
}
