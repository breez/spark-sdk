package com.example.kotlinmpplib

import breez_sdk_spark.*

class BuyingBitcoin {
    suspend fun buyBitcoinBasic(sdk: BreezSdk) {
        // ANCHOR: buy-bitcoin-basic
        // Buy Bitcoin using the SDK's auto-generated deposit address
        val request = BuyBitcoinRequest()

        val response = sdk.buyBitcoin(request)
        // Log.v("Breez", "Open this URL in a browser to complete the purchase:")
        // Log.v("Breez", "${response.url}")
        // ANCHOR_END: buy-bitcoin-basic
    }

    suspend fun buyBitcoinWithAmount(sdk: BreezSdk) {
        // ANCHOR: buy-bitcoin-with-amount
        // Lock the purchase to a specific amount (e.g., 0.001 BTC = 100,000 sats)
        val request = BuyBitcoinRequest(
            lockedAmountSat = 100_000u
        )

        val response = sdk.buyBitcoin(request)
        // Log.v("Breez", "Open this URL in a browser to complete the purchase:")
        // Log.v("Breez", "${response.url}")
        // ANCHOR_END: buy-bitcoin-with-amount
    }

    suspend fun buyBitcoinWithRedirect(sdk: BreezSdk) {
        // ANCHOR: buy-bitcoin-with-redirect
        // Provide a custom redirect URL for after the purchase
        val request = BuyBitcoinRequest(
            lockedAmountSat = 100_000u,
            redirectUrl = "https://example.com/purchase-complete"
        )

        val response = sdk.buyBitcoin(request)
        // Log.v("Breez", "Open this URL in a browser to complete the purchase:")
        // Log.v("Breez", "${response.url}")
        // ANCHOR_END: buy-bitcoin-with-redirect
    }

    suspend fun buyBitcoinWithAddress(sdk: BreezSdk) {
        // ANCHOR: buy-bitcoin-with-address
        // Specify a custom Bitcoin address to receive funds
        val request = BuyBitcoinRequest(
            address = "bc1qexample...",
            lockedAmountSat = 100_000u
        )

        val response = sdk.buyBitcoin(request)
        // Log.v("Breez", "Open this URL in a browser to complete the purchase:")
        // Log.v("Breez", "${response.url}")
        // ANCHOR_END: buy-bitcoin-with-address
    }
}
