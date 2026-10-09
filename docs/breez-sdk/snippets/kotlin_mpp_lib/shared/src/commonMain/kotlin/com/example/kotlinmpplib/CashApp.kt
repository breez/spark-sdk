package com.example.kotlinmpplib

import breez_sdk_spark.*
import com.ionspin.kotlin.bignum.integer.BigInteger

class CashApp {
    suspend fun bridgeFromCashApp(sdk: BreezSdk) {
        // ANCHOR: bridge-from-cash-app
        // Parse the recipient's external-chain address (EVM/Solana/Tron).
        val input = "<recipient address>"
        val parsed = sdk.parse(input)
        if (parsed !is InputType.CrossChainAddress) {
            throw IllegalArgumentException("Not a cross-chain address")
        }
        val addressDetails = parsed.v1

        // List the stablecoin destinations Cash App can fund over Lightning and
        // pick one, e.g. USDC on Base.
        val routes = sdk.getCrossChainRoutes(
            CrossChainRouteFilter.Send(
                addressDetails = addressDetails,
                deliveryMethod = DeliveryMethod.LIGHTNING,
            )
        )
        val route = routes.find { it.asset == "USDC" && it.chain == "base" }
            ?: throw IllegalArgumentException("No USDC route on Base")

        // Send $10 of USDC, funded by Cash App over Lightning. The amount is in
        // the route asset's base units (USDC, 6 decimals), so 10_000_000 =
        // 10 USDC, about $10.
        val response = sdk.bridgeFromCashApp(
            BridgeFromCashAppRequest(
                address = addressDetails.address,
                route = route,
                amount = BigInteger.fromLong(10_000_000L),
                feePolicy = null,
                maxSlippageBps = null,
            )
        )

        // Open this Cash App URL to pay. The recipient then receives the stablecoin.
        // Log.v("Breez", "Open this URL in Cash App: ${response.url}")
        // Log.v("Breez", "Recipient receives ~${response.estimatedOut} ${response.asset}")
        // ANCHOR_END: bridge-from-cash-app
    }

    suspend fun bridgeToCashApp(sdk: BreezSdk) {
        // ANCHOR: bridge-to-cash-app
        // List the stablecoin sources that can pay a Cash App user over Lightning
        // and pick one, e.g. USDC on Base.
        val routes = sdk.getCrossChainRoutes(
            CrossChainRouteFilter.Receive(
                contractAddress = null,
                deliveryMethod = DeliveryMethod.LIGHTNING,
            )
        )
        val route = routes.find { it.asset == "USDC" && it.chain == "base" }
            ?: throw IllegalArgumentException("No USDC route on Base")

        // Pay $10 of USDC to the Cash App user $alice. The amount is in the route
        // asset's base units (USDC, 6 decimals), so 10_000_000 = 10 USDC, about $10.
        // The deposit is refunded to the payer's address if delivery fails.
        val response = sdk.bridgeToCashApp(
            BridgeToCashAppRequest(
                recipient = "\$alice",
                route = route,
                amount = BigInteger.fromLong(10_000_000L),
                feePolicy = null,
                refundAddress = "<payer address>",
                maxSlippageBps = null,
            )
        )

        // Show the payer what to pay. The recipient then receives Bitcoin.
        val info = response.info
        // Log.v("Breez", "Pay with: ${response.paymentRequest}")
        // Log.v("Breez", "Deposit ${info.depositAmount} to ${info.depositAddress}, " +
        //     "recipient receives ~${info.expectedReceivedAmount} sats")
        // ANCHOR_END: bridge-to-cash-app
    }
}
