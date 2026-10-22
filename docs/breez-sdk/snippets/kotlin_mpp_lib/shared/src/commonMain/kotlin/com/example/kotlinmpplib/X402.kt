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

            val paymentSignature = response.paymentSignature
            if (paymentSignature != null) {
                // Send the request again with a PAYMENT-SIGNATURE header
                // Log.v("Breez", "PAYMENT-SIGNATURE: $paymentSignature")
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

            val paymentSignature = response.paymentSignature
            if (paymentSignature != null) {
                // Call the tool again with this JSON in params._meta["x402/payment"]
                // Log.v("Breez", "Payment: $paymentSignature")
            }
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: pay-mcp-tool-call
    }
}
