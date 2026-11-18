package com.example.kotlinmpplib

import breez_sdk_spark.*
import com.ionspin.kotlin.bignum.integer.BigInteger

class Bridging {
    suspend fun newClient(): BridgingClient {
        // ANCHOR: new-bridging-client
        val config = defaultBridgingConfig("<your api key>")
        val client = newBridgingClient(config)
        // ANCHOR_END: new-bridging-client
        return client
    }

    fun clientFromSdk(sdk: BreezSdk): BridgingClient {
        // ANCHOR: get-bridging-client
        val client = sdk.getBridgingClient()
        // ANCHOR_END: get-bridging-client
        return client
    }

    suspend fun createBridge(client: BridgingClient) {
        // ANCHOR: create-bridge
        // Who to pay: a Spark address, a Lightning address or an EVM/Solana/Tron
        // address.
        val recipient = "<recipient>"

        // List the routes that pay them and pick what the payer pays in, e.g.
        // USDC on Base.
        val routes = client.getRoutes(GetBridgeRoutesRequest(recipient = recipient)).routes
        val route = routes.find { it.source.asset == "USDC" && it.source.chain == "base" }
            ?: throw IllegalArgumentException("No route from USDC on Base")

        // Ask for $10. The amount is in the recipient's stablecoin when they are
        // paid in one, the payer's otherwise (6 decimals for USDC). The deposit is
        // refunded to the payer's address if the recipient can't be paid.
        val response = client.createBridge(
            CreateBridgeRequest(
                recipient = recipient,
                route = route,
                amount = BigInteger.fromLong(10_000_000L),
                // Required to pay a Lightning address.
                refundAddress = "<payer address on Base>",
                feePolicy = null,
                maxSlippageBps = null,
            )
        )

        // Show the payer what to pay, and store the bridge id and status token to
        // look up the status later.
        // Log.v("Breez", "Pay with: ${response.paymentRequest}")
        // Log.v("Breez", "Bridge id: ${response.bridgeId}")
        // ANCHOR_END: create-bridge
    }

    suspend fun getBridgeStatus(
        client: BridgingClient,
        bridgeId: String,
        statusToken: String?,
    ) {
        // ANCHOR: get-bridge-status
        val response = client.getBridgeStatus(
            GetBridgeStatusRequest(bridgeId = bridgeId, statusToken = statusToken)
        )
        when (response.status) {
            BridgeStatus.COMPLETED -> {
                // Log.v("Breez", "Delivered ${response.amountOut}")
            }
            BridgeStatus.REFUNDED, BridgeStatus.FAILED, BridgeStatus.EXPIRED -> {
                // Log.v("Breez", "Not delivered: ${response.status}")
            }
            else -> {
                // Log.v("Breez", "Not final yet: ${response.status}")
            }
        }
        // ANCHOR_END: get-bridge-status
    }
}
