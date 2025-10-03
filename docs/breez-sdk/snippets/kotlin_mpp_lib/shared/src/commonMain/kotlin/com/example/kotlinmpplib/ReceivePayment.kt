package com.example.kotlinmpplib

import breez_sdk_spark.*
class ReceivePayment {
    suspend fun receiveLightning(sdk: BreezSdk) {
        // ANCHOR: receive-payment-lightning-bolt11
        try {
            val description = "<invoice description>"
            // Optionally set the invoice amount you wish the payer to send
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

    suspend fun waitForPayment(sdk: BreezSdk, paymentRequest: String) {
        // ANCHOR: wait-for-payment
        try {
            // Wait for a payment to be completed using a payment request
            val response = sdk.waitForPayment(
                WaitForPaymentRequest(
                    WaitForPaymentIdentifier.PaymentRequest(paymentRequest)
                )
            )
            
            // Log.v("Breez", "Payment received with ID: ${response.payment.id}")
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: wait-for-payment
    }
}
