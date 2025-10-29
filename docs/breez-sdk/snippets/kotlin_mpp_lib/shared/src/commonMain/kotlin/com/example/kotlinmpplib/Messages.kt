package com.example.kotlinmpplib

import breez_sdk_spark.*

class Messages {
    suspend fun signMessage(sdk: BreezSdk) {
        // ANCHOR: sign-message
        val message = "<message to sign>"
        // Set to true to get a compact signature rather than a DER
        val compact = true
        try {
            val signMessageRequest = SignMessageRequest(message, compact)
            val signMessageResponse = sdk.signMessage(signMessageRequest)

            val signature = signMessageResponse?.signature
            val pubkey = signMessageResponse?.pubkey

            // Log.v("Breez", "Pubkey: ${pubkey}")
            // Log.v("Breez", "Signature: ${signature}")
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: sign-message
    }

    suspend fun checkMessage(sdk: BreezSdk) {
        // ANCHOR: check-message
        val message = "<message>"
        val pubkey = "<pubkey of signer>"
        val signature = "<message signature>"
        try {
            val checkMessageRequest = CheckMessageRequest(message, pubkey, signature)
            val checkMessageResponse = sdk.checkMessage(checkMessageRequest)

            val isValid = checkMessageResponse?.isValid

            // Log.v("Breez", "Signature valid: ${isValid}")
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: check-message
    }

}
