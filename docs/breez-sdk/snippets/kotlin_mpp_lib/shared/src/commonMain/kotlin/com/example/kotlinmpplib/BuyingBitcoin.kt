package com.example.kotlinmpplib

import breez_sdk_spark.*

class BuyingBitcoin {
    suspend fun buyBitcoinBasic(sdk: BreezSdk) {
        // ANCHOR: buy-bitcoin-basic
        val request = BuyBitcoinRequest(
            address = "bc1qexample...", // Your Bitcoin address
            lockedAmountSat = null,
            maxAmountSat = null,
            redirectUrl = null
        )

        val response = sdk.buyBitcoin(request)
        // Log.v("Breez", "Open this URL in a browser to complete the purchase:")
        // Log.v("Breez", "${response.url}")
        // ANCHOR_END: buy-bitcoin-basic
    }

    suspend fun buyBitcoinWithAmount(sdk: BreezSdk) {
        // ANCHOR: buy-bitcoin-with-amount
        // Lock the purchase to a specific amount (e.g., 0.001 BTC = 100,000 sats)
        val request = BuyBitcoinRequest(
            address = "bc1qexample...",
            lockedAmountSat = 100_000u, // Pre-fill with 100,000 sats
            maxAmountSat = null,
            redirectUrl = null
        )

        val response = sdk.buyBitcoin(request)
        // Log.v("Breez", "Open this URL in a browser to complete the purchase:")
        // Log.v("Breez", "${response.url}")
        // ANCHOR_END: buy-bitcoin-with-amount
    }

    suspend fun buyBitcoinWithLimits(sdk: BreezSdk) {
        // ANCHOR: buy-bitcoin-with-limits
        // Set both a locked amount and maximum amount
        val request = BuyBitcoinRequest(
            address = "bc1qexample...",
            lockedAmountSat = 50_000u,   // Pre-fill with 50,000 sats
            maxAmountSat = 500_000u,     // Limit to 500,000 sats max
            redirectUrl = null
        )

        val response = sdk.buyBitcoin(request)
        // Log.v("Breez", "Open this URL in a browser to complete the purchase:")
        // Log.v("Breez", "${response.url}")
        // ANCHOR_END: buy-bitcoin-with-limits
    }

    suspend fun buyBitcoinWithRedirect(sdk: BreezSdk) {
        // ANCHOR: buy-bitcoin-with-redirect
        // Provide a custom redirect URL for after the purchase
        val request = BuyBitcoinRequest(
            address = "bc1qexample...",
            lockedAmountSat = 100_000u,
            maxAmountSat = null,
            redirectUrl = "https://example.com/purchase-complete"
        )

        val response = sdk.buyBitcoin(request)
        // Log.v("Breez", "Open this URL in a browser to complete the purchase:")
        // Log.v("Breez", "${response.url}")
        // ANCHOR_END: buy-bitcoin-with-redirect
    }
}
