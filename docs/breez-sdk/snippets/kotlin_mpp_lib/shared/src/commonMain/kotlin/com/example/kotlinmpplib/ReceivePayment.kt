package com.example.kotlinmpplib

import breez_sdk_spark.*
class ReceivePayment {
    suspend fun receiveLightning(sdk: BreezSdk) {
        // ANCHOR: receive-payment-lightning-bolt11
        try {
            val description = "<invoice description>"
            // Set the invoice amount you wish the payer to send, which should be within the above limits
            val optionalAmountSats = 5_000.toULong()

            val request = ReceivePaymentRequest(
                ReceivePaymentMethod.Bolt11Invoice(description, optionalAmountSats)
            )
            val response = sdk.receivePayment(request)

            val paymentRequest = response.paymentRequest
            // Log.v("Breez", "Payment Request: ${paymentRequest}")
            val receiveFeeSats = response.feeSats
            // Log.v("Breez", "Fees: ${receiveFeeSats} sats")
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: receive-payment-lightning-bolt11
    }

    suspend fun receiveOnchain(sdk: BreezSdk) {
        // ANCHOR: receive-payment-onchain
        try {
            val request = ReceivePaymentRequest(
                ReceivePaymentMethod.BitcoinAddress
            )
            val response = sdk.receivePayment(request)

            val paymentRequest = response.paymentRequest
            // Log.v("Breez", "Payment Request: ${paymentRequest}")
            val receiveFeeSats = response.feeSats
            // Log.v("Breez", "Fees: ${receiveFeeSats} sats")
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: receive-payment-onchain
    }

    suspend fun receiveSpark(sdk: BreezSdk) {
        // ANCHOR: receive-payment-spark
        try {
            val request = ReceivePaymentRequest(
                ReceivePaymentMethod.SparkAddress
            );
            val response = sdk.receivePayment(request)

            val paymentRequest = response.paymentRequest
            // Log.v("Breez", "Payment Request: ${paymentRequest}")
            val receiveFeeSats = response.feeSats
            // Log.v("Breez", "Fees: ${receiveFeeSats} sats")
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: receive-payment-spark
    }
}
