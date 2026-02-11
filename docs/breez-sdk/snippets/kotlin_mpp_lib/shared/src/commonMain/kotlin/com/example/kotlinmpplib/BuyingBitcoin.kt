package com.example.kotlinmpplib

import breez_sdk_spark.*

class BuyingBitcoin {
    suspend fun buyBitcoin(sdk: BreezSdk) {
        // ANCHOR: buy-bitcoin
        // Optionally, lock the purchase to a specific amount
        val optionalLockedAmountSat: ULong = 100_000u
        // Optionally, set a redirect URL for after the purchase is completed
        val optionalRedirectUrl = "https://example.com/purchase-complete"

        val request = BuyBitcoinRequest(
            lockedAmountSat = optionalLockedAmountSat,
            redirectUrl = optionalRedirectUrl
        )

        val response = sdk.buyBitcoin(request)
        // Log.v("Breez", "Open this URL in a browser to complete the purchase:")
        // Log.v("Breez", "${response.url}")
        // ANCHOR_END: buy-bitcoin
    }
}
