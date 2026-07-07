package com.example.kotlinmpplib

import breez_sdk_spark.*

class ExternalSigner {
    // ANCHOR: default-external-signer
    fun createSigners(): breez_sdk_spark.ExternalSigners {
        val mnemonic = "<mnemonic words>"
        val network = Network.MAINNET
        val accountNumber = 0U

        val signers = defaultExternalSigners(
            mnemonic = mnemonic,
            passphrase = null,
            network = network,
            accountNumber = accountNumber
        )
        
        return signers
    }
    // ANCHOR_END: default-external-signer
    
    // ANCHOR: connect-with-signer
    suspend fun connectWithSigner(signers: breez_sdk_spark.ExternalSigners) {
        // Create the config
        val config = defaultConfig(Network.MAINNET)
        config.apiKey = "<breez api key>"

        try {
            // Connect using the external signers
            val sdk = connectWithSigner(ConnectWithSignerRequest(
                config = config,
                breezSigner = signers.breezSigner,
                sparkSigner = signers.sparkSigner,
                storageDir = "./.data"
            ))
        } catch (e: Exception) {
            // handle error
        }
    }
    // ANCHOR_END: connect-with-signer

    // ANCHOR: sdk-builder-with-signer
    suspend fun buildWithSigner(signers: breez_sdk_spark.ExternalSigners) {
        // Create the config
        val config = defaultConfig(Network.MAINNET)
        config.apiKey = "<breez api key>"

        try {
            val builder = SdkBuilder.newWithSigner(
                config,
                signers.breezSigner,
                signers.sparkSigner
            )
            // builder.withStorageBackend(<your storage backend>)
            // builder.withSharedContext(<your shared context>)
            val sdk = builder.build()
        } catch (e: Exception) {
            // handle error
        }
    }
    // ANCHOR_END: sdk-builder-with-signer

    // ANCHOR: sdk-builder-with-signing-only-signer
    suspend fun buildWithSigningOnlySigner(
        config: breez_sdk_spark.Config,
        signers: breez_sdk_spark.SigningOnlyExternalSigners
    ) {
        try {
            val builder = SdkBuilder.newWithSigningOnlySigner(
                config,
                signers.breezSigner,
                signers.sparkSigner
            )
            val sdk = builder.build()
        } catch (e: Exception) {
            // handle error
        }
    }
    // ANCHOR_END: sdk-builder-with-signing-only-signer
}
