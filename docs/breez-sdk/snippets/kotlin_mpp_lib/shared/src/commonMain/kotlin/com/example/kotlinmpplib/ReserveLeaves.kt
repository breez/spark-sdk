package com.example.kotlinmpplib

import breez_sdk_spark.*
import com.ionspin.kotlin.bignum.integer.BigInteger

class ReserveLeaves {
    suspend fun prepareSendPaymentReserveLeaves(sdk: BreezSdk) {
        // ANCHOR: prepare-send-payment-reserve-leaves
        val paymentRequest = "<payment request>"
        // Kotlin MPP (BigInteger from com.ionspin.kotlin.bignum.integer)
        val amountSats = BigInteger.fromLong(50_000L)
        // Android (BigInteger from java.math)
        // val amountSats = BigInteger.valueOf(50_000L)

        try {
            val req = PrepareSendPaymentRequest(
                paymentRequest,
                amount = amountSats,
                tokenIdentifier = null,
                conversionOptions = null,
                feePolicy = null,
                reserveLeaves = true,
            )
            val prepareResponse = sdk.prepareSendPayment(req)

            // The reservation ID can be used to cancel the reservation if needed
            prepareResponse.reservationId?.let { reservationId ->
                // Log.v("Breez", "Reservation ID: $reservationId")
            }

            // Send payment as usual using the prepare response
            // sdk.sendPayment(SendPaymentRequest(prepareResponse))
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: prepare-send-payment-reserve-leaves
    }

    suspend fun cancelPrepareSendPayment(sdk: BreezSdk) {
        // ANCHOR: cancel-prepare-send-payment
        val reservationId = "<reservation id from prepare response>"

        try {
            sdk.cancelPrepareSendPayment(
                CancelPrepareSendPaymentRequest(reservationId)
            )
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: cancel-prepare-send-payment
    }
}
