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
            val response = sdk.listPayments(ListPaymentsRequest())
            val payments = response.payments
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: list-payments
    }

    suspend fun listPaymentsFiltered(sdk: BreezSdk) {
        // ANCHOR: list-payments-filtered
        try {
            // Filter by asset (Bitcoin or Token)
            val assetFilter = AssetFilter.Token(tokenIdentifier = "token_identifier_here")
            // To filter by Bitcoin instead:
            // val assetFilter = AssetFilter.Bitcoin

            val response = sdk.listPayments(
                ListPaymentsRequest(
                    // Filter by payment type
                    typeFilter = listOf(PaymentType.SEND, PaymentType.RECEIVE),
                    // Filter by status
                    statusFilter = listOf(PaymentStatus.COMPLETED),
                    assetFilter = assetFilter,
                    // Time range filters
                    fromTimestamp = 1704067200u, // Unix timestamp
                    toTimestamp = 1735689600u,   // Unix timestamp
                    // Pagination
                    offset = 0u,
                    limit = 50u,
                    // Sort order (true = oldest first, false = newest first)
                    sortAscending = false
                ))
            val payments = response.payments
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: list-payments-filtered
    }
}
