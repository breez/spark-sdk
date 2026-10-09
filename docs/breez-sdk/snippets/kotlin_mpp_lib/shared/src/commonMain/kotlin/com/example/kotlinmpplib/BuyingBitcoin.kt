package com.example.kotlinmpplib

import breez_sdk_spark.*
import com.ionspin.kotlin.bignum.integer.BigInteger

class BuyingBitcoin {
    suspend fun buyBitcoin(sdk: BreezSdk) {
        // ANCHOR: buy-bitcoin
        // Optionally, prefill the purchase amount
        val optionalAmountSat: ULong? = 100_000u
        // Optionally, set a redirect URL for after the purchase is completed
        val optionalRedirectUrl: String? = "https://example.com/purchase-complete"

        val request = BuyBitcoinRequest.Moonpay(
            delivery = MoonpayDelivery.Bitcoin(amountSat = optionalAmountSat),
            redirectUrl = optionalRedirectUrl
        )

        val response = sdk.buyBitcoin(request)
        // Log.v("Breez", "Open this URL in a browser to complete the purchase:")
        // Log.v("Breez", "${response.url}")
        // ANCHOR_END: buy-bitcoin
    }

    suspend fun buyBitcoinViaCrossChain(sdk: BreezSdk) {
        // ANCHOR: buy-bitcoin-cross-chain
        // USD amount to receive, in 6-decimal base units ($50)
        val amount = BigInteger.fromLong(50_000_000L)

        val request = BuyBitcoinRequest.Moonpay(
            delivery = MoonpayDelivery.CrossChain(
                amount = amount,
                feeMode = null
            ),
            redirectUrl = null
        )

        val response = sdk.buyBitcoin(request)
        // Log.v("Breez", "Open this URL in a browser to complete the purchase:")
        // Log.v("Breez", "${response.url}")

        response.crossChainInfo?.let { info ->
            // Log.v("Breez", "USDC to buy: ${info.depositAmount}")
            // Log.v("Breez", "Expected to receive: ${info.expectedReceivedAmount} " +
            //     "${info.destinationAsset}")
            // Log.v("Breez", "Conversion fee: ${info.serviceFeeAmount}")
        }
        // ANCHOR_END: buy-bitcoin-cross-chain
    }

    suspend fun buyBitcoinViaCashapp(sdk: BreezSdk) {
        // ANCHOR: buy-bitcoin-cashapp
        // Cash App requires the amount to be specified up front.
        val amountSats: ULong = 50_000u

        val request = BuyBitcoinRequest.CashApp(amountSats = amountSats)

        val response = sdk.buyBitcoin(request)
        // Log.v("Breez", "Open this URL in Cash App to complete the purchase:")
        // Log.v("Breez", "${response.url}")
        // ANCHOR_END: buy-bitcoin-cashapp
    }
}
