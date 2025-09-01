package com.example.kotlinmpplib

import breez_sdk_spark.*
class ListPayments {
    suspend fun getPayment(sdk: BreezSdk) {
        // ANCHOR: get-payment
        try {
            val paymentId = "<payment id>";
            val response = sdk.getPayment(GetPaymentRequest(paymentId))
            val payment = response.payment
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: get-payment
    }

    suspend fun listPayments(sdk: BreezSdk) {
        // ANCHOR: list-payments
        try {
            val response = sdk.listPayments(ListPaymentsRequest(null, null))
            val payments = response.payments
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: list-payments
    }

    suspend fun listPaymentsFiltered(sdk: BreezSdk) {
        // ANCHOR: list-payments-filtered
        try {
            val response = sdk.listPayments(
                ListPaymentsRequest(
                    offset = 0u,
                    limit = 50u
                ))
            val payments = response.payments
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: list-payments-filtered
    }
}
