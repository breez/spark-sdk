package com.example.kotlinmpplib

import breez_sdk_spark.*

class LnurlPay {
    suspend fun prepareLnurlPay(sdk: BreezSdk) {
        // ANCHOR: prepare-lnurl-pay
        // Endpoint can also be of the form:
        // lnurlp://domain.com/lnurl-pay?key=val
        // lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4excttsv9un7um9wdekjmmw84jxywf5x43rvv35xgmr2enrxanr2cfcvsmnwe3jxcukvde48qukgdec89snwde3vfjxvepjxpjnjvtpxd3kvdnxx5crxwpjvyunsephsz36jf
        val lnurlPayUrl = "lightning@address.com"
        try {
            val inputType = sdk.parse(lnurlPayUrl)
            if (inputType is InputType.LightningAddress) {
                val amountSats = 5_000.toULong()
                val optionalComment = "<comment>"
                val payRequest = inputType.v1.payRequest
                val optionalValidateSuccessActionUrl = true
                // Optionally set to use token funds to pay via token conversion
                val optionalMaxSlippageBps = 50u
                val optionalCompletionTimeoutSecs = 30u
                val optionalConversionOptions = ConversionOptions(
                    conversionType = ConversionType.ToBitcoin(
                        "<token identifier>"
                    ),
                    maxSlippageBps = optionalMaxSlippageBps,
                    completionTimeoutSecs = optionalCompletionTimeoutSecs
                )

                val req = PrepareLnurlPayRequest(
                    amountSats = amountSats,
                    payRequest = payRequest,
                    comment = optionalComment,
                    validateSuccessActionUrl = optionalValidateSuccessActionUrl,
                    conversionOptions = optionalConversionOptions,
                    feePolicy = null,
                )
                val prepareResponse = sdk.prepareLnurlPay(req)

                // If the fees are acceptable, continue to create the LNURL Pay
                prepareResponse.conversionEstimate?.let { conversionEstimate ->
                    // Log.v("Breez", "Estimated conversion amount: ${conversionEstimate.amount} token base units")
                    // Log.v("Breez", "Estimated conversion fee: ${conversionEstimate.fee} token base units")
                }

                val feeSats = prepareResponse.feeSats;
                // Log.v("Breez", "Fees: ${feeSats} sats")
            }
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: prepare-lnurl-pay
    }

    suspend fun lnurlPay(sdk: BreezSdk, prepareResponse: PrepareLnurlPayResponse) {
        // ANCHOR: lnurl-pay
        try {
            val optionalIdempotencyKey = "<idempotency key uuid>"
            val response = sdk.lnurlPay(LnurlPayRequest(prepareResponse, optionalIdempotencyKey))
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: lnurl-pay
    }

    suspend fun prepareLnurlPayFeesIncluded(sdk: BreezSdk, payRequest: LnurlPayRequestDetails) {
        // ANCHOR: prepare-lnurl-pay-fees-included
        // By default (FeePolicy.FEES_EXCLUDED), fees are added on top of the amount.
        // Use FeePolicy.FEES_INCLUDED to deduct fees from the amount instead.
        // The receiver gets amount minus fees.
        val optionalComment = "<comment>"
        val optionalValidateSuccessActionUrl = true
        val amountSats = 5_000.toULong()

        val req = PrepareLnurlPayRequest(
            amountSats = amountSats,
            payRequest = payRequest,
            comment = optionalComment,
            validateSuccessActionUrl = optionalValidateSuccessActionUrl,
            conversionOptions = null,
            feePolicy = FeePolicy.FEES_INCLUDED,
        )
        val prepareResponse = sdk.prepareLnurlPay(req)

        // If the fees are acceptable, continue to create the LNURL Pay
        val feeSats = prepareResponse.feeSats
        // Log.v("Breez", "Fees: ${feeSats} sats")
        // The receiver gets amountSats - feeSats
        // ANCHOR_END: prepare-lnurl-pay-fees-included
    }
}