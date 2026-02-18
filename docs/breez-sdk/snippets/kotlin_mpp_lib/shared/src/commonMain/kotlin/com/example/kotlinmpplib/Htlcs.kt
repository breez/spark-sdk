package com.example.kotlinmpplib

import breez_sdk_spark.*
import com.ionspin.kotlin.bignum.integer.BigInteger
import org.kotlincrypto.hash.sha2.SHA256

@OptIn(kotlin.ExperimentalStdlibApi::class)
class Htlcs {
    suspend fun sendHtlcPayment(sdk: BreezSdk) {
        // ANCHOR: send-htlc-payment
        val paymentRequest = "<spark address>"
        // Set the amount you wish the pay the receiver
        // Kotlin MPP (BigInteger from com.ionspin.kotlin.bignum.integer)
        val amountSats = BigInteger.fromLong(50_000L)
        // Android (BigInteger from java.math)
        // val amountSats = BigInteger.valueOf(50_000L)
        try {
            val prepareRequest = PrepareSendPaymentRequest(
                paymentRequest = paymentRequest,
                amount = amountSats,
                tokenIdentifier = null,
                conversionOptions = null,
                feePolicy = null,
            )
            val prepareResponse = sdk.prepareSendPayment(prepareRequest)

            // If the fees are acceptable, continue to create the HTLC Payment
            val paymentMethod = prepareResponse.paymentMethod
            if (paymentMethod is SendPaymentMethod.SparkAddress) {
                val fee = paymentMethod.fee
                // Log.v("Breez", "Fees: ${fee} sats")
            }

            val preimage = "<32-byte unique preimage hex>"
            val preimageBytes = preimage.hexToByteArray()
            val digest = SHA256()
            digest.update(preimageBytes)
            val paymentHashBytes = digest.digest()
            val paymentHash = paymentHashBytes.toHexString()

            // Set the HTLC options
            val htlcOptions = SparkHtlcOptions(
                paymentHash = paymentHash,
                expiryDurationSecs = 1000u
            )
            val options = SendPaymentOptions.SparkAddress(htlcOptions = htlcOptions)

            val request = SendPaymentRequest(
                prepareResponse = prepareResponse,
                options = options
            )
            val sendResponse = sdk.sendPayment(request)
            val payment = sendResponse.payment
        } catch (e: Exception) {
            // handle error
            throw e
        }
        // ANCHOR_END: send-htlc-payment
    }

    suspend fun receiveHodlInvoicePayment(sdk: BreezSdk) {
        // ANCHOR: receive-hodl-invoice-payment
        try {
            val preimage = "<32-byte unique preimage hex>"
            val preimageBytes = preimage.hexToByteArray()
            val digest = SHA256()
            digest.update(preimageBytes)
            val paymentHashBytes = digest.digest()
            val paymentHash = paymentHashBytes.toHexString()

            val response = sdk.receivePayment(
                ReceivePaymentRequest(
                    paymentMethod = ReceivePaymentMethod.Bolt11Invoice(
                        description = "HODL invoice",
                        amountSats = 50_000u,
                        expirySecs = null,
                        paymentHash = paymentHash
                    )
                )
            )

            val invoice = response.paymentRequest
            // Log.v("Breez", "HODL invoice: $invoice")
        } catch (e: Exception) {
            // handle error
            throw e
        }
        // ANCHOR_END: receive-hodl-invoice-payment
    }

    suspend fun listClaimableHtlcPayments(sdk: BreezSdk) {
        // ANCHOR: list-claimable-htlc-payments
        try {
            val request = ListPaymentsRequest(
                typeFilter = listOf(PaymentType.RECEIVE),
                statusFilter = listOf(PaymentStatus.PENDING),
                paymentDetailsFilter = listOf(
                    PaymentDetailsFilter.Spark(
                        htlcStatus = listOf(SparkHtlcStatus.WAITING_FOR_PREIMAGE),
                        conversionRefundNeeded = null
                    ),
                    PaymentDetailsFilter.Lightning(
                        htlcStatus = listOf(SparkHtlcStatus.WAITING_FOR_PREIMAGE)
                    )
                )
            )

            val response = sdk.listPayments(request)
            val payments = response.payments

            for (payment in payments) {
                val details = payment.details
                when (details) {
                    is PaymentDetails.Spark -> {
                        val htlc = details.htlcDetails
                        if (htlc != null) {
                            // Log.v("Breez", "Spark HTLC expiry time: ${htlc.expiryTime}")
                        }
                    }
                    is PaymentDetails.Lightning -> {
                        val htlc = details.htlcDetails
                        // Log.v("Breez", "Lightning HTLC expiry time: ${htlc.expiryTime}")
                    }
                    else -> {}
                }
            }
        } catch (e: Exception) {
            // handle error
            throw e
        }
        // ANCHOR_END: list-claimable-htlc-payments
    }

    suspend fun claimHtlcPayment(sdk: BreezSdk) {
        // ANCHOR: claim-htlc-payment
        try {
            val preimage = "<preimage hex>"
            val request = ClaimHtlcPaymentRequest(preimage = preimage)
            val response = sdk.claimHtlcPayment(request)
            val payment = response.payment
        } catch (e: Exception) {
            // handle error
            throw e
        }
        // ANCHOR_END: claim-htlc-payment
    }
}
