package com.example.kotlinmpplib

import breez_sdk_spark.*
import com.ionspin.kotlin.bignum.integer.BigInteger

class PreparePaymentLink {
    suspend fun preparePaymentLinkViaCashapp(sdk: BreezSdk) {
        // ANCHOR: prepare-payment-link-cashapp
        // Parse the recipient's external-chain address (EVM/Solana/Tron).
        val input = "<recipient address>"
        val parsed = sdk.parse(input)
        if (parsed !is InputType.CrossChainAddress) {
            throw IllegalArgumentException("Not a cross-chain address")
        }
        val addressDetails = parsed.v1

        // List the stablecoin destinations you can send to and pick one, e.g. USDC
        // on Base. These routes are funded by an external rail, not the Spark wallet.
        // Restrict to Lightning-fundable routes, since Cash App pays over Lightning.
        val routes = sdk.getCrossChainRoutes(
            CrossChainRouteFilter.PaymentLink(
                addressDetails = addressDetails,
            )
        )
        val route = routes.find { it.asset == "USDC" && it.chain == "base" }
            ?: throw IllegalArgumentException("No USDC route on Base")

        // Send $10 of USDC, funded by Cash App over Lightning. The amount is in USD
        // base units (6-decimal), so 10_000_000 = $10.00.
        val request = PreparePaymentLinkRequest(
            address = addressDetails.address,
            route = route,
            amount = BigInteger.fromLong(10_000_000L),
            feePolicy = null,
            maxSlippageBps = null,
        )

        val response = sdk.preparePaymentLink(request)

        // Open this Cash App URL to pay; the recipient then receives the stablecoin.
        // Log.v("Breez", "Open this URL in Cash App: ${response.url}")
        // Log.v("Breez", "Recipient receives ~${response.estimatedOut} ${response.asset}")
        // ANCHOR_END: prepare-payment-link-cashapp
    }
}
