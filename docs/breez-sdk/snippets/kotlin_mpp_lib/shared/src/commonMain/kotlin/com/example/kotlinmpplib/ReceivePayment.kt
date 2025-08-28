package com.example.kotlinmpplib

import breez_sdk_spark.*
class ReceivePayment {
    fun prepareReceiveLightning(sdk: BreezSdk) {
        // ANCHOR: prepare-receive-payment-lightning
        try {
            val description = "<invoice description>"
            // Set the invoice amount you wish the payer to send, which should be within the above limits
            val optionalAmountSats = 5_000.toULong()

            val prepareRequest = PrepareReceivePaymentRequest(
                ReceivePaymentMethod.Bolt11Invoice(description, optionalAmountSats)
            );
            val prepareResponse = sdk.prepareReceivePayment(prepareRequest)

            val receiveFeeSats = prepareResponse.feeSats
            // Log.v("Breez", "Fees: ${receiveFeeSats} sats")
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: prepare-receive-payment-lightning
    }

    fun prepareReceiveOnchain(sdk: BreezSdk) {
        // ANCHOR: prepare-receive-payment-onchain
        try {
            val prepareRequest = PrepareReceivePaymentRequest(
                ReceivePaymentMethod.BitcoinAddress
            );
            val prepareResponse = sdk.prepareReceivePayment(prepareRequest)

            val receiveFeeSats = prepareResponse.feeSats
            // Log.v("Breez", "Fees: ${receiveFeeSats} sats")
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: prepare-receive-payment-onchain
    }

    fun prepareReceiveSpark(sdk: BreezSdk) {
        // ANCHOR: prepare-receive-payment-spark
        try {
            val prepareRequest = PrepareReceivePaymentRequest(
                ReceivePaymentMethod.SparkAddress
            );
            val prepareResponse = sdk.prepareReceivePayment(prepareRequest)

            val receiveFeeSats = prepareResponse.feeSats
            // Log.v("Breez", "Fees: ${receiveFeeSats} sats")
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: prepare-receive-payment-spark
    }

    fun receivePayment(sdk: BreezSdk, prepareResponse: PrepareReceivePaymentResponse) {
        // ANCHOR: receive-payment
        try {
            val req = ReceivePaymentRequest(prepareResponse)
            val res = sdk.receivePayment(req)
            val paymentRequest = res.paymentRequest
            // Log.v("Breez", "Payment Request: ${paymentRequest}")
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: receive-payment
    }
}
