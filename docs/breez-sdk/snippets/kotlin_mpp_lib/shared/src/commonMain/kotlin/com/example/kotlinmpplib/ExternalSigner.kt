package com.example.kotlinmpplib

import breez_sdk_spark.*

class ExternalSigner {
    // ANCHOR: default-external-signer
    fun createSigner(): breez_sdk_spark.ExternalSigner {
        val mnemonic = "<mnemonic words>"
        val network = Network.MAINNET
        val keySetType = KeySetType.DEFAULT
        val useAddressIndex = false
        val accountNumber = 0U
        
        val keySetConfig = KeySetConfig(
            keySetType = keySetType,
            useAddressIndex = useAddressIndex,
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
    suspend fun connectWithSigner(signer: breez_sdk_spark.ExternalSigner) {
        // Create the config
        val config = defaultConfig(Network.MAINNET)
        config.apiKey = "<breez api key>"
        
        try {
            // Connect using the external signer
            val sdk = connectWithSigner(ConnectWithSignerRequest(
                config = config,
                signer = signer,
                storageDir = "./.data"
            ))
        } catch (e: Exception) {
            // handle error
        }
    }
    // ANCHOR_END: connect-with-signer
}
