package com.example.kotlinmpplib

import breez_sdk_spark.*
import com.ionspin.kotlin.bignum.integer.BigInteger

class SendPayment {
    suspend fun prepareSendPaymentLightningBolt11(sdk: BreezSdk) {
        // ANCHOR: prepare-send-payment-lightning-bolt11
        val paymentRequest = "<bolt11 invoice>"
        // Optionally set the amount you wish the pay the receiver
        // Kotlin MPP (BigInteger from com.ionspin.kotlin.bignum.integer, which is included in package)
        val optionalAmountSats = BigInteger.fromLong(5_000L)
        // Android (BigInteger from java.math)
        // val optionalAmountSats = BigInteger.valueOf(5_000L)
        try {
            val req = PrepareSendPaymentRequest(paymentRequest, optionalAmountSats)
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
        // Set the amount you wish the pay the receiver
        // Kotlin MPP (BigInteger from com.ionspin.kotlin.bignum.integer, which is included in package)
        val amountSats = BigInteger.fromLong(50_000L)
        // Android (BigInteger from java.math)
        // val amountSats = BigInteger.valueOf(50_000L)
        try {
            val req = PrepareSendPaymentRequest(paymentRequest, amountSats)
            val prepareResponse = sdk.prepareSendPayment(req)

            // If the fees are acceptable, continue to create the Send Payment
            val paymentMethod = prepareResponse.paymentMethod
            if (paymentMethod is SendPaymentMethod.BitcoinAddress) {
                val feeQuote = paymentMethod.feeQuote
                val slowFeeSats = feeQuote.speedSlow.userFeeSat + feeQuote.speedSlow.l1BroadcastFeeSat
                val mediumFeeSats = feeQuote.speedMedium.userFeeSat + feeQuote.speedMedium.l1BroadcastFeeSat
                val fastFeeSats = feeQuote.speedFast.userFeeSat + feeQuote.speedFast.l1BroadcastFeeSat
                // Log.v("Breez", "Slow Fees: ${slowFeeSats} sats")
                // Log.v("Breez", "Medium Fees: ${mediumFeeSats} sats")
                // Log.v("Breez", "Fast Fees: ${fastFeeSats} sats")
            }
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: prepare-send-payment-onchain
    }

    suspend fun prepareSendPaymentSpark(sdk: BreezSdk) {
        // ANCHOR: prepare-send-payment-spark
        val paymentRequest = "<spark address>"
        // Set the amount you wish the pay the receiver
        // Kotlin MPP (BigInteger from com.ionspin.kotlin.bignum.integer, which is included in package)
        val amountSats = BigInteger.fromLong(50_000L)
        // Android (BigInteger from java.math)
        // val amountSats = BigInteger.valueOf(50_000L)
        try {
            val req = PrepareSendPaymentRequest(paymentRequest, amountSats)
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
        // ANCHOR_END: prepare-send-payment-spark
    }

    suspend fun sendPaymentLightningBolt11(sdk: BreezSdk, prepareResponse: PrepareSendPaymentResponse) {
        // ANCHOR: send-payment-lightning-bolt11
        try {
            val options = SendPaymentOptions.Bolt11Invoice(preferSpark = true, completionTimeoutSecs = 10u)
            val sendResponse = sdk.sendPayment(SendPaymentRequest(prepareResponse, options))
            val payment = sendResponse.payment
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: send-payment-lightning-bolt11
    }

    suspend fun sendPaymentOnchain(sdk: BreezSdk, prepareResponse: PrepareSendPaymentResponse) {
        // ANCHOR: send-payment-onchain
        try {
            val options = SendPaymentOptions.BitcoinAddress(OnchainConfirmationSpeed.MEDIUM)
            val sendResponse = sdk.sendPayment(SendPaymentRequest(prepareResponse, options))
            val payment = sendResponse.payment
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: send-payment-onchain
    }

    suspend fun sendPaymentSpark(sdk: BreezSdk, prepareResponse: PrepareSendPaymentResponse) {
        // ANCHOR: send-payment-spark
        try {
            val sendResponse = sdk.sendPayment(SendPaymentRequest(prepareResponse, null))
            val payment = sendResponse.payment
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: send-payment-spark
    }
}
