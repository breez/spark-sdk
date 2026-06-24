package com.example.kotlinmpplib

import breez_sdk_spark.*
import com.ionspin.kotlin.bignum.integer.BigInteger

class CrossChain {
    suspend fun getCrossChainRoutes(sdk: BreezSdk) {
        // ANCHOR: cross-chain-get-routes
        val input = "<recipient address>"

        try {
            val parsed = sdk.parse(input)
            if (parsed !is InputType.CrossChainAddress) {
                throw IllegalArgumentException("Not a cross-chain address")
            }
            val addressDetails = parsed.v1

            val routes = sdk.getCrossChainRoutes(
                CrossChainRouteFilter.Send(addressDetails = addressDetails)
            )

            for (route in routes) {
                // Log.v("Breez", "Route via ${route.provider}: ${route.chain}/${route.asset}")
            }
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: cross-chain-get-routes
    }

    suspend fun prepareSendPaymentCrossChain(
        sdk: BreezSdk,
        addressDetails: CrossChainAddressDetails,
        route: CrossChainRoutePair,
    ) {
        // ANCHOR: cross-chain-prepare
        // Optionally set the maximum slippage in basis points (10 to 500)
        val optionalMaxSlippageBps: UInt? = 100u

        try {
            val req = PrepareSendPaymentRequest(
                paymentRequest = PaymentRequest.CrossChain(
                    address = addressDetails.address,
                    route = route,
                    maxSlippageBps = optionalMaxSlippageBps,
                    targetOverpayBps = null,
                ),
                amount = BigInteger.fromLong(50_000L),
                tokenIdentifier = null,
                conversionOptions = null,
                feePolicy = null,
            )
            val prepareResponse = sdk.prepareSendPayment(req)

            val paymentMethod = prepareResponse.paymentMethod
            if (paymentMethod is SendPaymentMethod.CrossChainAddress) {
                val amountIn = paymentMethod.amountIn
                val estimatedOut = paymentMethod.estimatedOut
                val feeAmount = paymentMethod.feeAmount
                val expiresAt = paymentMethod.expiresAt
                // Log.v("Breez", "Amount in: $amountIn")
                // Log.v("Breez", "Estimated out: $estimatedOut")
                // Log.v("Breez", "Provider fee: $feeAmount")
                // Log.v("Breez", "Quote expires at: $expiresAt")
            }
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: cross-chain-prepare
    }

    suspend fun sendPaymentCrossChain(
        sdk: BreezSdk,
        prepareResponse: PrepareSendPaymentResponse,
    ) {
        // ANCHOR: cross-chain-send
        // Only valid for sends with no token leg (see Retry safety).
        val optionalIdempotencyKey = "<idempotency key uuid>"
        try {
            val req = SendPaymentRequest(
                prepareResponse = prepareResponse,
                options = null,
                idempotencyKey = optionalIdempotencyKey,
            )
            val sendResponse = sdk.sendPayment(req)
            val payment = sendResponse.payment
            // Log.v("Breez", "Payment: $payment")
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: cross-chain-send
    }

    suspend fun getCrossChainReceiveRoutes(sdk: BreezSdk) {
        // ANCHOR: cross-chain-get-receive-routes
        try {
            val routes = sdk.getCrossChainRoutes(
                CrossChainRouteFilter.Receive(contractAddress = null)
            )

            for (route in routes) {
                // Log.v(
                //   "Breez",
                //   "Route via ${route.provider}: ${route.chain}/${route.asset} -> Spark"
                // )
            }
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: cross-chain-get-receive-routes
    }

    suspend fun receivePaymentCrossChain(sdk: BreezSdk, route: CrossChainRoutePair) {
        // ANCHOR: cross-chain-receive
        // amount is in source-asset base units
        // (e.g. USDC base units when source is USDC)
        val amount = BigInteger.fromLong(1_000_000L)
        // Optionally set the destination Spark-side asset. null = auto:
        // active stable-balance token if the route supports it, otherwise BTC.
        val optionalDestination: SparkAsset? = null
        // Optionally set the maximum slippage in basis points (10 to 500)
        val optionalMaxSlippageBps: UInt? = 100u
        try {
            val req = ReceivePaymentRequest(
                paymentMethod = ReceivePaymentMethod.CrossChain(
                    route = route,
                    amount = amount,
                    destination = optionalDestination,
                    maxSlippageBps = optionalMaxSlippageBps,
                ),
            )
            val response = sdk.receivePayment(req)
            val depositAddress = response.paymentRequest
            // Log.v("Breez", "Share this deposit address with the sender: $depositAddress")
            val info = response.crossChainInfo
            if (info != null) {
                val depositAmount = info.depositAmount
                val expected = info.expectedReceivedAmount
                val denom = if (info.tokenIdentifier != null) "USDB" else "BTC"
                val expiresAt = info.expiresAt
                // Log.v("Breez", "Sender deposits: $depositAmount")
                // Log.v("Breez", "Receiver gets ~$expected $denom")
                // Log.v("Breez", "Quote expires at: $expiresAt")
            }
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: cross-chain-receive
    }
}
