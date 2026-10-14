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
            val optionalReceiverIdentityPublicKey: String? = null
            // Set to FeePolicy.FEES_INCLUDED to deduct a partner fee from the amount
            // instead of adding it on top for the payer
            val optionalFeePolicy: FeePolicy? = null

            val request = ReceivePaymentRequest(
                ReceivePaymentMethod.Bolt11Invoice(
                    description,
                    optionalAmountSats,
                    optionalExpirySecs,
                    null,
                    optionalReceiverIdentityPublicKey,
                    optionalFeePolicy
                )
            )
            val response = sdk.receivePayment(request)

            val paymentRequest = response.paymentRequest
            // Log.v("Breez", "Payment Request: ${paymentRequest}")
            when (val fee = response.fee) {
                is ReceiveFee.Fixed -> {
                    // Log.v("Breez", "Fees: ${fee.amount} sats")
                }
                // Only on an amountless invoice: a share of what the payer sends
                is ReceiveFee.Rate -> {
                    // Log.v("Breez", "Fees: ${fee.bps} bps of the amount sent" +
                    //     " plus ${fee.fixedSats} sats")
                }
            }
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: receive-payment-lightning-bolt11
    }

    suspend fun receiveOnchain(sdk: BreezSdk) {
        // ANCHOR: receive-payment-onchain
        try {
            val newAddress: Boolean? = null // Set to true to get a new address
            val request = ReceivePaymentRequest(
                ReceivePaymentMethod.BitcoinAddress(newAddress = newAddress)
            )
            val response = sdk.receivePayment(request)

            val paymentRequest = response.paymentRequest
            // Log.v("Breez", "Payment Request: ${paymentRequest}")
            val fee = response.fee
            if (fee is ReceiveFee.Fixed) {
                // Log.v("Breez", "Fees: ${fee.amount} sats")
            }
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
            val fee = response.fee
            if (fee is ReceiveFee.Fixed) {
                // Log.v("Breez", "Fees: ${fee.amount} sats")
            }
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
            val fee = response.fee
            if (fee is ReceiveFee.Fixed) {
                // Log.v("Breez", "Fees: ${fee.amount} sats")
            }
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: receive-payment-spark-invoice
    }
}
