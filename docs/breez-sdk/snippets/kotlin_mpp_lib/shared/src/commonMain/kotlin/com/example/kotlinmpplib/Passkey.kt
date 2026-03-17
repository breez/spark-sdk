package com.example.kotlinmpplib

import breez_sdk_spark.*

// ANCHOR: implement-prf-provider
// In practice, implement PRF provider using platform passkey APIs
class ExamplePasskeyPrfProvider : PasskeyPrfProvider {
    override suspend fun derivePrfSeed(salt: String): ByteArray {
        // Call platform passkey API with PRF extension
        // Returns 32-byte PRF output
        TODO("Implement using WebAuthn or native passkey APIs")
    }

    override suspend fun isPrfAvailable(): Boolean {
        // Check if PRF-capable passkey exists
        TODO("Check platform passkey availability")
    }
}
// ANCHOR_END: implement-prf-provider

class PasskeySnippets {
    suspend fun connectWithPasskey(): BreezSdk {
        // ANCHOR: connect-with-passkey
        val prfProvider = ExamplePasskeyPrfProvider()
        val passkey = Passkey(prfProvider, null)

        // Derive the wallet from the passkey (pass null for the default wallet)
        val wallet = passkey.getWallet("personal")

        val config = defaultConfig(Network.MAINNET)
        val sdk = connect(ConnectRequest(config, wallet.seed, "./.data"))
        // ANCHOR_END: connect-with-passkey
        return sdk
    }

    suspend fun listLabels(): List<String> {
        // ANCHOR: list-labels
        val prfProvider = ExamplePasskeyPrfProvider()
        val relayConfig = NostrRelayConfig(breezApiKey = "<breez api key>")
        val passkey = Passkey(prfProvider, relayConfig)

        // Query Nostr for labels associated with this passkey
        val labels = passkey.listLabels()

        for (label in labels) {
            // Log.v("Breez", "Found label: $label")
        }
        // ANCHOR_END: list-labels
        return labels
    }

    suspend fun storeLabel() {
        // ANCHOR: store-label
        val prfProvider = ExamplePasskeyPrfProvider()
        val relayConfig = NostrRelayConfig(breezApiKey = "<breez api key>")
        val passkey = Passkey(prfProvider, relayConfig)

        // Publish the label to Nostr for later discovery
        passkey.storeLabel("personal")
        // ANCHOR_END: store-label
    }
}
