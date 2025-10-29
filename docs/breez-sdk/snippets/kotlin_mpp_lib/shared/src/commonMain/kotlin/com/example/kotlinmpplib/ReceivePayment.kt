package com.example.kotlinmpplib

import breez_sdk_spark.*
import com.ionspin.kotlin.bignum.integer.BigInteger

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
            val receiveFeeSats = response.fee
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
            val receiveFeeSats = response.fee
            // Log.v("Breez", "Fees: ${receiveFeeSats} sats")
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: receive-payment-onchain
    }

    suspend fun receiveSparkAddress(sdk: BreezSdk) {
        // ANCHOR: receive-payment-spark-address
        try {
            val request = ReceivePaymentRequest(ReceivePaymentMethod.SparkAddress)
            val response = sdk.receivePayment(request)

            val paymentRequest = response.paymentRequest
            // Log.v("Breez", "Payment Request: ${paymentRequest}")
            val receiveFeeSats = response.fee
            // Log.v("Breez", "Fees: ${receiveFeeSats} sats")
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: receive-payment-spark-address
    }

    suspend fun receiveSparkInvoice(sdk: BreezSdk) {
        // ANCHOR: receive-payment-spark-invoice
        try {
            val optionalDescription = "<invoice description>"
            val optionalAmountSats = BigInteger.fromLong(5_000L)
            val optionalExpiryTimeSeconds = 1716691200.toULong()
            val optionalSenderPublicKey = "<sender public key>"

            val request = ReceivePaymentRequest(
                ReceivePaymentMethod.SparkInvoice(
                    tokenIdentifier = null,
                    description = optionalDescription,
                    amount = optionalAmountSats,
                    expiryTime = optionalExpiryTimeSeconds,
                    senderPublicKey = optionalSenderPublicKey
                )
            )
            val response = sdk.receivePayment(request)

            val paymentRequest = response.paymentRequest
            // Log.v("Breez", "Payment Request: ${paymentRequest}")
            val receiveFeeSats = response.fee
            // Log.v("Breez", "Fees: ${receiveFeeSats} sats")
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: receive-payment-spark-invoice
    }

    suspend fun waitForPayment(sdk: BreezSdk) {
        // ANCHOR: wait-for-payment
        try {
            // Waiting for a payment given its payment request (Bolt11 or Spark invoice)
            val paymentRequest = "<Bolt11 or Spark invoice>"

            // Wait for a payment to be completed using a payment request
            val paymentRequestResponse = sdk.waitForPayment(
                WaitForPaymentRequest(
                    WaitForPaymentIdentifier.PaymentRequest(paymentRequest)
                )
            )

            // Log.v("Breez", "Payment received with ID: ${paymentRequestResponse.payment.id}")

            // Waiting for a payment given its payment id
            val paymentId = "<payment id>"

            // Wait for a payment to be completed using a payment id
            val paymentIdResponse = sdk.waitForPayment(
                WaitForPaymentRequest(
                    WaitForPaymentIdentifier.PaymentId(paymentId)
                )
            )

            // Log.v("Breez", "Payment received with ID: ${paymentIdResponse.payment.id}")
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: wait-for-payment
    }
}
