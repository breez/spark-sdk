package com.example.kotlinmpplib

import breez_sdk_spark.*

class LightningAddress {
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
        val lnurl = addressInfo.lnurl
        // ANCHOR_END: register-lightning-address
    }

    suspend fun getLightningAddress(sdk: BreezSdk) {
        // ANCHOR: get-lightning-address
        val addressInfoOpt = sdk.getLightningAddress()
        
        if (addressInfoOpt != null) {
            val lightningAddress = addressInfoOpt.lightningAddress
            val username = addressInfoOpt.username
            val description = addressInfoOpt.description
            val lnurl = addressInfoOpt.lnurl
        }
        // ANCHOR_END: get-lightning-address
    }

    suspend fun deleteLightningAddress(sdk: BreezSdk) {
        // ANCHOR: delete-lightning-address
        sdk.deleteLightningAddress()
        // ANCHOR_END: delete-lightning-address
    }
}
