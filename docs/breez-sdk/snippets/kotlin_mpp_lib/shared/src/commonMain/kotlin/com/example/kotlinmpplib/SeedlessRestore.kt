package com.example.kotlinmpplib

import breez_sdk_spark.*

// ANCHOR: implement-prf-provider
// In practice, implement PRF provider using platform passkey APIs
class ExamplePasskeyPrfProvider : PasskeyPrfProvider {
    override suspend fun derivePrfSeed(salt: String): ByteArray {
        // Call platform passkey API with PRF extension
        // Returns 32-byte PRF output
        TODO("Implement using platform passkey APIs")
    }

    override suspend fun isPrfAvailable(): Boolean {
        // Check if PRF-capable passkey exists
        TODO("Check platform passkey availability")
    }
}
// ANCHOR_END: implement-prf-provider

class SeedlessRestoreSnippets {
    suspend fun createSeed(): Seed {
        // ANCHOR: create-seed
        val prfProvider = ExamplePasskeyPrfProvider()
        val seedless = SeedlessRestore(prfProvider, null)

        // Create a new seed with user-chosen salt
        // The salt is published to Nostr for later discovery
        val seed = seedless.createSeed("personal")

        // Use the seed to initialize the SDK
        val config = defaultConfig(Network.MAINNET)
        val builder = SdkBuilder(config, seed)
        builder.withDefaultStorage("./.data")
        val sdk = builder.build()
        // ANCHOR_END: create-seed
        return seed
    }

    suspend fun listSalts(): List<String> {
        // ANCHOR: list-salts
        val prfProvider = ExamplePasskeyPrfProvider()
        val seedless = SeedlessRestore(prfProvider, null)

        // Query Nostr for salts associated with this passkey
        val salts = seedless.listSalts()

        for (salt in salts) {
            // Log.v("Breez", "Found wallet: $salt")
            println("Found wallet: $salt")
        }
        // ANCHOR_END: list-salts
        return salts
    }

    suspend fun restoreSeed(): Seed {
        // ANCHOR: restore-seed
        val prfProvider = ExamplePasskeyPrfProvider()
        val seedless = SeedlessRestore(prfProvider, null)

        // Restore seed using a known salt
        val seed = seedless.restoreSeed("personal")

        // Use the seed to initialize the SDK
        val config = defaultConfig(Network.MAINNET)
        val builder = SdkBuilder(config, seed)
        builder.withDefaultStorage("./.data")
        val sdk = builder.build()
        // ANCHOR_END: restore-seed
        return seed
    }
}
