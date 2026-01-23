package com.example.kotlinmpplib

import breez_sdk_spark.*

class LightningAddress {
    fun configureLightningAddress(): breez_sdk_spark.Config {
        // ANCHOR: config-lightning-address
        val config = defaultConfig(Network.MAINNET)
        config.apiKey = "your-api-key"
        config.lnurlDomain = "yourdomain.com"
        // ANCHOR_END: config-lightning-address
        return config
    }

    suspend fun checkLightningAddressAvailability(sdk: BreezSdk) {
        val username = "myusername"
        
        // ANCHOR: check-lightning-address
        val request = CheckLightningAddressRequest(
            username = username
        )
        
        val available = sdk.checkLightningAddressAvailable(request)
        // ANCHOR_END: check-lightning-address
    }

    suspend fun registerLightningAddress(sdk: BreezSdk) {
        val username = "myusername"
        val description = "My Lightning Address"
        
        // ANCHOR: register-lightning-address
        val request = RegisterLightningAddressRequest(
            username = username,
            description = description
        )
        
        val addressInfo = sdk.registerLightningAddress(request)
        val lightningAddress = addressInfo.lightningAddress
        val lnurlUrl = addressInfo.lnurl.url
        val lnurlBech32 = addressInfo.lnurl.bech32
        // ANCHOR_END: register-lightning-address
    }

    suspend fun getLightningAddress(sdk: BreezSdk) {
        // ANCHOR: get-lightning-address
        val addressInfoOpt = sdk.getLightningAddress()
        
        if (addressInfoOpt != null) {
            val lightningAddress = addressInfoOpt.lightningAddress
            val username = addressInfoOpt.username
            val description = addressInfoOpt.description
            val lnurlUrl = addressInfoOpt.lnurl.url
            val lnurlBech32 = addressInfoOpt.lnurl.bech32
        }
        // ANCHOR_END: get-lightning-address
    }

    suspend fun deleteLightningAddress(sdk: BreezSdk) {
        // ANCHOR: delete-lightning-address
        sdk.deleteLightningAddress()
        // ANCHOR_END: delete-lightning-address
    }

    suspend fun accessSenderComment(sdk: BreezSdk) {
        val paymentId = "<payment id>"
        val response = sdk.getPayment(GetPaymentRequest(paymentId = paymentId))
        val payment = response.payment
        
        // ANCHOR: access-sender-comment
        // Check if this is a lightning payment with LNURL receive metadata
        if (payment.details is PaymentDetails.Lightning) {
            val details = payment.details as PaymentDetails.Lightning
            val metadata = details.lnurlReceiveMetadata
            
            // Access the sender comment if present
            metadata?.senderComment?.let { comment ->
                println("Sender comment: $comment")
            }
        }
        // ANCHOR_END: access-sender-comment
    }

    suspend fun accessNostrZap(sdk: BreezSdk) {
        val paymentId = "<payment id>"
        val response = sdk.getPayment(GetPaymentRequest(paymentId = paymentId))
        val payment = response.payment
        
        // ANCHOR: access-nostr-zap
        // Check if this is a lightning payment with LNURL receive metadata
        if (payment.details is PaymentDetails.Lightning) {
            val details = payment.details as PaymentDetails.Lightning
            val metadata = details.lnurlReceiveMetadata
            
            // Access the Nostr zap request if present
            metadata?.nostrZapRequest?.let { zapRequest ->
                // The zapRequest is a JSON string containing the Nostr event (kind 9734)
                println("Nostr zap request: $zapRequest")
            }
            
            // Access the Nostr zap receipt if present
            metadata?.nostrZapReceipt?.let { zapReceipt ->
                // The zapReceipt is a JSON string containing the Nostr event (kind 9735)
                println("Nostr zap receipt: $zapReceipt")
            }
        }
        // ANCHOR_END: access-nostr-zap
    }
}
