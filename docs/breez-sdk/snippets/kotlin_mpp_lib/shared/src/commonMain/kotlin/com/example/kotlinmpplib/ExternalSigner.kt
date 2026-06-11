package com.example.kotlinmpplib

import breez_sdk_spark.*

class ExternalSigner {
    // ANCHOR: default-external-signer
    fun createSigners(): breez_sdk_spark.ExternalSigners {
        val mnemonic = "<mnemonic words>"
        val network = Network.MAINNET
        val accountNumber = 0U

        val keySetConfig = KeySetConfig(
            accountNumber = accountNumber
        )
        
        val signers = defaultExternalSigners(
            mnemonic = mnemonic,
            passphrase = null,
            network = network,
            keySetConfig = keySetConfig
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
}
