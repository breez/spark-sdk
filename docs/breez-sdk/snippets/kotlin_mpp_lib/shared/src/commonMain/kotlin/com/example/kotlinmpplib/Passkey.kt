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

    suspend fun listWalletNames(): List<String> {
        // ANCHOR: list-wallet-names
        val prfProvider = ExamplePasskeyPrfProvider()
        val relayConfig = NostrRelayConfig(breezApiKey = "<breez api key>")
        val passkey = Passkey(prfProvider, relayConfig)

        // Query Nostr for wallet names associated with this passkey
        val walletNames = passkey.listWalletNames()

        for (walletName in walletNames) {
            // Log.v("Breez", "Found wallet: $walletName")
        }
        // ANCHOR_END: list-wallet-names
        return walletNames
    }

    suspend fun storeWalletName() {
        // ANCHOR: store-wallet-name
        val prfProvider = ExamplePasskeyPrfProvider()
        val relayConfig = NostrRelayConfig(breezApiKey = "<breez api key>")
        val passkey = Passkey(prfProvider, relayConfig)

        // Publish the wallet name to Nostr for later discovery
        passkey.storeWalletName("personal")
        // ANCHOR_END: store-wallet-name
    }
}
