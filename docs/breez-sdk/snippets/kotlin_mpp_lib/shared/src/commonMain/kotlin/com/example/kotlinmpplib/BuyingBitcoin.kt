package com.example.kotlinmpplib

import breez_sdk_spark.*

class BuyingBitcoin {
    suspend fun buyBitcoin(sdk: BreezSdk) {
        // ANCHOR: buy-bitcoin
        // Buy Bitcoin with funds deposited directly into the user's wallet.
        // Optionally lock the purchase to a specific amount and provide a redirect URL.
        val request = BuyBitcoinRequest(
            lockedAmountSat = 100_000u,
            redirectUrl = "https://example.com/purchase-complete"
        )

        val response = sdk.buyBitcoin(request)
        // Log.v("Breez", "Open this URL in a browser to complete the purchase:")
        // Log.v("Breez", "${response.url}")
        // ANCHOR_END: buy-bitcoin
    }
}
