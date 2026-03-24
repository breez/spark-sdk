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
            provider = BuyBitcoinProvider.MOONPAY,
            lockedAmountSat = optionalLockedAmountSat,
            redirectUrl = optionalRedirectUrl
        )

        val response = sdk.buyBitcoin(request)
        // Log.v("Breez", "Open this URL in a browser to complete the purchase:")
        // Log.v("Breez", "${response.url}")
        // ANCHOR_END: buy-bitcoin
    }

    suspend fun buyBitcoinViaCashapp(sdk: BreezSdk) {
        // ANCHOR: buy-bitcoin-cashapp
        val request = BuyBitcoinRequest(
            provider = BuyBitcoinProvider.CASH_APP
        )

        val response = sdk.buyBitcoin(request)
        // Log.v("Breez", "Open this URL in Cash App to complete the purchase:")
        // Log.v("Breez", "${response.url}")
        // ANCHOR_END: buy-bitcoin-cashapp
    }
}
