import BreezSdkSpark
import Foundation

// ANCHOR: implement-prf-provider
// In practice, implement PRF provider using platform passkey APIs
class ExamplePasskeyPrfProvider: PasskeyPrfProvider {
    func derivePrfSeed(salt: String) async throws -> Data {
        // Call platform passkey API with PRF extension
        // Returns 32-byte PRF output
        fatalError("Implement using platform passkey APIs")
    }

    func isPrfAvailable() async throws -> Bool {
        // Check if PRF-capable passkey exists
        fatalError("Check platform passkey availability")
    }
}
// ANCHOR_END: implement-prf-provider

func createSeed() async throws -> Seed {
    // ANCHOR: create-seed
    let prfProvider = ExamplePasskeyPrfProvider()
    let seedless = SeedlessRestore(prfProvider: prfProvider, relayConfig: nil)

    // Create a new seed with user-chosen salt
    // The salt is published to Nostr for later discovery
    let seed = try await seedless.createSeed(salt: "personal")

    // Use the seed to initialize the SDK
    let config = defaultConfig(network: .mainnet)
    let builder = SdkBuilder(config: config, seed: seed)
    await builder.withDefaultStorage(storageDir: "./.data")
    let sdk = try await builder.build()
    // ANCHOR_END: create-seed
    return seed
}

func listSalts() async throws -> [String] {
    // ANCHOR: list-salts
    let prfProvider = ExamplePasskeyPrfProvider()
    let seedless = SeedlessRestore(prfProvider: prfProvider, relayConfig: nil)

    // Query Nostr for salts associated with this passkey
    let salts = try await seedless.listSalts()

    for salt in salts {
        print("Found wallet: \(salt)")
    }
    // ANCHOR_END: list-salts
    return salts
}

func restoreSeed() async throws -> Seed {
    // ANCHOR: restore-seed
    let prfProvider = ExamplePasskeyPrfProvider()
    let seedless = SeedlessRestore(prfProvider: prfProvider, relayConfig: nil)

    // Restore seed using a known salt
    let seed = try await seedless.restoreSeed(salt: "personal")

    // Use the seed to initialize the SDK
    let config = defaultConfig(network: .mainnet)
    let builder = SdkBuilder(config: config, seed: seed)
    await builder.withDefaultStorage(storageDir: "./.data")
    let sdk = try await builder.build()
    // ANCHOR_END: restore-seed
    return seed
}
