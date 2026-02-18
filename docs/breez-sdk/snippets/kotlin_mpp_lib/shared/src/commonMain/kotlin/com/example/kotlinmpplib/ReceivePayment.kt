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
            // Optionally set the expiry duration in seconds
            val optionalExpirySecs = 3600.toUInt()

            val request = ReceivePaymentRequest(
                ReceivePaymentMethod.Bolt11Invoice(description, optionalAmountSats, optionalExpirySecs, null)
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
            // Kotlin MPP (BigInteger from com.ionspin.kotlin.bignum.integer, which is included in
            // package)
            val optionalAmountSats = BigInteger.fromLong(5_000L)
            // Android (BigInteger from java.math)
            // val optionalAmountSats = BigInteger.valueOf(5_000L)
            // Optionally set the expiry UNIX timestamp in seconds
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
}
