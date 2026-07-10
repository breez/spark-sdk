package com.example.kotlinmpplib

import breez_sdk_spark.*

class Turnkey {
    suspend fun connectWithTurnkey() {
        // ANCHOR: turnkey-connect
        val turnkeyConfig = TurnkeyConfig(
            baseUrl = null,
            organizationId = "<turnkey sub-organization id>",
            apiPublicKey = "<api public key hex>",
            apiPrivateKey = "<api private key hex>",
            walletId = "<turnkey wallet id>",
            network = Network.MAINNET,
            accountNumber = null,
            // Set after the first connect to make later signer setup network-free
            identityPublicKey = null,
            retry = null,
            maxRps = null
        )

        try {
            val signers = createTurnkeySigner(turnkeyConfig)

            val config = defaultConfig(Network.MAINNET)
            config.apiKey = "<breez api key>"

            val sdk = connectWithSigner(ConnectWithSignerRequest(
                config = config,
                breezSigner = signers.breezSigner,
                sparkSigner = signers.sparkSigner,
                storageDir = "./.data"
            ))
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: turnkey-connect
    }
}
