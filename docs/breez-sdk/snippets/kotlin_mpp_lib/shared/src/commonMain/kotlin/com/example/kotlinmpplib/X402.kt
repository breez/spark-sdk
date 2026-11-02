package com.example.kotlinmpplib

import breez_sdk_spark.*

class X402Snippets {
    fun getX402(sdk: BreezSdk): X402 {
        // ANCHOR: get-x402
        val x402 = sdk.getX402()
        // ANCHOR_END: get-x402
        return x402
    }

    suspend fun payHttpRequest(x402: X402, paymentRequiredHeader: String) {
        // ANCHOR: pay-http-request
        try {
            // The request that returned 402, exactly as it will be sent again
            val request = X402Request.Http(
                method = "GET",
                url = "https://api.example.com/article/A",
                headers = listOf(X402Header(name = "accept", value = "application/json")),
                body = null
            )
            val response = x402.pay(
                PayX402Request(
                    // The PAYMENT-REQUIRED header of the 402 response
                    paymentRequired = paymentRequiredHeader,
                    request = request,
                    maxAmount = X402PaymentAmount.Lightning(amountSats = 100u),
                    conversionOptions = null,
                    completionTimeoutSecs = null
                )
            )

            if (response.paymentSignature != null) {
                // Send the request again with it as the PAYMENT-SIGNATURE header.
                // It proves the payment, so don't log it.
            } else {
                // Still in flight: paying the same challenge again picks it up
                // Log.v("Breez", "Payment status: ${response.status}")
            }
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: pay-http-request
    }

    suspend fun payMcpToolCall(x402: X402, structuredContent: String) {
        // ANCHOR: pay-mcp-tool-call
        try {
            val request = X402Request.Mcp(
                server = "https://api.example.com/mcp",
                // The tools/call params, exactly as they will be sent again
                params = """{"name":"get_article","arguments":{"article":"A"}}"""
            )
            val response = x402.pay(
                PayX402Request(
                    // The structuredContent of the tool result
                    paymentRequired = structuredContent,
                    request = request,
                    maxAmount = X402PaymentAmount.Lightning(amountSats = 100u),
                    conversionOptions = null,
                    completionTimeoutSecs = null
                )
            )

            if (response.paymentSignature != null) {
                // Call the tool again with this JSON in params._meta["x402/payment"]
            }
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: pay-mcp-tool-call
    }

    suspend fun createChallenge(x402: X402) {
        // ANCHOR: create-challenge
        try {
            // The unpaid request, with its URL built from your public origin
            val request = X402Request.Http(
                method = "GET",
                url = "https://api.example.com/article/A",
                headers = listOf(X402Header(name = "accept", value = "application/json")),
                body = null
            )
            val response = x402.createChallenge(
                CreateX402ChallengeRequest(
                    request = request,
                    // Headers that change what is served, so the payment covers them too
                    boundNames = listOf("accept"),
                    amounts = listOf(X402PaymentAmount.Lightning(amountSats = 21u)),
                    maxTimeoutSecs = null,
                    description = "An article",
                    mimeType = "application/json"
                )
            )

            // Respond with status 402 and this PAYMENT-REQUIRED header
            // Log.v("Breez", "PAYMENT-REQUIRED: ${response.paymentRequired}")
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: create-challenge
    }

    suspend fun settlePayment(x402: X402, paymentSignatureHeader: String) {
        // ANCHOR: settle-payment
        try {
            // The paid request, exactly as received
            val request = X402Request.Http(
                method = "GET",
                url = "https://api.example.com/article/A",
                headers = listOf(X402Header(name = "accept", value = "application/json")),
                body = null
            )
            val response = x402.settle(
                SettleX402Request(
                    // The PAYMENT-SIGNATURE header of the request
                    paymentSignature = paymentSignatureHeader,
                    request = request,
                    boundNames = listOf("accept"),
                    // The same price and lifetime the challenge was created with
                    amounts = listOf(X402PaymentAmount.Lightning(amountSats = 21u)),
                    maxTimeoutSecs = null
                )
            )

            // Send the PAYMENT-RESPONSE header whatever the status
            // Log.v("Breez", "PAYMENT-RESPONSE: ${response.paymentResponse}")
            when (response.status) {
                X402SettleStatus.SETTLED -> {
                    // Serve the resource
                }
                X402SettleStatus.PENDING -> {
                    // Respond with status 402 and no new challenge
                }
                X402SettleStatus.MALFORMED -> {
                    // Respond with status 400
                }
                X402SettleStatus.REFUSED -> {
                    // Respond with status 402 and a new challenge
                }
                X402SettleStatus.ERROR -> {
                    // Respond with status 500
                }
            }
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: settle-payment
    }
}
