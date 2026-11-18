package com.example.kotlinmpplib

import breez_sdk_spark.*
import com.ionspin.kotlin.bignum.integer.BigInteger

class CashApp {
    suspend fun bridgeFromCashApp(client: BridgingClient) {
        // ANCHOR: bridge-from-cash-app
        // List the routes to the recipient's external-chain address
        // (EVM/Solana/Tron) and pick a stablecoin Cash App can fund over
        // Lightning, e.g. USDC on Base.
        val recipient = "<recipient address>"
        val routes = client.getRoutes(GetBridgeRoutesRequest(recipient = recipient)).routes
        val route = routes.find {
            it.source.chain == "lightning" &&
                it.destination.asset == "USDC" &&
                it.destination.chain == "base"
        } ?: throw IllegalArgumentException("No USDC route on Base")

        // Send $10 of USDC, funded by Cash App over Lightning. The amount is in
        // USDC base units (6 decimals), so 10_000_000 = 10 USDC, about $10.
        val response = client.bridgeFromCashApp(
            CreateBridgeRequest(
                recipient = recipient,
                route = route,
                amount = BigInteger.fromLong(10_000_000L),
                refundAddress = null,
                feePolicy = null,
                maxSlippageBps = null,
            )
        )

        // Open this Cash App link to pay. The recipient then receives the stablecoin.
        val info = response.info
        // Log.v("Breez", "Open this URL in Cash App: ${response.paymentRequest}")
        // Log.v("Breez", "Recipient receives ~${info.expectedReceivedAmount} " +
        //     "${info.destinationAsset}")
        // ANCHOR_END: bridge-from-cash-app
    }

    suspend fun bridgeToCashApp(client: BridgingClient) {
        // ANCHOR: bridge-to-cash-app
        // List the routes to the Cash App user and pick what the payer pays in,
        // e.g. USDC on Base.
        val recipient = "\$alice"
        val routes = client.getRoutes(GetBridgeRoutesRequest(recipient = recipient)).routes
        val route = routes.find { it.source.asset == "USDC" && it.source.chain == "base" }
            ?: throw IllegalArgumentException("No route from USDC on Base")

        // Pay $10 of USDC to the Cash App user $alice. The amount is in USDC base
        // units (6 decimals), so 10_000_000 = 10 USDC, about $10. The deposit is
        // refunded to the payer's address if delivery fails.
        val response = client.bridgeToCashApp(
            CreateBridgeRequest(
                recipient = recipient,
                route = route,
                amount = BigInteger.fromLong(10_000_000L),
                refundAddress = "<payer address>",
                feePolicy = null,
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
