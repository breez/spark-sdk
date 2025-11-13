package com.example.kotlinmpplib

import breez_sdk_spark.*
class LnurlWithdraw {
    suspend fun lnurlWithdraw(sdk: BreezSdk) {
        // ANCHOR: lnurl-withdraw
        // Endpoint can also be of the form:
        // lnurlw://domain.com/lnurl-withdraw?key=val
        val lnurlWithdrawUrl = "lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7..."
        try {
            val inputType = sdk.parse(lnurlWithdrawUrl)
            if (inputType is InputType.LnurlWithdraw) {
                // Amount to withdraw in sats between min/max withdrawable amounts
                val amountSats = 5_000.toULong()
                val withdrawRequest = inputType.v1
                val optionalCompletionTimeoutSecs = 30.toUInt()

                val request = LnurlWithdrawRequest(
                    amountSats,
                    withdrawRequest,
                    optionalCompletionTimeoutSecs
                )
                val response = sdk.lnurlWithdraw(request)

                val payment = response.payment
                // Log.v("Breez", "Payment: $payment")
            }
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: lnurl-withdraw
    }
}