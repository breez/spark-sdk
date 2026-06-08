package com.example.kotlinmpplib

import breez_sdk_spark.*

class ExternalSigner {
    // ANCHOR: default-external-signer
    fun createSigner(): breez_sdk_spark.ExternalBreezSigner {
        val mnemonic = "<mnemonic words>"
        val network = Network.MAINNET
        val accountNumber = 0U

        val keySetConfig = KeySetConfig(
            accountNumber = accountNumber
        )
        
        val signer = defaultExternalSigner(
            mnemonic = mnemonic,
            passphrase = null,
            network = network,
            keySetConfig = keySetConfig
        )
        
        return signer
    }
    // ANCHOR_END: default-external-signer
    
    // ANCHOR: connect-with-signer
    suspend fun connectWithSigner(signer: breez_sdk_spark.ExternalBreezSigner, sparkSigner: breez_sdk_spark.ExternalSparkSigner) {
        // Create the config
        val config = defaultConfig(Network.MAINNET)
        config.apiKey = "<breez api key>"

        try {
            // Connect using the external signers
            val sdk = connectWithSigner(ConnectWithSignerRequest(
                config = config,
                signer = signer,
                sparkSigner = sparkSigner,
                storageDir = "./.data"
            ))
        } catch (e: Exception) {
            // handle error
        }
    }
    // ANCHOR_END: connect-with-signer
}
