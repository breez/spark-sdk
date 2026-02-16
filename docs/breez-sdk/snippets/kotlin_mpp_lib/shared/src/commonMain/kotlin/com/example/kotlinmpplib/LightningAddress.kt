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

    suspend fun checkLightningAddressAvailability(client: BreezClient) {
        val username = "myusername"
        
        // ANCHOR: check-lightning-address
        val request = CheckLightningAddressRequest(
            username = username
        )
        
        val available = client.lightningAddress().isAvailable(request)
        // ANCHOR_END: check-lightning-address
    }

    suspend fun registerLightningAddress(client: BreezClient) {
        val username = "myusername"
        val description = "My Lightning Address"
        
        // ANCHOR: register-lightning-address
        val request = RegisterLightningAddressRequest(
            username = username,
            description = description
        )
        
        val addressInfo = client.lightningAddress().register(request)
        val lightningAddress = addressInfo.lightningAddress
        val lnurlUrl = addressInfo.lnurl.url
        val lnurlBech32 = addressInfo.lnurl.bech32
        // ANCHOR_END: register-lightning-address
    }

    suspend fun getLightningAddress(client: BreezClient) {
        // ANCHOR: get-lightning-address
        val addressInfoOpt = client.lightningAddress().get()
        
        if (addressInfoOpt != null) {
            val lightningAddress = addressInfoOpt.lightningAddress
            val username = addressInfoOpt.username
            val description = addressInfoOpt.description
            val lnurlUrl = addressInfoOpt.lnurl.url
            val lnurlBech32 = addressInfoOpt.lnurl.bech32
        }
        // ANCHOR_END: get-lightning-address
    }

    suspend fun deleteLightningAddress(client: BreezClient) {
        // ANCHOR: delete-lightning-address
        client.lightningAddress().delete()
        // ANCHOR_END: delete-lightning-address
    }

    suspend fun accessSenderComment(client: BreezClient) {
        val paymentId = "<payment id>"
        val response = client.payments().get(GetPaymentRequest(paymentId = paymentId))
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

    suspend fun accessNostrZap(client: BreezClient) {
        val paymentId = "<payment id>"
        val response = client.payments().get(GetPaymentRequest(paymentId = paymentId))
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
