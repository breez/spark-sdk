package com.example.kotlinmpplib

import breez_sdk_spark.*

class SendPayment {
    suspend fun prepareSendPaymentLightningBolt11(sdk: BreezSdk) {
        // ANCHOR: prepare-send-payment-lightning-bolt11
        val paymentRequest = "<bolt11 invoice>"
        // Optionally set the amount you wish to pay the receiver
        val optionalPayAmount = PayAmount.Bitcoin(amountSats = 5_000.toULong())

        try {
            val req = PrepareSendPaymentRequest(
                paymentRequest,
                payAmount = optionalPayAmount,
            )
            val prepareResponse = sdk.prepareSendPayment(req)

            // If the fees are acceptable, continue to create the Send Payment
            val paymentMethod = prepareResponse.paymentMethod
            if (paymentMethod is SendPaymentMethod.Bolt11Invoice) {
                // Fees to pay via Lightning
                val lightningFeeSats = paymentMethod.lightningFeeSats
                // Or fees to pay (if available) via a Spark transfer
                val sparkTransferFeeSats = paymentMethod.sparkTransferFeeSats
                // Log.v("Breez", "Lightning Fees: ${lightningFeeSats} sats")
                // Log.v("Breez", "Spark Transfer Fees: ${sparkTransferFeeSats} sats")
            }
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: prepare-send-payment-lightning-bolt11
    }

    suspend fun prepareSendPaymentOnchain(sdk: BreezSdk) {
        // ANCHOR: prepare-send-payment-onchain
        val paymentRequest = "<bitcoin address>"
        // Set the amount you wish to pay the receiver
        val payAmount = PayAmount.Bitcoin(amountSats = 50_000.toULong())

        try {
            val req = PrepareSendPaymentRequest(
                paymentRequest,
                payAmount,
            )
            val prepareResponse = sdk.prepareSendPayment(req)

            // Review the fee quote for each confirmation speed
            val paymentMethod = prepareResponse.paymentMethod
            if (paymentMethod is SendPaymentMethod.BitcoinAddress) {
                val feeQuote = paymentMethod.feeQuote
                val slowFeeSats = feeQuote.speedSlow.userFeeSat + feeQuote.speedSlow.l1BroadcastFeeSat
                val mediumFeeSats = feeQuote.speedMedium.userFeeSat + feeQuote.speedMedium.l1BroadcastFeeSat
                val fastFeeSats = feeQuote.speedFast.userFeeSat + feeQuote.speedFast.l1BroadcastFeeSat
                // Log.v("Breez", "Slow fee: $slowFeeSats sats")
                // Log.v("Breez", "Medium fee: $mediumFeeSats sats")
                // Log.v("Breez", "Fast fee: $fastFeeSats sats")
            }
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: prepare-send-payment-onchain
    }

    suspend fun prepareSendPaymentDrain(sdk: BreezSdk) {
        // ANCHOR: prepare-send-payment-drain
        // Use PayAmount.Drain to send all available funds
        val paymentRequest = "<payment request>"
        val payAmount = PayAmount.Drain

        try {
            val req = PrepareSendPaymentRequest(
                paymentRequest,
                payAmount,
            )
            val prepareResponse = sdk.prepareSendPayment(req)

            // The response contains PayAmount.Drain to indicate this is a drain operation
            // Log.v("Breez", "Pay amount: ${prepareResponse.payAmount}")
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: prepare-send-payment-drain
    }

    suspend fun prepareSendPaymentSparkAddress(sdk: BreezSdk) {
        // ANCHOR: prepare-send-payment-spark-address
        val paymentRequest = "<spark address>"
        // Set the amount you wish to pay the receiver
        val payAmount = PayAmount.Bitcoin(amountSats = 50_000.toULong())

        try {
            val req = PrepareSendPaymentRequest(
                paymentRequest,
                payAmount,
            )
            val prepareResponse = sdk.prepareSendPayment(req)

            // If the fees are acceptable, continue to create the Send Payment
            val paymentMethod = prepareResponse.paymentMethod
            if (paymentMethod is SendPaymentMethod.SparkAddress) {
                val feeSats = paymentMethod.fee
                // Log.v("Breez", "Fees: ${feeSats} sats")
            }
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: prepare-send-payment-spark-address
    }

    suspend fun prepareSendPaymentSparkInvoice(sdk: BreezSdk) {
        // ANCHOR: prepare-send-payment-spark-invoice
        val paymentRequest = "<spark invoice>"
        // Optionally set the amount you wish to pay the receiver
        val optionalPayAmount = PayAmount.Bitcoin(amountSats = 50_000.toULong())

        try {
            val req = PrepareSendPaymentRequest(
                paymentRequest,
                optionalPayAmount,
            )
            val prepareResponse = sdk.prepareSendPayment(req)

            // If the fees are acceptable, continue to create the Send Payment
            val paymentMethod = prepareResponse.paymentMethod
            if (paymentMethod is SendPaymentMethod.SparkInvoice) {
                val feeSats = paymentMethod.fee
                // Log.v("Breez", "Fees: ${feeSats} sats")
            }
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: prepare-send-payment-spark-invoice
    }
    
    suspend fun prepareSendPaymentTokenConversion(sdk: BreezSdk) {
        // ANCHOR: prepare-send-payment-with-conversion
        val paymentRequest = "<payment request>"
        // Set to use token funds to pay via conversion
        val optionalMaxSlippageBps = 50u
        val optionalCompletionTimeoutSecs = 30u
        val conversionOptions = ConversionOptions(
            conversionType = ConversionType.ToBitcoin(
                "<token identifier>"
            ),
            maxSlippageBps = optionalMaxSlippageBps,
            completionTimeoutSecs = optionalCompletionTimeoutSecs
        )

        try {
            val req = PrepareSendPaymentRequest(
                paymentRequest,
                payAmount = null,
                conversionOptions = conversionOptions,
            )
            val prepareResponse = sdk.prepareSendPayment(req)

            // If the fees are acceptable, continue to create the Send Payment
            prepareResponse.conversionEstimate?.let { conversionEstimate ->
                // Log.v("Breez", "Estimated conversion amount: ${conversionEstimate.amount} token base units")
                // Log.v("Breez", "Estimated conversion fee: ${conversionEstimate.fee} token base units")
            }
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: prepare-send-payment-with-conversion
    }

    suspend fun sendPaymentLightningBolt11(
            sdk: BreezSdk,
            prepareResponse: PrepareSendPaymentResponse
    ) {
        // ANCHOR: send-payment-lightning-bolt11
        try {
            val options = SendPaymentOptions.Bolt11Invoice(
                preferSpark = false, 
                completionTimeoutSecs = 10u
            )
            val optionalIdempotencyKey = "<idempotency key uuid>"
            val sendResponse = sdk.sendPayment(
                SendPaymentRequest(
                    prepareResponse,
                    options,
                    optionalIdempotencyKey
                )
            )
            val payment = sendResponse.payment
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: send-payment-lightning-bolt11
    }

    suspend fun sendPaymentOnchain(sdk: BreezSdk, prepareResponse: PrepareSendPaymentResponse) {
        // ANCHOR: send-payment-onchain
        try {
            // Select the confirmation speed for the on-chain transaction
            val options = SendPaymentOptions.BitcoinAddress(
                confirmationSpeed = OnchainConfirmationSpeed.MEDIUM
            )
            val optionalIdempotencyKey = "<idempotency key uuid>"
            val sendResponse = sdk.sendPayment(
                SendPaymentRequest(
                    prepareResponse,
                    options,
                    optionalIdempotencyKey
                )
            )
            val payment = sendResponse.payment
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: send-payment-onchain
    }

    suspend fun sendPaymentSpark(sdk: BreezSdk, prepareResponse: PrepareSendPaymentResponse) {
        // ANCHOR: send-payment-spark
        try {
            val optionalIdempotencyKey = "<idempotency key uuid>"
            val sendResponse = sdk.sendPayment(
                SendPaymentRequest(
                    prepareResponse,
                    idempotencyKey = optionalIdempotencyKey
                )
            )
            val payment = sendResponse.payment
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: send-payment-spark
    }
}
